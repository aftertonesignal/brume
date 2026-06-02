// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! DSP core for Brume.
//!
//! In-tree DSP primitives. The audio-primitive modules (`delay`,
//! `dynamics`, `envelope`, `filter`, `saturation`, `utility`) are
//! GPL-3.0-only and vendored from the private `aftertone-dsp-kit`
//! library — see each file's SPDX header. The Brume-specific
//! primitives ([`PhaseOscillator`], [`morph_wave`], [`Smoother`]) are
//! authored directly for this crate.
//!
//! The surface Brume actually consumes is re-exported at the crate
//! root for convenience; module paths (`brume_dsp_core::delay::…`,
//! `brume_dsp_core::filter::…`, etc.) are also available for code that
//! prefers them.

pub mod delay;
pub mod dynamics;
pub mod envelope;
pub mod filter;
pub mod saturation;
pub mod utility;

mod denormal;
mod halfband;
mod phase_oscillator;
mod sine_lut;
mod smoother;

pub use denormal::flush_subnormal;
pub use halfband::HalfbandDecimator;
pub use phase_oscillator::{PhaseOscillator, morph_wave};
pub use sine_lut::{sine_normalized, sine_radians};
pub use smoother::Smoother;

// Convenience re-exports — types imported across multiple downstream
// crates via `use brume_dsp_core::{…};`.
pub use dynamics::Limiter;
pub use envelope::{Adsr, AdsrStage};
pub use filter::{DcBlocker, StateVariableFilter, SvfMode};
pub use saturation::Wavefolder;
pub use utility::one_pole_coeff;
