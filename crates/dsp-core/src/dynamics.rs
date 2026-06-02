// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Dynamics processing — the Brume engine uses the Limiter on the
//! master bus.
//!
//! The kit's `Compressor` and `Gate` are intentionally not vendored —
//! nothing in the Brume code path uses them.

#![allow(clippy::must_use_candidate)]

use crate::utility::{db_to_gain, gain_to_db, one_pole_coeff};

/// Safety limiter with instant attack and smooth release. Tracks the
/// signal envelope and applies gain reduction when the signal exceeds
/// the threshold; instant attack catches transients, release time
/// tunes transparency.
///
/// # Example
///
/// ```
/// use brume_dsp_core::dynamics::Limiter;
///
/// let mut limiter = Limiter::new(44100.0);
/// limiter.set_threshold_db(-1.0);
///
/// let output = limiter.process(1.5);
/// assert!(output <= 1.0);
/// ```
pub struct Limiter {
    threshold: f32,
    release_coeff: f32,
    envelope: f32,
    sample_rate: f32,
    release_ms: f32,
}

impl Limiter {
    /// Creates a new limiter. Defaults: -0.5 dB threshold, 100ms release.
    pub fn new(sample_rate: f32) -> Self {
        let mut limiter = Self {
            threshold: 0.94,
            release_coeff: 0.9995,
            envelope: 0.0,
            sample_rate,
            release_ms: 100.0,
        };
        limiter.update_coefficients();
        limiter
    }

    /// Sets the sample rate and recalculates coefficients.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update_coefficients();
    }

    /// Resets internal state.
    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    /// Sets the threshold in decibels. Typical -6 dB → 0 dB; values
    /// above 0 dB effectively disable limiting for normal signals.
    pub fn set_threshold_db(&mut self, db: f32) {
        self.threshold = db_to_gain(db);
    }

    /// Returns the current threshold in decibels.
    pub fn threshold_db(&self) -> f32 {
        gain_to_db(self.threshold)
    }

    /// Sets the release time in milliseconds. 10-50ms is aggressive;
    /// 100-500ms is more transparent.
    pub fn set_release_ms(&mut self, time_ms: f32) {
        self.release_ms = time_ms.max(1.0);
        self.update_coefficients();
    }

    /// Returns the current release time in milliseconds.
    pub fn release_ms(&self) -> f32 {
        self.release_ms
    }

    /// Returns the current gain reduction in decibels (always ≤ 0).
    pub fn gain_reduction_db(&self) -> f32 {
        if self.envelope > self.threshold {
            gain_to_db(self.threshold / self.envelope)
        } else {
            0.0
        }
    }

    fn update_coefficients(&mut self) {
        self.release_coeff = one_pole_coeff(self.release_ms, self.sample_rate);
    }

    /// Processes a single sample.
    pub fn process(&mut self, input: f32) -> f32 {
        let abs_input = input.abs();

        if abs_input > self.envelope {
            self.envelope = abs_input;
        } else {
            self.envelope =
                self.envelope * self.release_coeff + abs_input * (1.0 - self.release_coeff);
        }

        let gain = if self.envelope > self.threshold {
            self.threshold / self.envelope
        } else {
            1.0
        };

        input * gain
    }

    /// Processes a stereo pair. Envelope detection uses the max of both
    /// channels so stereo balance is preserved.
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        let abs_input = left.abs().max(right.abs());

        if abs_input > self.envelope {
            self.envelope = abs_input;
        } else {
            self.envelope =
                self.envelope * self.release_coeff + abs_input * (1.0 - self.release_coeff);
        }

        let gain = if self.envelope > self.threshold {
            self.threshold / self.envelope
        } else {
            1.0
        };

        (left * gain, right * gain)
    }

    /// Processes a block of samples in-place.
    pub fn process_block(&mut self, samples: &mut [f32]) {
        let threshold = self.threshold;
        let release_coeff = self.release_coeff;
        let one_minus_release = 1.0 - release_coeff;
        let mut envelope = self.envelope;

        for sample in samples.iter_mut() {
            let input = *sample;
            let abs_input = input.abs();

            if abs_input > envelope {
                envelope = abs_input;
            } else {
                envelope = envelope * release_coeff + abs_input * one_minus_release;
            }

            let gain = if envelope > threshold {
                threshold / envelope
            } else {
                1.0
            };

            *sample = input * gain;
        }

        self.envelope = envelope;
    }

    /// Processes an interleaved stereo block in-place. Samples expected
    /// as `[L0, R0, L1, R1, …]`.
    pub fn process_block_stereo_interleaved(&mut self, samples: &mut [f32]) {
        let threshold = self.threshold;
        let release_coeff = self.release_coeff;
        let one_minus_release = 1.0 - release_coeff;
        let mut envelope = self.envelope;

        for pair in samples.chunks_exact_mut(2) {
            let left = pair[0];
            let right = pair[1];
            let abs_input = left.abs().max(right.abs());

            if abs_input > envelope {
                envelope = abs_input;
            } else {
                envelope = envelope * release_coeff + abs_input * one_minus_release;
            }

            let gain = if envelope > threshold {
                threshold / envelope
            } else {
                1.0
            };

            pair[0] = left * gain;
            pair[1] = right * gain;
        }

        self.envelope = envelope;
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_quiet_signal() {
        let mut limiter = Limiter::new(44100.0);
        limiter.set_threshold_db(0.0);
        for i in 0..100 {
            let input = (i as f32 * 0.1).sin() * 0.5;
            let output = limiter.process(input);
            assert!((input - output).abs() < 0.01);
        }
    }

    #[test]
    fn limits_loud_signal() {
        let mut limiter = Limiter::new(48000.0);
        limiter.set_threshold_db(-6.0);
        for _ in 0..100 {
            let output = limiter.process(2.0);
            assert!(output <= 0.55, "got {output}");
        }
    }

    #[test]
    fn stereo_preserves_balance() {
        let mut limiter = Limiter::new(44100.0);
        limiter.set_threshold_db(-6.0);
        let (out_l, out_r) = limiter.process_stereo(1.0, 0.5);
        assert!((out_l / out_r - 2.0).abs() < 0.01);
    }

    #[test]
    fn block_matches_sample_by_sample() {
        let mut limiter1 = Limiter::new(44100.0);
        let mut limiter2 = Limiter::new(44100.0);
        limiter1.set_threshold_db(-6.0);
        limiter2.set_threshold_db(-6.0);

        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.1).sin() * 1.5).collect();
        let expected: Vec<f32> = input.iter().map(|&s| limiter1.process(s)).collect();

        let mut block = input.clone();
        limiter2.process_block(&mut block);

        for (i, (&exp, &got)) in expected.iter().zip(block.iter()).enumerate() {
            assert!((exp - got).abs() < 1e-6, "mismatch at {i}: {exp} vs {got}");
        }
    }

    #[test]
    fn stereo_block_matches_per_pair() {
        let mut limiter1 = Limiter::new(44100.0);
        let mut limiter2 = Limiter::new(44100.0);
        limiter1.set_threshold_db(-3.0);
        limiter2.set_threshold_db(-3.0);

        let mut interleaved: Vec<f32> = Vec::with_capacity(512);
        for i in 0..256 {
            let left = (i as f32 * 0.1).sin() * 1.2;
            let right = (i as f32 * 0.15).sin() * 1.3;
            interleaved.push(left);
            interleaved.push(right);
        }

        let mut expected = Vec::with_capacity(512);
        for pair in interleaved.chunks(2) {
            let (l, r) = limiter1.process_stereo(pair[0], pair[1]);
            expected.push(l);
            expected.push(r);
        }

        let mut block = interleaved.clone();
        limiter2.process_block_stereo_interleaved(&mut block);

        for (i, (&exp, &got)) in expected.iter().zip(block.iter()).enumerate() {
            assert!((exp - got).abs() < 1e-6, "mismatch at {i}: {exp} vs {got}");
        }
    }
}
