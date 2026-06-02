// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Target selection — resolves `--host` / `BRUME_HOST` env / config defaults
//! into a single `host:port`-ish string for ssh/scp.
//!
//! Trust model is the operator's existing SSH access to the CM5: no
//! pairing, no tokens; SSH finds the right key via agent or ~/.ssh/config.

use std::fmt;

// Matches the first-user account cloud-init provisions on a fresh
// Pi OS Lite + brumectl-seeded install (see
// apps/brumectl/src/cmds/flash/ops.rs::render_user_data). Root SSH is
// not enabled out of the box. Override with --host or $BRUME_HOST
// if you've already flashed a device with a different username.
const DEFAULT_HOST: &str = "brume@brume.local";

#[derive(Debug, Clone)]
pub struct Target {
    pub ssh_target: String,
    pub ssh_key: Option<String>,
}

impl Target {
    pub fn resolve(cli_host: Option<String>, cli_key: Option<String>) -> Self {
        let ssh_target = cli_host
            .or_else(|| std::env::var("BRUME_HOST").ok())
            .unwrap_or_else(|| DEFAULT_HOST.to_string());
        let ssh_key = cli_key.or_else(|| std::env::var("BRUME_SSH_KEY").ok());
        Self {
            ssh_target,
            ssh_key,
        }
    }

    /// Extra ssh/scp args to inject the identity file if set. Returned as a
    /// `Vec` so callers can splice it into a `Command::args(...)` builder.
    pub fn key_args(&self) -> Vec<String> {
        self.ssh_key
            .as_ref()
            .map(|k| vec!["-i".to_string(), k.clone()])
            .unwrap_or_default()
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.ssh_target)
    }
}
