// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Brume audio engine — voice management, message dispatch, and block rendering.

mod engine;
mod fm_oscillator;
mod granular_oscillator;
mod harmonic_oscillator;
mod oscillator_core;
mod part;
mod timbral_oscillator;
mod transport;
mod voice;

pub use engine::BrumeEngine;
pub use fm_oscillator::{ALGORITHMS, Algorithm};
pub use part::Part;
pub use transport::{ClockMode, TimeDivision, Transport};
pub use voice::BrumeVoice;
