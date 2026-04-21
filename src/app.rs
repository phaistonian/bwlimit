use anyhow::{Context, Result};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use tracing::{info, error};

use crate::bw;

enum UserEvent {
    Menu(MenuEvent),
    Exit,
    UpdateUsage((Vec<bw::AppUsage>, f64, f64)),
}

/// (display label, Mbit/s value) — 0 means Unlimited
const PRESETS: &[(&str, u32)] = &[
    ("1 Mbit/s",   1),
    ("2 Mbit/s",   2),
    ("3 Mbit/s",   3),
    ("4 Mbit/s",   4),
    ("5 Mbit/s",   5),
    ("10 Mbit/s",  10),
    ("25 Mbit/s",  25),
    ("50 Mbit/s",  50),
    ("100 Mbit/s", 100),
    ("Unlimited",  0),
];

pub fn run() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    let mut event_loop = builder.build();
    #[cfg(target_os = "macos")]
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    // Route menu events into the event loop
    let proxy_menu = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy_menu.send_event(UserEvent::Menu(event));
    }));

    // ── Load persisted limit ──────────────────────────────────────────────────
    let saved_mbits = bw::load_limit().unwrap_or(0); // 0 = Unlimited

    // ── Build menu ────────────────────────────────────────────────────────────
    let menu = Menu::new();

    let preset_items: Vec<(u32, CheckMenuItem)> = PRESETS
        .iter()
        .map(|(label, mbits)| {
            let checked = *mbits == saved_mbits;
            (*mbits, CheckMenuItem::new(*label, true, checked, None))
        })
        .collect();

    for (_, item) in &preset_items {
        menu.append(item)?;
    }

    let usage_header = MenuItem::new("Network Activity:", false, None);
    let usage_placeholders: Vec<MenuItem> = (0..5)
        .map(|_| MenuItem::new("", false, None))
        .collect();

    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &usage_header,
    ])?;
    for item in &usage_placeholders {
        menu.append(item)?;
    }

    let quit_item = MenuItem::new("Quit", true, None);
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])?;

    // ── Tray icon ─────────────────────────────────────────────────────────────
    let icon = build_icon()?;
    let tray_icon = TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(limit_tooltip(saved_mbits))
        .with_menu(Box::new(menu))
        .build()?;

    // ── Restore saved limit on startup ────────────────────────────────────────
    if saved_mbits > 0 {
        if let Err(e) = bw::set_limit(saved_mbits) {
            error!("Failed to restore saved limit on startup: {}", e);
        }
    }

    // ── Signal handler (Ctrl-C) ───────────────────────────────────────────────
    let proxy_signal = proxy.clone();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = proxy_signal.send_event(UserEvent::Exit);
            }
        });

    // ── Background polling thread ─────────────────────────────────────────────
    // Accounts for nettop's own ~2 s runtime so the refresh period doesn't drift.
    let proxy_usage = proxy.clone();
    std::thread::spawn(move || {
        let interval = std::time::Duration::from_secs(3);
        loop {
            let t0 = std::time::Instant::now();
            let usage = bw::get_top_uploaders();
            let _ = proxy_usage.send_event(UserEvent::UpdateUsage(usage));
            let elapsed = t0.elapsed();
            if elapsed < interval {
                std::thread::sleep(interval - elapsed);
            }
        }
    });

    // ── Event loop ────────────────────────────────────────────────────────────
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::Exit) => {
                info!("Exit signal received");
                if let Err(e) = bw::reset_limit() {
                    error!("Failed to reset limit on exit: {}", e);
                }
                *control_flow = ControlFlow::Exit;
            }

            Event::UserEvent(UserEvent::UpdateUsage((usage, total_up, total_down))) => {
                // Tray title: show ↑/↓ when either direction is active
                if total_up > 10.0 || total_down > 10.0 {
                    let label = format!(
                        "↑{} ↓{}",
                        format_bps_short(total_up),
                        format_bps_short(total_down)
                    );
                    let _ = tray_icon.set_title(Some(label));
                } else {
                    let _ = tray_icon.set_title(None::<String>);
                }

                // Per-app rows with ↑ and ↓
                for i in 0..5 {
                    if i < usage.len() {
                        let app = &usage[i];
                        let label = format!(
                            "{}: ↑{} ↓{}",
                            app.name,
                            format_bps(app.upload_bps),
                            format_bps(app.download_bps),
                        );
                        usage_placeholders[i].set_text(label);
                    } else if i == 0 && usage.is_empty() {
                        usage_placeholders[i].set_text("(no activity)");
                    } else {
                        usage_placeholders[i].set_text("");
                    }
                }
            }

            Event::UserEvent(UserEvent::Menu(menu_event)) => {
                if menu_event.id == quit_item.id() {
                    info!("Quit requested");
                    let _ = proxy.send_event(UserEvent::Exit);
                    return;
                }

                for (mbits, item) in &preset_items {
                    if menu_event.id == *item.id() {
                        let selected = *mbits;
                        if selected == 0 {
                            if let Err(e) = bw::reset_limit() {
                                error!("Failed to reset limit: {}", e);
                            }
                        } else if let Err(e) = bw::set_limit(selected) {
                            error!("Failed to set limit: {}", e);
                        }

                        // Update checkmarks
                        for (other_mbits, other_item) in &preset_items {
                            other_item.set_checked(*other_mbits == selected);
                        }

                        // Update tooltip
                        let _ = tray_icon.set_tooltip(Some(limit_tooltip(selected)));
                        break;
                    }
                }
            }

            _ => (),
        }
    });
}

/// Tooltip string reflecting the active limit.
fn limit_tooltip(mbits: u32) -> String {
    if mbits == 0 {
        "Bandwidth Limiter – Unlimited".to_string()
    } else {
        format!("Bandwidth Limiter – {} Mbit/s", mbits)
    }
}

/// Short label for the tray title (e.g. "1.2M", "340k", "800b").
fn format_bps_short(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.1}M", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1}k", bps / 1_000.0)
    } else {
        format!("{}b", bps as u64)
    }
}

/// Long label for menu rows (e.g. "1.2 Mbps", "340.0 Kbps", "800 bps").
fn format_bps(bps: f64) -> String {
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
                let offset = (y * SIZE + x) * 4;
                rgba[offset] = 0;
                rgba[offset + 1] = 0;
                rgba[offset + 2] = 0;
                rgba[offset + 3] = 255;
            }
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).context("failed to build tray icon")
}
