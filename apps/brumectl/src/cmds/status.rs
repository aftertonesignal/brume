// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! `brumectl status` — quick health snapshot of a connected Brume.
//!
//! The ssh probe + parse is exposed as `probe()` so the TUI can reuse it
//! for its Device panel instead of duplicating the remote shell.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use super::target::Target;

const REMOTE_PROBE: &str = r#"
set +e
echo "=os-release="
cat /etc/os-release 2>/dev/null | head -20
echo "=uname="
uname -r
echo "=uptime="
cat /proc/uptime
echo "=meminfo="
awk '/^MemTotal|^MemAvailable|^MemFree/ {print $1, $2}' /proc/meminfo
echo "=tempm="
cat /sys/class/thermal/thermal_zone0/temp 2>/dev/null || echo ""
echo "=brume_pid="
pgrep -x brume 2>/dev/null || echo ""
echo "=eth0_addr="
ip -4 -o addr show eth0 2>/dev/null | awk '{print $4}' | cut -d/ -f1
echo "=wlan0_addr="
ip -4 -o addr show wlan0 2>/dev/null | awk '{print $4}' | cut -d/ -f1
"#;

#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub firmware: String,
    pub kernel: String,
    pub uptime: Duration,
    pub temp_celsius: Option<f32>,
    pub mem_used_mb: u64,
    pub mem_avail_mb: u64,
    pub brume_pid: Option<String>,
    pub eth_addr: Option<String>,
    pub wlan_addr: Option<String>,
}

/// Probe the target once. Short timeout so the TUI stays responsive
/// when the device is unreachable.
/// Probe result — also carries the host we actually reached, which may
/// differ from what the user passed on the command line if we had to
/// fall back to an ARP-scanned IP.
pub struct ProbeOk {
    pub info: DeviceInfo,
    pub reached: String,
}

pub fn probe(target: &Target) -> Result<ProbeOk> {
    probe_target(&target.ssh_target, target)
}

fn probe_target(ssh_target: &str, target: &Target) -> Result<ProbeOk> {
    let out = Command::new("ssh")
        .args(target.key_args())
        .args(["-o", "ConnectTimeout=3", "-o", "BatchMode=yes"])
        .arg(ssh_target)
        .arg(REMOTE_PROBE)
        .output()
        .with_context(|| "failed to run ssh — is it on PATH?")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.trim().to_string();
        // Auth failure — the device *is* reachable, don't ARP-scan;
        // we'd just hit the same key rejection on another IP.
        let is_auth = msg.contains("Permission denied")
            || msg.contains("publickey")
            || msg.contains("No supported authentication methods");
        // Otherwise — network, resolve, or unknown failure — silently
        // ARP-scan for Pi-OUI hosts before giving up. Covers both the
        // "mDNS didn't resolve" and "device DHCP'd to a new IP" cases.
        if !is_auth {
            if let Some(ok) = probe_via_arp_fallback(target) {
                return Ok(ok);
            }
        }
        return Err(anyhow!("{}", categorize_ssh_failure(&msg)));
    }

    let raw = String::from_utf8_lossy(&out.stdout);
    let sections = parse_sections(&raw);
    Ok(ProbeOk {
        info: build_device_info(&sections),
        reached: ssh_target.to_string(),
    })
}

fn probe_via_arp_fallback(target: &Target) -> Option<ProbeOk> {
    use super::discovery::pi_ips_from_arp;
    let user = target
        .ssh_target
        .split_once('@')
        .map(|(u, _)| format!("{u}@"))
        .unwrap_or_default();
    for ip in pi_ips_from_arp() {
        let candidate = format!("{user}{ip}");
        if let Ok(ok) = probe_target_direct(&candidate, target) {
            return Some(ok);
        }
    }
    None
}

fn categorize_ssh_failure(stderr: &str) -> String {
    // Auth — device is reachable but SSH key doesn't match.
    if stderr.contains("Permission denied")
        || stderr.contains("publickey")
        || stderr.contains("No supported authentication methods")
    {
        return "SSH key didn't match. The device trusts a different key than what your Mac \
             is offering.\nTry `ssh-add <your-key>`, or add your pubkey to ~brume/.ssh/authorized_keys \
             on the device."
            .to_string();
    }
    // Network — nothing on the other end.
    if stderr.contains("No route to host")
        || stderr.contains("Connection refused")
        || stderr.contains("Connection timed out")
        || stderr.contains("Operation timed out")
        || stderr.contains("Could not resolve")
        || stderr.contains("nodename nor servname")
    {
        return "Couldn't reach Brume. Is it powered on and connected?".to_string();
    }
    // Host key mismatch — image was reflashed with different host keys.
    if stderr.contains("HOST IDENTIFICATION HAS CHANGED")
        || stderr.contains("REMOTE HOST IDENTIFICATION")
    {
        return "SSH host-key changed (expected after a reflash). Clear the old fingerprint:\n\
             `ssh-keygen -R <host>`  then retry."
            .to_string();
    }
    // Default — be honest rather than guess.
    format!(
        "SSH to Brume failed: {}",
        stderr.lines().next().unwrap_or("unknown")
    )
}

fn probe_target_direct(ssh_target: &str, target: &Target) -> Result<ProbeOk> {
    let out = Command::new("ssh")
        .args(target.key_args())
        .args([
            "-o",
            "ConnectTimeout=3",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
        ])
        .arg(ssh_target)
        .arg(REMOTE_PROBE)
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("unreachable"));
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    Ok(ProbeOk {
        info: build_device_info(&parse_sections(&raw)),
        reached: ssh_target.to_string(),
    })
}

pub fn run(target: &Target) -> Result<()> {
    let ok = probe(target)?;
    println!();
    println!("  Brume  ·  {}", strip_user(&ok.reached));
    println!();
    print_report(&ok.info);
    println!();
    Ok(())
}

fn strip_user(s: &str) -> &str {
    s.split_once('@').map(|(_, h)| h).unwrap_or(s)
}

fn build_device_info(s: &HashMap<String, String>) -> DeviceInfo {
    let firmware = s
        .get("os-release")
        .and_then(|v| v.lines().find(|l| l.starts_with("PRETTY_NAME=")))
        .map(|l| {
            l.trim_start_matches("PRETTY_NAME=")
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string());

    let kernel = s.get("uname").cloned().unwrap_or_default();

    let uptime = s
        .get("uptime")
        .and_then(|u| u.split_whitespace().next()?.parse::<f64>().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or_default();

    let mem = s
        .get("meminfo")
        .map(|m| parse_meminfo(m))
        .unwrap_or_default();
    let mem_total_kb = *mem.get("MemTotal").unwrap_or(&0);
    let mem_avail_kb = *mem.get("MemAvailable").unwrap_or(&0);
    let mem_used_mb = mem_total_kb.saturating_sub(mem_avail_kb) / 1024;
    let mem_avail_mb = mem_avail_kb / 1024;

    let temp_celsius = s
        .get("tempm")
        .filter(|v| !v.is_empty())
        .and_then(|t| t.parse::<i32>().ok())
        .map(|milli| milli as f32 / 1000.0);

    let brume_pid = s.get("brume_pid").filter(|v| !v.is_empty()).cloned();
    let eth_addr = s.get("eth0_addr").filter(|v| !v.is_empty()).cloned();
    let wlan_addr = s.get("wlan0_addr").filter(|v| !v.is_empty()).cloned();

    DeviceInfo {
        firmware,
        kernel,
        uptime,
        temp_celsius,
        mem_used_mb,
        mem_avail_mb,
        brume_pid,
        eth_addr,
        wlan_addr,
    }
}

fn parse_sections(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current: Option<String> = None;
    let mut buf = String::new();
    for line in raw.lines() {
        if let Some(name) = line.strip_prefix("=").and_then(|s| s.strip_suffix("=")) {
            if let Some(k) = current.take() {
                map.insert(k, std::mem::take(&mut buf).trim().to_string());
            }
            current = Some(name.to_string());
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(k) = current {
        map.insert(k, buf.trim().to_string());
    }
    map
}

fn print_report(info: &DeviceInfo) {
    let kv = |k: &str, v: &str| println!("  {k:<10}  {v}");
    kv("firmware", &info.firmware);
    kv("kernel", &info.kernel);
    kv("uptime", &format_uptime(info.uptime));
    kv(
        "memory",
        &format!(
            "{} MB used  ·  {} MB free",
            info.mem_used_mb, info.mem_avail_mb
        ),
    );
    if let Some(c) = info.temp_celsius {
        kv("temp", &format!("{c:.1} °C"));
    }
    kv(
        "brume",
        &info.brume_pid.as_ref().map_or_else(
            || "not running".to_string(),
            |pid| format!("running (pid {pid})"),
        ),
    );
    let mut parts = Vec::new();
    if let Some(a) = &info.eth_addr {
        parts.push(format!("eth0 {a}"));
    }
    if let Some(a) = &info.wlan_addr {
        parts.push(format!("wlan0 {a}"));
    }
    if !parts.is_empty() {
        kv("network", &parts.join(", "));
    }
}

fn parse_meminfo(raw: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let mut it = line.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            let key = k.trim_end_matches(':').to_string();
            if let Ok(n) = v.parse::<u64>() {
                out.insert(key, n);
            }
        }
    }
    out
}

pub fn format_uptime(d: Duration) -> String {
    let s = d.as_secs();
    let (h, rem) = (s / 3600, s % 3600);
    let (m, sec) = (rem / 60, rem % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}
