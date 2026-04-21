# BwLimit

A lightweight macOS menu-bar app that throttles your upload bandwidth on demand — useful for preventing a single machine from saturating a shared connection while working, gaming, or running large backups.

<p align="center">
  <img src="https://img.shields.io/badge/platform-macOS%2013%2B-blue?style=flat-square" />
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" />
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" />
</p>

---

## Features

- **One-click presets** — 1, 2, 3, 4, 5, 10, 25, 50, 100 Mbit/s or Unlimited
- **Persistent state** — active limit is saved and automatically restored on next launch
- **Live traffic monitor** — tray title shows real-time `↑ upload / ↓ download` speed
- **Network Activity menu** — top 5 apps by combined traffic with per-direction rates
- **Native auth dialog** — uses macOS privilege escalation (no passwordless sudo required)
- **Clean exit** — limit is removed automatically when the app quits
- **Accessory-mode app** — lives only in the menu bar, no Dock icon

## Screenshot

```
┌─────────────────────────┐
│ ↑1.2M ↓3.4M     [icon] │  ← tray title (live)
└─────────────────────────┘

  1 Mbit/s
  2 Mbit/s
  ✓ 5 Mbit/s            ← active limit
  10 Mbit/s
  25 Mbit/s
  50 Mbit/s
  100 Mbit/s
  Unlimited
  ─────────────────
  Network Activity:
  Dropbox: ↑1.2 Mbps ↓0 bps
  Chrome:  ↑0 bps ↓3.4 Mbps
  ─────────────────
  Quit
```

## Requirements

- macOS 13 Ventura or later
- Rust toolchain (`rustup install stable`)

## Installation

### Build from source

```bash
git clone https://github.com/phaistonian/bwlimit.git
cd bwlimit
bash scripts/install-local.sh
```

This compiles a release build, packages it into `BwLimit.app`, installs it to `/Applications`, and launches it.

### Update

Re-run the same script — it kills the running instance, syncs the new bundle, and relaunches.

```bash
bash scripts/install-local.sh
```

## How it works

BwLimit uses two built-in macOS kernel facilities:

| Tool | Role |
|---|---|
| `dnctl` | Creates a dummynet pipe with a configured bandwidth ceiling |
| `pfctl` | Installs a packet filter rule that routes all outbound traffic through the pipe |

When you select **Unlimited**, the pf rule is removed and the default `/etc/pf.conf` is reloaded. No third-party kernel extensions or drivers are involved.

Traffic monitoring uses `nettop -L 2 -P -t external`, which samples external (non-loopback) network I/O per process.

## Privilege escalation

`dnctl` and `pfctl` require root. BwLimit uses `osascript do shell script ... with administrator privileges`, which shows the native macOS password dialog the first time per session. Credentials are cached by the OS — subsequent preset changes apply instantly.

## Development

```bash
# Fast debug build (no .app packaging)
cargo build

# Run directly (prints tracing logs to stdout)
cargo run

# Full release build + packaging + install
bash scripts/install-local.sh
```

### Project structure

```
src/
  main.rs   – entry point
  app.rs    – event loop, tray icon, menu construction
  bw.rs     – nettop parsing, dnctl/pfctl wrappers, state persistence
macos/
  Info.plist            – app bundle metadata
  AppIcon.icns          – generated via scripts/generate-macos-icon.swift
scripts/
  install-local.sh      – build → package → install → launch
  package-macos-app.sh  – assembles the .app bundle
  generate-macos-icon.swift – generates AppIcon.icns programmatically
```

## Roadmap

- [ ] **Launch at Login** — toggle via menu using `SMAppService`
- [ ] **Download limiting** — extend dummynet pipe to cover inbound traffic
- [ ] **Per-interface limiting** — apply the limit to a specific interface (Wi-Fi / Ethernet)
- [ ] **Scheduled limits** — apply a limit between configurable hours
- [ ] **Traffic sparkline** — mini graph in the menu showing the last 60 s of throughput
- [ ] **Auto-disable on sleep/wake** — detect `IOPMrootDomain` notifications and manage state
- [ ] **Global hotkey** — toggle the active limit without opening the menu

## License

MIT © [phaistonian](https://github.com/phaistonian)
