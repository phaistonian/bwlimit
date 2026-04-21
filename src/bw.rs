use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

// ── Public data types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppState {
    pub limit_up: u32,           // Mbit/s; 0 = unlimited
    pub limit_down: u32,         // Mbit/s; 0 = unlimited
    pub interface: String,       // "" = all interfaces
    pub launch_at_login: bool,
    pub show_latency: bool,
    pub schedule: Option<Schedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub start_hour: u8, // 0-23
    pub end_hour: u8,   // 0-23 (exclusive)
}

#[derive(Debug, Clone)]
pub struct AppUsage {
    pub name: String,
    pub upload_bps: f64,
    pub download_bps: f64,
}

#[derive(Debug, Clone)]
pub struct NetInterface {
    pub id: String,
    pub name: String,
}

// ── State persistence ──────────────────────────────────────────────────────────

fn state_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Library/Application Support/com.gp.bwlimit");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("state.json"))
}

pub fn load_state() -> AppState {
    state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &AppState) {
    if let Some(path) = state_path() {
        if let Ok(json) = serde_json::to_string_pretty(state) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ── pfctl / dnctl wrappers ─────────────────────────────────────────────────────

/// Build and run the osascript privilege-escalation command to apply limits.
pub fn apply_limit(state: &AppState) -> anyhow::Result<()> {
    if state.limit_up == 0 && state.limit_down == 0 {
        return remove_limit();
    }

    let iface_clause = if state.interface.is_empty() {
        String::new()
    } else {
        format!(" on {}", state.interface)
    };

    let mut cmds: Vec<String> = Vec::new();
    let mut pf_rules: Vec<String> = Vec::new();

    if state.limit_up > 0 {
        info!("Setting upload limit to {} Mbit/s", state.limit_up);
        cmds.push(format!("dnctl pipe 1 config bw {}Mbit/s", state.limit_up));
        pf_rules.push(format!("dummynet out{} from any to any pipe 1", iface_clause));
    }
    if state.limit_down > 0 {
        info!("Setting download limit to {} Mbit/s", state.limit_down);
        cmds.push(format!("dnctl pipe 2 config bw {}Mbit/s", state.limit_down));
        pf_rules.push(format!("dummynet in{} from any to any pipe 2", iface_clause));
    }

    let pf_cmd = if pf_rules.len() == 1 {
        format!("echo '{}' | pfctl -f -", pf_rules[0])
    } else {
        let echos = pf_rules
            .iter()
            .map(|r| format!("echo '{}'", r))
            .collect::<Vec<_>>()
            .join("; ");
        format!("{{ {}; }} | pfctl -f -", echos)
    };
    cmds.push(pf_cmd);
    cmds.push("pfctl -E".to_string());

    let shell_cmd = cmds.join(" && ");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell_cmd
    );

    let status = Command::new("osascript").args(["-e", &script]).status()?;
    if !status.success() {
        error!("osascript failed applying limit");
        return Err(anyhow::anyhow!("osascript failed"));
    }
    Ok(())
}

pub fn remove_limit() -> anyhow::Result<()> {
    info!("Removing bandwidth limits");
    let script = "do shell script \
        \"pfctl -d; \
          dnctl pipe 1 delete 2>/dev/null; \
          dnctl pipe 2 delete 2>/dev/null; \
          pfctl -f /etc/pf.conf\" \
        with administrator privileges";

    let status = Command::new("osascript").args(["-e", script]).status()?;
    if !status.success() {
        error!("osascript failed removing limit");
        return Err(anyhow::anyhow!("osascript failed"));
    }
    Ok(())
}

// ── Interface enumeration ──────────────────────────────────────────────────────

pub fn list_interfaces() -> Vec<NetInterface> {
    let output = Command::new("networksetup")
        .args(["-listallhardwareports"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut result = Vec::new();
    let mut current_name: Option<String> = None;
    for line in output.lines() {
        if let Some(name) = line.strip_prefix("Hardware Port: ") {
            current_name = Some(name.trim().to_string());
        } else if let Some(id) = line.strip_prefix("Device: ") {
            if let Some(name) = current_name.take() {
                result.push(NetInterface {
                    id: id.trim().to_string(),
                    name,
                });
            }
        }
    }
    result
}

// ── Launch at Login ────────────────────────────────────────────────────────────

pub fn set_launch_at_login(enabled: bool) {
    let script = if enabled {
        r#"tell application "System Events" to make login item at end with properties {path:"/Applications/BwLimit.app", hidden:false, name:"BwLimit"}"#
    } else {
        r#"tell application "System Events" to delete login item "BwLimit""#
    };
    let _ = Command::new("osascript").args(["-e", script]).status();
}

pub fn check_launch_at_login() -> bool {
    Command::new("osascript")
        .args(["-e", r#"tell application "System Events" to get name of every login item"#])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("BwLimit"))
        .unwrap_or(false)
}

// ── Schedule ───────────────────────────────────────────────────────────────────

pub fn schedule_is_active(s: &Schedule) -> bool {
    let hour_str = Command::new("date")
        .arg("+%H")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let hour: u8 = match hour_str.trim().parse() {
        Ok(h) => h,
        Err(_) => return false,
    };
    if s.start_hour <= s.end_hour {
        hour >= s.start_hour && hour < s.end_hour
    } else {
        hour >= s.start_hour || hour < s.end_hour
    }
}

/// Open osascript dialogs to let the user set a schedule. Returns None on cancel.
pub fn prompt_schedule() -> Option<Schedule> {
    let output = Command::new("osascript")
        .args([
            "-e", r#"set s to text returned of (display dialog "Limit active from hour (0-23):" default answer "9" buttons {"Cancel","OK"} default button "OK")"#,
            "-e", r#"set e to text returned of (display dialog "Until hour (0-23):" default answer "18" buttons {"Cancel","OK"} default button "OK")"#,
            "-e", "return s & \",\" & e",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let result = String::from_utf8(output.stdout).ok()?;
    let mut parts = result.trim().split(',');
    let start: u8 = parts.next()?.trim().parse().ok()?;
    let end: u8 = parts.next()?.trim().parse().ok()?;
    if start > 23 || end > 23 {
        return None;
    }
    Some(Schedule { start_hour: start, end_hour: end })
}

// ── Ping latency ───────────────────────────────────────────────────────────────

/// Ping 1.1.1.1 once and return the round-trip time in milliseconds.
pub fn ping_latency_ms() -> Option<f64> {
    let output = Command::new("ping")
        .args(["-c", "1", "-W", "1000", "1.1.1.1"])
        .output()
        .ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    for line in stdout.lines() {
        // macOS: "round-trip min/avg/max/stddev = 12.3/12.3/12.3/0.0 ms"
        if line.contains("round-trip") || line.contains("rtt") {
            let after_eq = line.split('=').nth(1)?;
            let avg = after_eq.trim().split('/').nth(1)?;
            return avg.trim().parse().ok();
        }
    }
    None
}

// ── Network usage (nettop) ─────────────────────────────────────────────────────

pub fn get_top_uploaders() -> (Vec<AppUsage>, f64, f64) {
    let output = Command::new("nettop")
        .args(["-L", "2", "-P", "-t", "external"])
        .output();

    let output = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return (vec![], 0.0, 0.0),
    };

    struct Sample {
        time: f64,
        bytes_out: HashMap<String, u64>,
        bytes_in: HashMap<String, u64>,
    }

    let mut samples: Vec<Sample> = Vec::new();
    let mut cur_out: HashMap<String, u64> = HashMap::new();
    let mut cur_in: HashMap<String, u64> = HashMap::new();
    let mut cur_time = 0.0f64;

    for line in output.lines() {
        if line.starts_with("time,") {
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() > 5 {
            if let Ok(t) = parse_time(parts[0]) {
                if (t - cur_time).abs() > 0.001 {
                    if !cur_out.is_empty() || !cur_in.is_empty() {
                        samples.push(Sample { time: cur_time, bytes_out: cur_out, bytes_in: cur_in });
                        cur_out = HashMap::new();
                        cur_in = HashMap::new();
                    }
                    cur_time = t;
                }
            }
            let name = {
                let p = parts[1];
                match p.rfind('.') {
                    Some(i) => p[..i].to_string(),
                    None => p.to_string(),
                }
            };
            if let Ok(v) = parts[4].parse::<u64>() {
                *cur_in.entry(name.clone()).or_insert(0) += v;
            }
            if let Ok(v) = parts[5].parse::<u64>() {
                *cur_out.entry(name).or_insert(0) += v;
            }
        }
    }
    if !cur_out.is_empty() || !cur_in.is_empty() {
        samples.push(Sample { time: cur_time, bytes_out: cur_out, bytes_in: cur_in });
    }

    if samples.len() < 2 {
        return (vec![], 0.0, 0.0);
    }
    let s1 = &samples[samples.len() - 2];
    let s2 = &samples[samples.len() - 1];
    let dt = s2.time - s1.time;
    if dt <= 0.0 {
        return (vec![], 0.0, 0.0);
    }

    let mut app_map: HashMap<String, AppUsage> = HashMap::new();
    let mut total_up = 0.0f64;
    let mut total_down = 0.0f64;

    for (name, b2) in &s2.bytes_out {
        let b1 = s1.bytes_out.get(name).cloned().unwrap_or(0);
        if *b2 > b1 {
            let bps = ((*b2 - b1) as f64 * 8.0) / dt;
            total_up += bps;
            if bps > 100.0 {
                app_map.entry(name.clone())
                    .or_insert_with(|| AppUsage { name: name.clone(), upload_bps: 0.0, download_bps: 0.0 })
                    .upload_bps = bps;
            }
        }
    }
    for (name, b2) in &s2.bytes_in {
        let b1 = s1.bytes_in.get(name).cloned().unwrap_or(0);
        if *b2 > b1 {
            let bps = ((*b2 - b1) as f64 * 8.0) / dt;
            total_down += bps;
            if bps > 100.0 {
                app_map.entry(name.clone())
                    .or_insert_with(|| AppUsage { name: name.clone(), upload_bps: 0.0, download_bps: 0.0 })
                    .download_bps = bps;
            }
        }
    }

    let mut rates: Vec<AppUsage> = app_map.into_values().collect();
    rates.sort_by(|a, b| {
        (b.upload_bps + b.download_bps).partial_cmp(&(a.upload_bps + a.download_bps)).unwrap()
    });
    rates.truncate(5);
    (rates, total_up, total_down)
}

fn parse_time(s: &str) -> Result<f64, std::num::ParseFloatError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse()?;
        let m: f64 = parts[1].parse()?;
        let sec: f64 = parts[2].parse()?;
        Ok(h * 3600.0 + m * 60.0 + sec)
    } else {
        s.parse()
    }
}
