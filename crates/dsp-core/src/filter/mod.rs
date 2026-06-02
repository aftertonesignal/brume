// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Audio filters used by the Brume engine.
//!
//! - [`OnePole`] — simple lowpass / highpass with 6 dB/oct slopes
//! - [`DcBlocker`] — very-low-cutoff highpass for DC offset removal
//! - [`StateVariableFilter`] — multi-mode filter with resonance
//!
//! The kit's `Biquad` and `CombFilter` are intentionally not vendored —
//! Brume doesn't consume them.

mod dc_blocker;
mod one_pole;
mod state_variable;

pub use dc_blocker::DcBlocker;
pub use one_pole::OnePole;
pub use state_variable::{StateVariableFilter, SvfMode};
