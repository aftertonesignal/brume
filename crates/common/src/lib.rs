// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Shared types and error definitions for Brume.

pub mod error;
pub mod ipc;
pub mod types;

pub use error::BrumeError;
pub use types::{KnobBinding, KnobMapping, LatencyPreset, OscillatorMode, ParameterId, Scale};
