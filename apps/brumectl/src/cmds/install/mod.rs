// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! `brumectl install` — SSH-driven Brume install onto a user-provisioned
//! Pi OS Lite CM5.
//!
//! The onboarding path. Takes a
//! running, network-reachable CM5 with Pi OS Lite already installed
//! (hostname, user, SSH key, WiFi all configured upstream) and installs
//! over SSH: apt deps, the brume binary, factory presets, appliance
//! configs, autologin, and labwc autostart.
//!
//! Every config file deployed is `include_*!`'d from `deploy/`
//! (`config/` + `systemd/`), so the brumectl binary carries its own
//! install payload — no external files needed at install time.
//!
//! Idempotent: re-running on an already-installed CM5 is safe. apt sees
//! packages already present, configs get overwritten with identical
//! bytes, systemd unit-file updates are reload-on-change.

use anyhow::{Context, Result};

use super::target::Target;

mod fetch;
mod phases;

/// Options gathered from `brumectl install` CLI flags.
pub struct Opts {
    /// Print what would be done without actually touching the target.
    /// `probe` still runs (to verify connectivity); every other phase
    /// logs its intent and skips the remote side.
    pub dry_run: bool,

    /// Content-refresh path: re-stage the brume binary and the factory
    /// presets, then restart brume.service. Skips the one-time appliance
    /// plumbing (apt, configs, Meridian, autologin) — those only change on
    /// a full install.
    pub update: bool,

    /// Explicit path to the aarch64 brume binary. Overrides env +
    /// workspace autodetection when Some.
    pub binary_override: Option<String>,

    /// Refuse to fetch the brume binary from GitHub Releases when no
    /// local build is found. Useful for offline / strict-CI flows
    /// where an unexpected download would surprise the operator.
    pub no_download: bool,

    /// Re-fetch from GitHub Releases even if the user-cache copy at
    /// the matching version already exists. For the rare case of a
    /// release re-cut under the same tag.
    pub force_download: bool,
}

pub fn run(target: &Target, opts: Opts) -> Result<()> {
    let mode = match (opts.dry_run, opts.update) {
        (true, true) => "update, dry-run",
        (true, false) => "dry-run",
        (false, true) => "update",
        (false, false) => "install",
    };
    println!("brumectl install → {target}  ({mode})");

    phases::probe(target).context("probe failed")?;

    if !opts.update {
        phases::apt_install(target, &opts).context("apt phase failed")?;
    }
    phases::stage_binary(target, &opts).context("binary phase failed")?;
    phases::stage_factory(target, &opts).context("factory phase failed")?;
    if !opts.update {
        phases::deploy_configs(target, &opts).context("config phase failed")?;
        phases::stage_meridian(target, &opts).context("meridian phase failed")?;
        phases::enable_autologin(target, &opts).context("autologin phase failed")?;
    } else {
        phases::restart_brume(target, &opts).context("restart phase failed")?;
    }
    phases::verify(target, &opts).context("verify phase failed")?;

    println!();
    if opts.dry_run {
        println!("dry-run complete. Re-run without --dry-run to apply.");
    } else if opts.update {
        println!("✓ Brume binary refreshed on {target}.");
    } else {
        println!("✓ Brume install complete.");
        println!("  UI on the CM5's HDMI display; ssh {target} to manage.");
    }
    Ok(())
}
