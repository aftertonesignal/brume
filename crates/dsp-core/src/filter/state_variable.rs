// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! State-variable filter — multi-mode filter with resonance.

#![allow(clippy::must_use_candidate)]

use std::f32::consts::PI;

/// Filter mode for the state-variable filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvfMode {
    /// Lowpass: passes frequencies below cutoff.
    #[default]
    Lowpass,
    /// Highpass: passes frequencies above cutoff.
    Highpass,
    /// Bandpass: passes frequencies around cutoff.
    Bandpass,
    /// Notch (band-reject): attenuates frequencies around cutoff.
    Notch,
    /// Allpass: phase shift without amplitude change.
    Allpass,
}

/// State-variable filter with multiple output modes and resonance.
///
/// Topology simultaneously computes lowpass, highpass, and bandpass —
/// mode switching doesn't need coefficient recalculation. Resonance is
/// stable up to self-oscillation.
///
/// # Example
///
/// ```
/// use brume_dsp_core::filter::{StateVariableFilter, SvfMode};
///
/// let mut filter = StateVariableFilter::new(44100.0);
/// filter.set_mode(SvfMode::Lowpass);
/// filter.set_cutoff(1000.0);
/// filter.set_resonance(0.5);
/// let output = filter.process(1.0);
/// ```
pub struct StateVariableFilter {
    mode: SvfMode,
    cutoff: f32,
    resonance: f32,
    sample_rate: f32,
    ic1eq: f32,
    ic2eq: f32,
}

impl StateVariableFilter {
    /// Creates a new state-variable filter. Default: lowpass at 1 kHz,
    /// no resonance.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            mode: SvfMode::Lowpass,
            cutoff: 1000.0,
            resonance: 0.0,
            sample_rate,
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }

    /// Sets the sample rate and re-clamps cutoff to the new Nyquist.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.cutoff = self.cutoff.clamp(20.0, sample_rate * 0.49);
    }

    /// Resets the filter state.
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Sets the filter mode.
    pub fn set_mode(&mut self, mode: SvfMode) {
        self.mode = mode;
    }

    /// Returns the current filter mode.
    pub fn mode(&self) -> SvfMode {
        self.mode
    }

    /// Sets the cutoff frequency in Hz. Clamped to 20 Hz → Nyquist.
    pub fn set_cutoff(&mut self, freq_hz: f32) {
        self.cutoff = freq_hz.clamp(20.0, self.sample_rate * 0.49);
    }

    /// Returns the current cutoff frequency in Hz.
    pub fn cutoff(&self) -> f32 {
        self.cutoff
    }

    /// Sets the resonance amount. 0.0 = none, 1.0 = self-oscillation
    /// threshold; values above 0.95 may self-oscillate.
    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 1.0);
    }

    /// Returns the current resonance amount.
    pub fn resonance(&self) -> f32 {
        self.resonance
    }

    /// Processes input and returns `(lowpass, bandpass, highpass)` —
    /// useful when a caller wants multiple outputs from one pass.
    pub fn process_multi(&mut self, input: f32) -> (f32, f32, f32) {
        let g = (PI * self.cutoff / self.sample_rate).tan();
        // k controls resonance: 2.0 = no resonance, 0.1 = max resonance
        let k = 2.0 - 1.9 * self.resonance;

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        // Flush subnormal state. Without it, an SVF that's been
        // playing into silence drifts into the IEEE 754 subnormal
        // range over a few seconds and pays the slow-path FPU tax on
        // every subsequent sample, even at zero input. See the
        // dsp-core::denormal module for the rationale.
        self.ic1eq = crate::flush_subnormal(2.0 * v1 - self.ic1eq);
        self.ic2eq = crate::flush_subnormal(2.0 * v2 - self.ic2eq);

        let low = v2;
        let band = v1;
        let high = input - k * v1 - v2;

        (low, band, high)
    }

    /// Processes a single sample through the filter.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let (low, band, high) = self.process_multi(input);

        match self.mode {
            SvfMode::Lowpass => low,
            SvfMode::Highpass => high,
            SvfMode::Bandpass => band,
            SvfMode::Notch => low + high,
            SvfMode::Allpass => low + high - band,
        }
    }

    /// Processes a block of samples in-place. More efficient than
    /// calling `process()` per sample because coefficients compute once.
    pub fn process_block(&mut self, samples: &mut [f32]) {
        let g = (std::f32::consts::PI * self.cutoff / self.sample_rate).tan();
        let k = 2.0 - 1.9 * self.resonance;

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let mut ic1eq = self.ic1eq;
        let mut ic2eq = self.ic2eq;
        let mode = self.mode;

        for sample in samples.iter_mut() {
            let input = *sample;
            let v3 = input - ic2eq;
            let v1 = a1 * ic1eq + a2 * v3;
            let v2 = ic2eq + a2 * ic1eq + a3 * v3;

            ic1eq = 2.0 * v1 - ic1eq;
            ic2eq = 2.0 * v2 - ic2eq;

            let low = v2;
            let band = v1;
            let high = input - k * v1 - v2;

            *sample = match mode {
                SvfMode::Lowpass => low,
                SvfMode::Highpass => high,
                SvfMode::Bandpass => band,
                SvfMode::Notch => low + high,
                SvfMode::Allpass => low + high - band,
            };
        }

        self.ic1eq = ic1eq;
        self.ic2eq = ic2eq;
    }
}

impl Default for StateVariableFilter {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_attenuates_high_freq() {
        let mut filter = StateVariableFilter::new(48000.0);
        filter.set_cutoff(100.0);
        filter.set_mode(SvfMode::Lowpass);

        let freq = 10000.0;
        let mut max_output = 0.0_f32;
        for i in 0..1000 {
            let phase = 2.0 * PI * freq * (i as f32) / 48000.0;
            let output = filter.process(phase.sin());
            max_output = max_output.max(output.abs());
        }
        assert!(
            max_output < 0.1,
            "lowpass should attenuate 10kHz; got {max_output}"
        );
    }

    #[test]
    fn highpass_attenuates_low_freq() {
        let mut filter = StateVariableFilter::new(48000.0);
        filter.set_cutoff(5000.0);
        filter.set_mode(SvfMode::Highpass);
        for _ in 0..1000 {
            filter.process(0.0);
        }

        let freq = 100.0;
        let mut max_output = 0.0_f32;
        for i in 0..1000 {
            let phase = 2.0 * PI * freq * (i as f32) / 48000.0;
            let output = filter.process(phase.sin());
            max_output = max_output.max(output.abs());
        }
        assert!(
            max_output < 0.2,
            "highpass should attenuate 100Hz; got {max_output}"
        );
    }

    #[test]
    fn resonance_boosts_cutoff() {
        let mut filter = StateVariableFilter::new(48000.0);
        filter.set_cutoff(1000.0);
        filter.set_mode(SvfMode::Lowpass);

        filter.set_resonance(0.0);
        filter.reset();
        let mut max_no_reso = 0.0_f32;
        for i in 0..2000 {
            let phase = 2.0 * PI * 1000.0 * (i as f32) / 48000.0;
            let output = filter.process(phase.sin());
            if i > 500 {
                max_no_reso = max_no_reso.max(output.abs());
            }
        }

        filter.set_resonance(0.9);
        filter.reset();
        let mut max_with_reso = 0.0_f32;
        for i in 0..2000 {
            let phase = 2.0 * PI * 1000.0 * (i as f32) / 48000.0;
            let output = filter.process(phase.sin());
            if i > 500 {
                max_with_reso = max_with_reso.max(output.abs());
            }
        }

        assert!(
            max_with_reso > max_no_reso,
            "resonance should boost signal at cutoff"
        );
    }

    #[test]
    fn reset_silences() {
        let mut filter = StateVariableFilter::new(48000.0);
        filter.set_cutoff(1000.0);
        filter.set_resonance(0.9);
        for _ in 0..100 {
            filter.process(1.0);
        }
        filter.reset();
        let output = filter.process(0.0);
        assert!(output.abs() < 0.0001, "filter should be silent after reset");
    }
}
