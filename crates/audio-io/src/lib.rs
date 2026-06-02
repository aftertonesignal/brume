// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Audio I/O backend abstraction for Brume.
//!
//! Provides a trait-based interface so the engine can run on different
//! audio backends (cpal for prototyping, ALSA for tighter Pi control).

mod backend;
mod cpal_backend;

pub use backend::{AudioBackend, AudioError, AudioOutputConfig, AudioStream, OutputDeviceInfo};
pub use cpal_backend::{CpalBackend, friendly_label};
