// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! `brumectl shell` — pass-through to ssh.

use std::process::Command;

use anyhow::{Context, Result};

use super::target::Target;

pub fn run(target: &Target) -> Result<()> {
    let status = Command::new("ssh")
        .args(target.key_args())
        .arg(&target.ssh_target)
        .status()
        .with_context(|| format!("failed to invoke ssh for {target}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
