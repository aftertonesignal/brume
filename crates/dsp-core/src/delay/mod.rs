// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Delay lines for audio processing.
//!
//! - [`DelayLine`] — fixed-size delay buffer with fractional-delay interpolation
//! - [`AllpassDelay`] — Schroeder allpass for reverb diffusion
//! - [`ModulatedDelay`] — LFO-modulated delay for chorus / flanger

mod allpass;
mod delay_line;
mod modulated;

pub use allpass::AllpassDelay;
pub use delay_line::DelayLine;
pub use modulated::ModulatedDelay;
