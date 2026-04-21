use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};
use tracing::{error, info};

use crate::bw;

// ── Events ─────────────────────────────────────────────────────────────────────

struct PollResult {
    usage: Vec<bw::AppUsage>,
    total_up: f64,
    total_down: f64,
    latency_ms: Option<f64>,
}

enum UserEvent {
    Menu(MenuEvent),
    Exit,
    Poll(PollResult),
    WakeFromSleep,
    ScheduleChanged(bool), // true = became active, false = became inactive
}

// ── Bandwidth presets ──────────────────────────────────────────────────────────

const PRESETS: &[(&str, u32)] = &[
    ("Unlimited", 0),
    ("1 Mbit/s", 1),
    ("2 Mbit/s", 2),
    ("3 Mbit/s", 3),
    ("4 Mbit/s", 4),
    ("5 Mbit/s", 5),
    ("10 Mbit/s", 10),
    ("25 Mbit/s", 25),
    ("50 Mbit/s", 50),
    ("100 Mbit/s", 100),
];

// ── Entry point ────────────────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    let mut event_loop = builder.build();
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    let proxy_menu = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy_menu.send_event(UserEvent::Menu(e));
    }));

    // ── Load persisted state ──────────────────────────────────────────────────
    let state = Arc::new(Mutex::new(bw::load_state()));
    let init_state = state.lock().unwrap().clone();

    // ── Build menu ────────────────────────────────────────────────────────────
    let menu = Menu::new();

    // Upload submenu
    let upload_sub = Submenu::new("Upload Limit", true);
    let upload_items: Vec<(u32, CheckMenuItem)> = PRESETS
        .iter()
        .map(|(label, mbits)| {
            let checked = *mbits == init_state.limit_up;
            (*mbits, CheckMenuItem::new(*label, true, checked, None))
        })
        .collect();
    for (_, item) in &upload_items {
        upload_sub.append(item)?;
    }

    // Download submenu
    let download_sub = Submenu::new("Download Limit", true);
    let download_items: Vec<(u32, CheckMenuItem)> = PRESETS
        .iter()
        .map(|(label, mbits)| {
            let checked = *mbits == init_state.limit_down;
            (*mbits, CheckMenuItem::new(*label, true, checked, None))
        })
        .collect();
    for (_, item) in &download_items {
        download_sub.append(item)?;
    }

    // Interface submenu
    let interface_sub = Submenu::new("Interface", true);
    let all_iface_item = CheckMenuItem::new("All Interfaces", true, init_state.interface.is_empty(), None);
    interface_sub.append(&all_iface_item)?;
    interface_sub.append(&PredefinedMenuItem::separator())?;
    let iface_list = bw::list_interfaces();
    let iface_items: Vec<(String, CheckMenuItem)> = iface_list
        .iter()
        .map(|iface| {
            let checked = iface.id == init_state.interface;
            let label = format!("{} ({})", iface.name, iface.id);
            (iface.id.clone(), CheckMenuItem::new(label, true, checked, None))
        })
        .collect();
    for (_, item) in &iface_items {
        interface_sub.append(item)?;
    }

    // Schedule submenu
    let schedule_sub = Submenu::new("Schedule", true);
    let sched_off_item = CheckMenuItem::new("No Schedule", true, init_state.schedule.is_none(), None);
    let sched_set_item = MenuItem::new("Set Hours…", true, None);
    let sched_label = {
        let txt = schedule_label_text(&init_state.schedule);
        MenuItem::new(txt, false, None)
    };
    schedule_sub.append_items(&[
        &sched_off_item,
        &sched_set_item,
        &PredefinedMenuItem::separator(),
        &sched_label,
    ])?;

    // Launch at Login
    let login_item = CheckMenuItem::new(
        "Launch at Login",
        true,
        bw::check_launch_at_login(),
        None,
    );

    // Show Latency toggle
    let show_latency_item = CheckMenuItem::new(
        "Show Latency",
        true,
        init_state.show_latency,
        None,
    );

    // Network Activity section
    let net_header = MenuItem::new("Network Activity:", false, None);
    let latency_item = MenuItem::new("Latency: —", false, None);
    let usage_items: Vec<MenuItem> = (0..5).map(|_| MenuItem::new("", false, None)).collect();

    // Quit
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append_items(&[
        &upload_sub,
        &download_sub,
        &interface_sub,
        &PredefinedMenuItem::separator(),
        &schedule_sub,
        &PredefinedMenuItem::separator(),
        &login_item,
        &show_latency_item,
        &PredefinedMenuItem::separator(),
        &net_header,
        &latency_item,
    ])?;
    for item in &usage_items {
        menu.append(item)?;
    }
    menu.append_items(&[&PredefinedMenuItem::separator(), &quit_item])?;

    // ── Tray icon ─────────────────────────────────────────────────────────────
    let icon = build_icon()?;
    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(tooltip_text(&init_state))
        .with_menu(Box::new(menu))
        .build()?;

    // ── Build MenuId lookup maps ───────────────────────────────────────────────
    let mut upload_id_map: HashMap<tray_icon::menu::MenuId, u32> = HashMap::new();
    for (mbits, item) in &upload_items {
        upload_id_map.insert(item.id().clone(), *mbits);
    }
    let mut download_id_map: HashMap<tray_icon::menu::MenuId, u32> = HashMap::new();
    for (mbits, item) in &download_items {
        download_id_map.insert(item.id().clone(), *mbits);
    }
    let mut iface_id_map: HashMap<tray_icon::menu::MenuId, String> = HashMap::new();
    iface_id_map.insert(all_iface_item.id().clone(), String::new());
    for (id_str, item) in &iface_items {
        iface_id_map.insert(item.id().clone(), id_str.clone());
    }

    // ── Restore saved state on startup ────────────────────────────────────────
    if init_state.limit_up > 0 || init_state.limit_down > 0 {
        let should_apply = match &init_state.schedule {
            None => true,
            Some(s) => bw::schedule_is_active(s),
        };
        if should_apply {
            if let Err(e) = bw::apply_limit(&init_state) {
                error!("Failed to restore limits on startup: {}", e);
            }
        }
    }

    // ── Signal handler ────────────────────────────────────────────────────────
    let proxy_sig = proxy.clone();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = proxy_sig.send_event(UserEvent::Exit);
            }
        });

    // ── Background poll thread ─────────────────────────────────────────────────
    let proxy_poll = proxy.clone();
    let state_poll = Arc::clone(&state);
    std::thread::spawn(move || {
        let interval = Duration::from_secs(3);
        let mut last_schedule_active: Option<bool> = None;
        loop {
            let t0 = Instant::now();

            let (usage, total_up, total_down) = bw::get_top_uploaders();

            // Only ping when the user has enabled latency display
            let do_ping = state_poll.lock().unwrap().show_latency;
            let latency_ms = if do_ping { bw::ping_latency_ms() } else { None };

            let _ = proxy_poll.send_event(UserEvent::Poll(PollResult {
                usage,
                total_up,
                total_down,
                latency_ms,
            }));

            // Schedule boundary detection
            let sched_active = {
                let s = state_poll.lock().unwrap();
                s.schedule.as_ref().map(|sch| bw::schedule_is_active(sch))
            };
            if let Some(active) = sched_active {
                if last_schedule_active != Some(active) {
                    last_schedule_active = Some(active);
                    let _ = proxy_poll.send_event(UserEvent::ScheduleChanged(active));
                }
            } else {
                last_schedule_active = None;
            }

            // Wake-from-sleep detection (normal poll takes ~2.3 s; >8 s means we slept)
            let elapsed = t0.elapsed();
            if elapsed > Duration::from_secs(8) {
                let _ = proxy_poll.send_event(UserEvent::WakeFromSleep);
            }
            if elapsed < interval {
                std::thread::sleep(interval - elapsed);
            }
        }
    });

    // ── Sparkline ring buffer (removed) ───────────────────────────────────────

    // ── Event loop ────────────────────────────────────────────────────────────
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            // ── Exit ──────────────────────────────────────────────────────────
            Event::UserEvent(UserEvent::Exit) => {
                info!("Exiting – removing limits");
                let _ = bw::remove_limit();
                *control_flow = ControlFlow::Exit;
            }

            // ── Wake from sleep ───────────────────────────────────────────────
            Event::UserEvent(UserEvent::WakeFromSleep) => {
                info!("Wake from sleep detected – re-applying limits");
                let s = state.lock().unwrap().clone();
                if s.limit_up > 0 || s.limit_down > 0 {
                    if let Err(e) = bw::apply_limit(&s) {
                        error!("Failed to re-apply limits after wake: {}", e);
                    }
                }
            }

            // ── Schedule boundary ─────────────────────────────────────────────
            Event::UserEvent(UserEvent::ScheduleChanged(now_active)) => {
                let s = state.lock().unwrap().clone();
                if now_active {
                    info!("Schedule activated – applying limits");
                    if s.limit_up > 0 || s.limit_down > 0 {
                        let _ = bw::apply_limit(&s);
                    }
                } else {
                    info!("Schedule deactivated – removing limits");
                    let _ = bw::remove_limit();
                }
            }

            // ── Poll update ───────────────────────────────────────────────────
            Event::UserEvent(UserEvent::Poll(result)) => {
                // Tray title: just ↑/↓ rates
                if result.total_up > 10.0 || result.total_down > 10.0 {
                    let title = format!(
                        "↑{} ↓{}",
                        fmt_short(result.total_up),
                        fmt_short(result.total_down),
                    );
                    let _ = tray.set_title(Some(title));
                } else {
                    let _ = tray.set_title(None::<String>);
                }

                // Latency item (only shown/updated when enabled)
                let show_lat = state.lock().unwrap().show_latency;
                if show_lat {
                    let lat_text = match result.latency_ms {
                        Some(ms) => format!("Latency: {:.0}ms", ms),
                        None => "Latency: —".to_string(),
                    };
                    latency_item.set_text(lat_text);
                } else {
                    latency_item.set_text("");
                }

                // Per-app rows
                for i in 0..5 {
                    if i < result.usage.len() {
                        let app = &result.usage[i];
                        let label = format!(
                            "{}: ↑{} ↓{}",
                            app.name,
                            fmt_long(app.upload_bps),
                            fmt_long(app.download_bps)
                        );
                        usage_items[i].set_text(label);
                    } else if i == 0 && result.usage.is_empty() {
                        usage_items[i].set_text("(no activity)");
                    } else {
                        usage_items[i].set_text("");
                    }
                }
            }

            // ── Menu click ────────────────────────────────────────────────────
            Event::UserEvent(UserEvent::Menu(ev)) => {
                let id = &ev.id;

                // Quit
                if id == quit_item.id() {
                    let _ = proxy.send_event(UserEvent::Exit);
                    return;
                }

                // Upload preset
                if let Some(&mbits) = upload_id_map.get(id) {
                    let mut s = state.lock().unwrap();
                    s.limit_up = mbits;
                    let s_clone = s.clone();
                    drop(s);
                    apply_and_save(&s_clone);
                    for (m, item) in &upload_items {
                        item.set_checked(*m == mbits);
                    }
                    let _ = tray.set_tooltip(Some(tooltip_text(&s_clone)));
                    return;
                }

                // Download preset
                if let Some(&mbits) = download_id_map.get(id) {
                    let mut s = state.lock().unwrap();
                    s.limit_down = mbits;
                    let s_clone = s.clone();
                    drop(s);
                    apply_and_save(&s_clone);
                    for (m, item) in &download_items {
                        item.set_checked(*m == mbits);
                    }
                    let _ = tray.set_tooltip(Some(tooltip_text(&s_clone)));
                    return;
                }

                // Interface
                if let Some(iface_id) = iface_id_map.get(id).cloned() {
                    let mut s = state.lock().unwrap();
                    s.interface = iface_id.clone();
                    let s_clone = s.clone();
                    drop(s);
                    apply_and_save(&s_clone);
                    all_iface_item.set_checked(iface_id.is_empty());
                    for (id_str, item) in &iface_items {
                        item.set_checked(id_str == &iface_id);
                    }
                    return;
                }

                // Schedule: turn off
                if id == sched_off_item.id() {
                    let mut s = state.lock().unwrap();
                    s.schedule = None;
                    let s_clone = s.clone();
                    drop(s);
                    bw::save_state(&s_clone);
                    sched_off_item.set_checked(true);
                    sched_label.set_text(schedule_label_text(&None));
                    return;
                }

                // Schedule: set hours
                if id == sched_set_item.id() {
                    if let Some(sched) = bw::prompt_schedule() {
                        let txt = schedule_label_text(&Some(sched.clone()));
                        let mut s = state.lock().unwrap();
                        s.schedule = Some(sched);
                        bw::save_state(&*s);
                        drop(s);
                        sched_off_item.set_checked(false);
                        sched_label.set_text(txt);
                    }
                    return;
                }

                // Show Latency toggle
                if id == show_latency_item.id() {
                    let new_val = !show_latency_item.is_checked();
                    show_latency_item.set_checked(new_val);
                    if new_val {
                        // Give immediate feedback; actual value arrives on next poll
                        latency_item.set_text("Latency: measuring…");
                    } else {
                        latency_item.set_text("");
                    }
                    let mut s = state.lock().unwrap();
                    s.show_latency = new_val;
                    bw::save_state(&*s);
                    return;
                }

                // Launch at Login
                if id == login_item.id() {
                    let new_val = !login_item.is_checked();
                    bw::set_launch_at_login(new_val);
                    login_item.set_checked(new_val);
                    let mut s = state.lock().unwrap();
                    s.launch_at_login = new_val;
                    bw::save_state(&*s);
                    return;
                }
            }

            _ => {}
        }
    });
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn apply_and_save(state: &bw::AppState) {
    bw::save_state(state);
    if state.limit_up == 0 && state.limit_down == 0 {
        if let Err(e) = bw::remove_limit() {
            error!("Failed to remove limit: {}", e);
        }
    } else {
        let should_apply = match &state.schedule {
            None => true,
            Some(s) => bw::schedule_is_active(s),
        };
        if should_apply {
            if let Err(e) = bw::apply_limit(state) {
                error!("Failed to apply limit: {}", e);
            }
        }
    }
}

fn tooltip_text(state: &bw::AppState) -> String {
    let up = if state.limit_up == 0 {
        "∞".to_string()
    } else {
        format!("{} Mbps", state.limit_up)
    };
    let down = if state.limit_down == 0 {
        "∞".to_string()
    } else {
        format!("{} Mbps", state.limit_down)
    };
    format!("BwLimit  ↑{} ↓{}", up, down)
}

fn schedule_label_text(sched: &Option<bw::Schedule>) -> String {
    match sched {
        None => "  No schedule set".to_string(),
        Some(s) => format!("  Active {:02}:00 – {:02}:00", s.start_hour, s.end_hour),
    }
}



fn fmt_short(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.1}M", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1}k", bps / 1_000.0)
    } else {
        format!("{}b", bps as u64)
    }
}

fn fmt_long(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.1} Mbps", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1} Kbps", bps / 1_000.0)
    } else {
        format!("{} bps", bps as u64)
    }
}

fn build_icon() -> Result<Icon> {
    const SIZE: usize = 18;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = (x as i32) - (SIZE as i32 / 2);
            let is_head = y >= 3 && y <= 8 && dx.abs() <= (y as i32 - 3);
            let is_shaft = y >= 8 && y <= 15 && dx.abs() <= 1;
            if is_head || is_shaft {
                let o = (y * SIZE + x) * 4;
                rgba[o + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).context("failed to build tray icon")
}
