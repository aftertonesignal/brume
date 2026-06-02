// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Error types for Brume.

/// Top-level error type for Brume operations.
#[derive(Debug, thiserror::Error)]
pub enum BrumeError {
    #[error("audio: {0}")]
    Audio(String),

    #[error("midi: {0}")]
    Midi(String),

    #[error("config: {0}")]
    Config(String),
}
