use std::process::Command;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, error};

#[derive(Debug, Clone)]
pub struct AppUsage {
    pub name: String,
    pub upload_bps: f64,
    pub download_bps: f64,
}

pub fn get_top_uploaders() -> (Vec<AppUsage>, f64, f64) {
    // Run nettop with 2 samples. Use -t external to exclude local/loopback traffic
    // so we don't confuse users with fast local IPC traffic (like language_server).
    // CSV columns: time, proc.pid, interface, state, bytes_in, bytes_out, …
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
    let mut current_out: HashMap<String, u64> = HashMap::new();
    let mut current_in: HashMap<String, u64> = HashMap::new();
    let mut current_time = 0.0;

    for line in output.lines() {
        if line.starts_with("time,") {
            continue;
        }

        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() > 5 {
            if let Ok(t) = parse_time(parts[0]) {
                if (t - current_time).abs() > 0.001 {
                    if !current_out.is_empty() || !current_in.is_empty() {
                        samples.push(Sample {
                            time: current_time,
                            bytes_out: current_out,
                            bytes_in: current_in,
                        });
                        current_out = HashMap::new();
                        current_in = HashMap::new();
                    }
                    current_time = t;
                }
            }

            let proc_with_pid = parts[1];
            let name = match proc_with_pid.rfind('.') {
                Some(idx) => &proc_with_pid[..idx],
                None => proc_with_pid,
            }
            .to_string();

            // bytes_in is column 4 (download), bytes_out is column 5 (upload)
            if let Ok(v) = parts[4].parse::<u64>() {
                *current_in.entry(name.clone()).or_insert(0) += v;
            }
            if let Ok(v) = parts[5].parse::<u64>() {
                *current_out.entry(name).or_insert(0) += v;
            }
        }
    }
    if !current_out.is_empty() || !current_in.is_empty() {
        samples.push(Sample {
            time: current_time,
            bytes_out: current_out,
            bytes_in: current_in,
        });
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
    let mut total_upload_bps = 0.0f64;
    let mut total_download_bps = 0.0f64;

    // Upload rates
    for (name, b2) in &s2.bytes_out {
        let b1 = s1.bytes_out.get(name).cloned().unwrap_or(0);
        if *b2 > b1 {
            let bps = ((*b2 - b1) as f64 * 8.0) / dt;
            total_upload_bps += bps;
            if bps > 100.0 {
                app_map
                    .entry(name.clone())
                    .or_insert_with(|| AppUsage { name: name.clone(), upload_bps: 0.0, download_bps: 0.0 })
                    .upload_bps = bps;
            }
        }
    }

    // Download rates
    for (name, b2) in &s2.bytes_in {
        let b1 = s1.bytes_in.get(name).cloned().unwrap_or(0);
        if *b2 > b1 {
            let bps = ((*b2 - b1) as f64 * 8.0) / dt;
            total_download_bps += bps;
            if bps > 100.0 {
                app_map
                    .entry(name.clone())
                    .or_insert_with(|| AppUsage { name: name.clone(), upload_bps: 0.0, download_bps: 0.0 })
                    .download_bps = bps;
            }
        }
    }

    // Sort by combined traffic, keep top 5
    let mut rates: Vec<AppUsage> = app_map.into_values().collect();
    rates.sort_by(|a, b| {
        let a_total = a.upload_bps + a.download_bps;
        let b_total = b.upload_bps + b.download_bps;
        b_total.partial_cmp(&a_total).unwrap()
    });
    rates.truncate(5);

    (rates, total_upload_bps, total_download_bps)
}

fn parse_time(s: &str) -> Result<f64, std::num::ParseFloatError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h = parts[0].parse::<f64>()?;
        let m = parts[1].parse::<f64>()?;
        let s = parts[2].parse::<f64>()?;
        Ok(h * 3600.0 + m * 60.0 + s)
    } else {
        s.parse::<f64>()
    }
}

// ── State persistence ──────────────────────────────────────────────────────────

fn state_file_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Library/Application Support/com.gp.bwlimit");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("state"))
}

/// Persist the active limit so it is restored on next launch.
/// `mbits = 0` means Unlimited.
pub fn save_limit(mbits: u32) {
    if let Some(path) = state_file_path() {
        let _ = std::fs::write(path, mbits.to_string());
    }
}

/// Load the previously saved limit. Returns `None` when no state exists yet.
pub fn load_limit() -> Option<u32> {
    let path = state_file_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

// ── pfctl / dnctl wrappers ─────────────────────────────────────────────────────

pub fn set_limit(mbits: u32) -> anyhow::Result<()> {
    info!("Setting upload limit to {} Mbit/s", mbits);

    // Use osascript to elevate privileges — sudo requires a TTY and fails
    // silently inside a GUI .app bundle. The `with administrator privileges`
    // clause shows the native macOS auth dialog and caches credentials.
    let shell_cmd = format!(
        "dnctl pipe 1 config bw {mbits}Mbit/s && \
         echo 'dummynet out from any to any pipe 1' | pfctl -f - && \
         pfctl -E",
        mbits = mbits
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell_cmd
    );

    let status = Command::new("osascript").args(["-e", &script]).status()?;

    if !status.success() {
        error!("Failed to set bandwidth limit via osascript");
        return Err(anyhow::anyhow!("osascript privilege escalation failed"));
    }

    save_limit(mbits);
    Ok(())
}

pub fn reset_limit() -> anyhow::Result<()> {
    info!("Removing bandwidth limit");

    // pfctl -d may fail if already disabled; ignore that error.
    let script =
        "do shell script \"pfctl -d; pfctl -f /etc/pf.conf\" with administrator privileges";

    let status = Command::new("osascript").args(["-e", script]).status()?;

    if !status.success() {
        error!("Failed to reset bandwidth limit via osascript");
        return Err(anyhow::anyhow!("osascript privilege escalation failed"));
    }

    save_limit(0);
    Ok(())
}
