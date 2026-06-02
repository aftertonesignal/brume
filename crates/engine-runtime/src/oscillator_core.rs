// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Oscillator mode dispatch — wraps the active oscillator architecture.
//!
//! Each architecture (FM, Harmonic, Timbral, Granular) implements the
//! [`Engine`] trait so a `BrumeVoice` can hand it a `(ParameterId, f32)`
//! pair without knowing the concrete engine type. The audio hot path
//! (`set_frequency`, `process`, `reset`) stays as inherent methods on
//! `OscillatorCore` to avoid a virtual call inside the inner loop.

use brume_common::ParameterId;

use crate::fm_oscillator::FmOscillator;
use crate::granular_oscillator::GranularOscillator;
use crate::harmonic_oscillator::HarmonicOscillator;
use crate::timbral_oscillator::TimbralOscillator;

/// Message-driven write surface every oscillator architecture exposes
/// to a voice. Engines silently ignore parameters that don't belong to
/// their architecture, so the voice can hand any `ParameterId` to
/// whichever engine happens to be loaded in its slot.
pub trait Engine {
    fn set_parameter(&mut self, id: ParameterId, value: f32);
}

/// The active oscillator mode for a voice.
///
/// Each variant contains the full oscillator state for that mode.
// Granular is ~6× the other variants; boxing would add a hot-path indirection.
#[allow(clippy::large_enum_variant)]
pub enum OscillatorCore {
    Fm(FmOscillator),
    Harmonic(HarmonicOscillator),
    Timbral(TimbralOscillator),
    Granular(GranularOscillator),
}

impl OscillatorCore {
    #[must_use]
    pub fn new_fm(sample_rate: f32) -> Self {
        Self::Fm(FmOscillator::new(sample_rate))
    }

    #[must_use]
    pub fn new_harmonic(sample_rate: f32) -> Self {
        Self::Harmonic(HarmonicOscillator::new(sample_rate))
    }

    #[must_use]
    pub fn new_timbral(sample_rate: f32) -> Self {
        Self::Timbral(TimbralOscillator::new(sample_rate))
    }

    #[must_use]
    pub fn new_granular(sample_rate: f32) -> Self {
        Self::Granular(GranularOscillator::new(sample_rate))
    }

    pub fn set_frequency(&mut self, freq: f32) {
        match self {
            Self::Fm(o) => o.set_frequency(freq),
            Self::Harmonic(o) => o.set_frequency(freq),
            Self::Timbral(o) => o.set_frequency(freq),
            Self::Granular(o) => o.set_frequency(freq),
        }
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        match self {
            Self::Fm(o) => o.process(),
            Self::Harmonic(o) => o.process(),
            Self::Timbral(o) => o.process(),
            Self::Granular(o) => o.process(),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Fm(o) => o.reset(),
            Self::Harmonic(o) => o.reset(),
            Self::Timbral(o) => o.reset(),
            Self::Granular(o) => o.reset(),
        }
    }

    /// Returns true if the filter and DC blocker should be reset on note-on.
    /// Granular needs continuous filter state since grains survive across notes.
    #[inline]
    pub fn needs_hard_reset(&self) -> bool {
        !matches!(self, Self::Granular(_))
    }

    /// Push the per-sample effective FM index into the FM engine.
    /// No-op on other modes — the voice's `fm_index_base` smoother is
    /// only meaningful when the FM engine is loaded.
    pub fn set_fm_index(&mut self, value: f32) {
        if let Self::Fm(osc) = self {
            osc.set_fm_index(value);
        }
    }
}

impl Engine for OscillatorCore {
    fn set_parameter(&mut self, id: ParameterId, value: f32) {
        match self {
            Self::Fm(o) => o.set_parameter(id, value),
            Self::Harmonic(o) => o.set_parameter(id, value),
            Self::Timbral(o) => o.set_parameter(id, value),
            Self::Granular(o) => o.set_parameter(id, value),
        }
    }
}
