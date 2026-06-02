// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Linux system telemetry for the SYS page. Reads procfs / sysfs
//! directly so there's no dependency on shell tools (`vcgencmd`,
//! `free`, etc.) that may or may not be in the image.
//!
//! Each `read_*` function returns a fully-formatted display string,
//! falling back to `"--"` on any failure — the SYS rows just hold the
//! last-good value when a read trips, which beats showing an error
//! card the user can't act on.

#[derive(Debug, Default, Clone)]
pub struct Telemetry {
    /// Formatted CPU/SoC temperature, e.g. "48.2°C".
    pub temperature: String,
    /// Used / total memory, e.g. "679 / 8058 MB".
    pub memory: String,
    /// Uptime, e.g. "2h 14m".
    pub uptime: String,
    /// Active monitor — manufacturer + model from EDID, e.g.
    /// "MAGEX 1920×1200" or just "HDMI 1920×1200" if EDID can't be
    /// parsed but a mode is connected. Empty if nothing detected.
    pub display: String,
    /// Touchscreen device name from /proc/bus/input/devices.
    pub touch: String,
}

#[cfg(target_os = "linux")]
impl Telemetry {
    pub fn poll() -> Self {
        Self {
            temperature: read_temperature(),
            memory: read_memory(),
            uptime: read_uptime(),
            display: read_display(),
            touch: read_touch(),
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl Telemetry {
    pub fn poll() -> Self {
        // procfs / sysfs don't exist on macOS dev. Empty strings
        // render as `--` in the SYS rows; same shape the Linux side
        // produces when an individual read trips.
        Self::default()
    }
}

#[cfg(target_os = "linux")]
fn read_temperature() -> String {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|t| format!("{:.1}°C", t / 1000.0))
        .unwrap_or_else(|| "--".into())
}

#[cfg(target_os = "linux")]
fn read_memory() -> String {
    let Ok(s) = std::fs::read_to_string("/proc/meminfo") else {
        return "--".into();
    };
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    if total == 0 {
        return "--".into();
    }
    let used = total.saturating_sub(avail);
    // /proc/meminfo reports kB; bring both sides to MB.
    format!("{} / {} MB", used / 1024, total / 1024)
}

#[cfg(target_os = "linux")]
fn read_uptime() -> String {
    let Ok(s) = std::fs::read_to_string("/proc/uptime") else {
        return "--".into();
    };
    let Some(secs) = s
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
    else {
        return "--".into();
    };
    let h = (secs / 3600.0) as u32;
    let m = ((secs % 3600.0) / 60.0) as u32;
    format!("{h}h {m}m")
}

#[cfg(target_os = "linux")]
fn read_touch() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/bus/input/devices") else {
        return "--".into();
    };
    // Per-device records are separated by a blank line. Find the
    // first record whose `B: ABS=` bits are non-zero (absolute-
    // positioning input — i.e. a touch device, not a relative-
    // pointing mouse) and return its `N: Name="..."` value.
    for record in text.split("\n\n") {
        let mut name: Option<String> = None;
        let mut has_abs = false;
        for line in record.lines() {
            if let Some(rest) = line.strip_prefix("N: Name=\"") {
                if let Some(end) = rest.strip_suffix('"') {
                    name = Some(end.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("B: ABS=") {
                let v = rest.trim();
                if !v.is_empty() && v != "0" {
                    has_abs = true;
                }
            }
        }
        if has_abs {
            if let Some(n) = name {
                return n;
            }
        }
    }
    "--".into()
}

#[cfg(target_os = "linux")]
fn read_display() -> String {
    // Walk /sys/class/drm looking for a connector that's "connected"
    // and read its EDID. Connectors look like `card0-HDMI-A-1`. Skip
    // cardN-eDP / writeback / unconnected.
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return "--".into();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip the per-card root nodes (no '-' separator) and the
        // virtual writeback connector.
        if !name.contains('-') || name.contains("Writeback") {
            continue;
        }
        let status = std::fs::read_to_string(path.join("status"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if status != "connected" {
            continue;
        }
        let edid = std::fs::read(path.join("edid")).ok();
        let mode = std::fs::read_to_string(path.join("modes"))
            .ok()
            .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
            .filter(|s| !s.is_empty());
        let label = match (edid.and_then(|b| parse_edid_name(&b)), mode) {
            (Some(name), Some(m)) => format!("{name} · {m}"),
            (Some(name), None) => name,
            (None, Some(m)) => format!("{} · {m}", connector_short_name(&name)),
            (None, None) => connector_short_name(&name).to_string(),
        };
        return label;
    }
    "--".into()
}

/// Pull the manufacturer + model name out of an EDID blob. Returns
/// e.g. `"MAGEX MX-1920P"` or just the manufacturer if no descriptor
/// type 0xFC (monitor name) is present.
#[cfg(target_os = "linux")]
fn parse_edid_name(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 {
        return None;
    }
    // Manufacturer code: bytes 8–9, big-endian, 5-bit-per-letter A=1.
    let mfr_raw = u16::from_be_bytes([edid[8], edid[9]]);
    let l1 = ((mfr_raw >> 10) & 0x1F) as u8;
    let l2 = ((mfr_raw >> 5) & 0x1F) as u8;
    let l3 = (mfr_raw & 0x1F) as u8;
    let to_letter = |n: u8| -> Option<char> {
        if (1..=26).contains(&n) {
            Some((b'A' + n - 1) as char)
        } else {
            None
        }
    };
    let mfr: String = [l1, l2, l3].iter().copied().filter_map(to_letter).collect();
    if mfr.len() != 3 {
        return None;
    }

    // Detailed Timing Descriptors live at 54, 72, 90, 108 (each 18
    // bytes). A "monitor descriptor" has the first 5 bytes set to
    // 00 00 00 TT 00 where TT is the descriptor type. 0xFC = monitor
    // name (ASCII, terminated with 0x0A and padded with spaces).
    for off in [54usize, 72, 90, 108] {
        if edid.len() < off + 18 {
            break;
        }
        let block = &edid[off..off + 18];
        if block[0] == 0 && block[1] == 0 && block[2] == 0 && block[4] == 0 && block[3] == 0xFC {
            let name_bytes = &block[5..18];
            let name: String = name_bytes
                .iter()
                .take_while(|&&b| b != 0x0A)
                .map(|&b| b as char)
                .collect();
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(format!("{mfr} {trimmed}"));
            }
        }
    }
    Some(mfr)
}

#[cfg(target_os = "linux")]
fn connector_short_name(full: &str) -> &str {
    // `card0-HDMI-A-1` → `HDMI-A-1`. Falls back to the full name if
    // the strip pattern doesn't match.
    full.split_once('-').map_or(full, |(_, rest)| rest)
}
