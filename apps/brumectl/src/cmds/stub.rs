// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Stub placeholder for verbs whose design is defined but implementation
//! is sequenced later.

use anyhow::Result;

pub fn not_implemented(verb: &str) -> Result<()> {
    eprintln!("brumectl: `{verb}` is not implemented yet.");
    eprintln!("  it is on the brumectl roadmap, not yet wired up.");
    std::process::exit(2);
}
