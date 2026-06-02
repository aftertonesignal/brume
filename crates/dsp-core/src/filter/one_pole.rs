// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! One-pole filter.

use std::f32::consts::PI;

/// A simple one-pole filter for lowpass or highpass filtering. Provides
/// gentle 6 dB/octave slopes — useful for smoothing control signals or
/// gentle high-frequency roll-off.
///
/// # Example
///
/// ```
/// use brume_dsp_core::filter::OnePole;
///
/// let mut filter = OnePole::lowpass(44100.0);
/// filter.set_cutoff(1000.0);
/// let output = filter.process(1.0);
/// ```
pub struct OnePole {
    sample_rate: f32,
    cutoff: f32,
    coefficient: f32,
    state: f32,
    highpass: bool,
}

impl OnePole {
    /// Creates a new lowpass one-pole filter.
    #[must_use]
    pub fn lowpass(sample_rate: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            cutoff: 1000.0,
            coefficient: 0.0,
            state: 0.0,
            highpass: false,
        };
        filter.update_coefficient();
        filter
    }

    /// Creates a new highpass one-pole filter.
    #[must_use]
    pub fn highpass(sample_rate: f32) -> Self {
        let mut filter = Self {
            sample_rate,
            cutoff: 1000.0,
            coefficient: 0.0,
            state: 0.0,
            highpass: true,
        };
        filter.update_coefficient();
        filter
    }

    /// Sets the sample rate and recalculates coefficients.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update_coefficient();
    }

    /// Sets the cutoff frequency in Hz.
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.max(1.0).min(self.sample_rate * 0.49);
        self.update_coefficient();
    }

    /// Resets the filter state to zero.
    pub fn reset(&mut self) {
        self.state = 0.0;
    }

    /// Processes a single sample through the filter.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.state = input + self.coefficient * (self.state - input);
        if self.highpass {
            input - self.state
        } else {
            self.state
        }
    }

    fn update_coefficient(&mut self) {
        let omega = 2.0 * PI * self.cutoff / self.sample_rate;
        self.coefficient = (-omega).exp();
    }
}

impl Default for OnePole {
    fn default() -> Self {
        Self::lowpass(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_passes_dc() {
        let mut filter = OnePole::lowpass(44100.0);
        filter.set_cutoff(1000.0);
        for _ in 0..1000 {
            filter.process(1.0);
        }
        assert!((filter.process(1.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn highpass_blocks_dc() {
        let mut filter = OnePole::highpass(44100.0);
        filter.set_cutoff(100.0);
        for _ in 0..10000 {
            filter.process(1.0);
        }
        assert!(filter.process(1.0).abs() < 0.01);
    }

    #[test]
    fn reset_clears_state() {
        let mut filter = OnePole::lowpass(44100.0);
        filter.process(1.0);
        filter.process(1.0);
        assert!(filter.state != 0.0);
        filter.reset();
        assert_eq!(filter.state, 0.0);
    }
}
