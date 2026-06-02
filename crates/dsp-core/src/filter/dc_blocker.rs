// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! DC-blocking filter.

/// Removes DC offset from audio signals via a highpass with a very low
/// cutoff (~5 Hz) — minimally affects audible content.
///
/// # Example
///
/// ```
/// use brume_dsp_core::filter::DcBlocker;
///
/// let mut blocker = DcBlocker::new(44100.0);
/// let output = blocker.process(1.5); // DC offset of 0.5
/// ```
pub struct DcBlocker {
    coefficient: f32,
    x_prev: f32,
    y_prev: f32,
}

impl DcBlocker {
    /// Creates a new DC blocker for the given sample rate. Cutoff ≈ 5 Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let coefficient = 1.0 - (5.0 * 2.0 * std::f32::consts::PI / sample_rate);
        Self {
            coefficient: coefficient.clamp(0.9, 0.9999),
            x_prev: 0.0,
            y_prev: 0.0,
        }
    }

    /// Sets the sample rate and recalculates the coefficient.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let coefficient = 1.0 - (5.0 * 2.0 * std::f32::consts::PI / sample_rate);
        self.coefficient = coefficient.clamp(0.9, 0.9999);
    }

    /// Resets the filter state.
    pub fn reset(&mut self) {
        self.x_prev = 0.0;
        self.y_prev = 0.0;
    }

    /// Processes a single sample, removing DC offset.
    /// `y[n] = x[n] - x[n-1] + R * y[n-1]`
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = input - self.x_prev + self.coefficient * self.y_prev;
        self.x_prev = input;
        self.y_prev = output;
        output
    }
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_dc() {
        let mut blocker = DcBlocker::new(44100.0);
        for _ in 0..44100 {
            blocker.process(1.0);
        }
        let output = blocker.process(1.0);
        assert!(output.abs() < 0.01);
    }

    #[test]
    fn passes_ac() {
        let mut blocker = DcBlocker::new(44100.0);
        for i in 0..44100 {
            let input = (i as f32 * 0.1).sin();
            blocker.process(input);
        }
        let mut max_output = 0.0_f32;
        for i in 0..1000 {
            let input = (i as f32 * 0.1).sin();
            let output = blocker.process(input);
            max_output = max_output.max(output.abs());
        }
        assert!(max_output > 0.5);
    }

    #[test]
    fn reset_clears_state() {
        let mut blocker = DcBlocker::new(44100.0);
        blocker.process(1.0);
        blocker.process(1.0);
        blocker.reset();
        assert_eq!(blocker.x_prev, 0.0);
        assert_eq!(blocker.y_prev, 0.0);
    }
}
