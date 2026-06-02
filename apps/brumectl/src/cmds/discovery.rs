// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Device discovery helpers shared across verbs.
//!
//! Minimal-image Brume units don't ship avahi (yet), so `<host>.local`
//! doesn't resolve on the Mac even when the CM5 is happily DHCP'd onto
//! the LAN. `pi_ips_from_arp` scrapes the Mac's ARP cache for any MAC
//! with a Raspberry Pi Foundation OUI and returns the paired IPv4 —
//! good enough to try as a fallback when mDNS resolution fails.

use std::process::Command;

const PI_OUIS: &[&str] = &[
    "2c:cf:67", // Pi 4 / Pi 5 / CM5
    "dc:a6:32", // Pi 4
    "e4:5f:01", // Pi 4
    "b8:27:eb", // older Pi
    "d8:3a:dd", // newer Pi
];

pub fn pi_ips_from_arp() -> Vec<String> {
    let out = match Command::new("arp").arg("-a").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut ips: Vec<String> = Vec::new();
    for line in text.lines() {
        let lower = line.to_lowercase();
        if !PI_OUIS.iter().any(|oui| lower.contains(oui)) {
            continue;
        }
        if let Some(open) = line.find('(') {
            if let Some(close) = line[open..].find(')') {
                let ip = &line[open + 1..open + close];
                if !ip.is_empty() && ip.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    let ip = ip.to_string();
                    // macOS arp -a lists the same IP once per interface
                    // (en0 + en1, etc.) — dedupe.
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
    }
    ips
}
