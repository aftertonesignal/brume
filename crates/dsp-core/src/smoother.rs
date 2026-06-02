// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! One-pole parameter smoother for zipper-noise-free control changes.

use crate::utility::one_pole_coeff;

/// Smooths parameter changes over time using a one-pole filter.
///
/// Prevents audible zipper noise when parameters are updated from
/// the control thread. Each destination parameter should have its
/// own `Smoother` instance.
///
/// The glide rate can differ by direction — see
/// [`Smoother::new_asymmetric`] — for controls that must react fast
/// one way and slow the other.
pub struct Smoother {
    current: f32,
    target: f32,
    /// Coefficient applied while gliding **up** toward a higher target
    /// (`current < target`).
    rise_coeff: f32,
    /// Coefficient applied while gliding **down** toward a lower target
    /// (`current > target`). Equal to `rise_coeff` for a symmetric
    /// smoother built with [`Smoother::new`].
    fall_coeff: f32,
}

impl Smoother {
    /// Creates a new smoother.
    ///
    /// - `initial`: starting value (both current and target)
    /// - `time_ms`: smoothing time constant in milliseconds
    /// - `sample_rate`: audio sample rate in Hz
    #[must_use]
    pub fn new(initial: f32, time_ms: f32, sample_rate: f32) -> Self {
        let coeff = one_pole_coeff(time_ms, sample_rate);
        Self {
            current: initial,
            target: initial,
            rise_coeff: coeff,
            fall_coeff: coeff,
        }
    }

    /// Creates a smoother with direction-dependent time constants.
    ///
    /// - `rise_ms`: smoothing time while gliding **up** toward a higher
    ///   target (`current < target`)
    /// - `fall_ms`: smoothing time while gliding **down** toward a lower
    ///   target (`current > target`)
    ///
    /// Use this when a control should react at different rates in each
    /// direction — e.g. a mix-headroom tracker that must duck fast on a
    /// chord onset (so the summed attack transient can't overshoot a
    /// downstream saturator) yet restore slowly as voices release (so
    /// the step back up doesn't read as a ghost re-trigger).
    #[must_use]
    pub fn new_asymmetric(initial: f32, rise_ms: f32, fall_ms: f32, sample_rate: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            rise_coeff: one_pole_coeff(rise_ms, sample_rate),
            fall_coeff: one_pole_coeff(fall_ms, sample_rate),
        }
    }

    /// Sets a new target value. The smoother will glide toward it.
    pub fn set_target(&mut self, value: f32) {
        self.target = value;
    }

    /// Advances one sample and returns the smoothed value.
    #[inline]
    pub fn process(&mut self) -> f32 {
        // Pick the coefficient by travel direction so asymmetric
        // smoothers glide at different rates up vs. down. Symmetric
        // smoothers set both coefficients equal, so the branch is a
        // no-op for them.
        let coeff = if self.current > self.target {
            self.fall_coeff
        } else {
            self.rise_coeff
        };
        self.current = self.target + coeff * (self.current - self.target);
        self.current
    }

    /// Jumps immediately to a value (bypasses smoothing).
    pub fn set_immediate(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }

    /// Returns `true` when the smoother has essentially reached its target.
    #[must_use]
    pub fn is_settled(&self, epsilon: f32) -> bool {
        (self.current - self.target).abs() < epsilon
    }

    /// Returns the current smoothed value without advancing.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.current
    }

    /// Updates the smoothing time constant.
    pub fn set_time(&mut self, time_ms: f32, sample_rate: f32) {
        let coeff = one_pole_coeff(time_ms, sample_rate);
        self.rise_coeff = coeff;
        self.fall_coeff = coeff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_converges() {
        let mut s = Smoother::new(0.0, 5.0, 48000.0);
        s.set_target(1.0);

        // One-pole needs ~5x the time constant to settle within 1%.
        // 5ms * 5 = 25ms = 1200 samples at 48kHz.
        for _ in 0..2400 {
            s.process();
        }

        assert!(
            s.is_settled(0.001),
            "smoother should converge: current={}, target={}",
            s.value(),
            1.0
        );
    }

    #[test]
    fn set_immediate_jumps() {
        let mut s = Smoother::new(0.0, 10.0, 48000.0);
        s.set_immediate(0.75);
        assert!((s.value() - 0.75).abs() < f32::EPSILON);
        assert!((s.process() - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn asymmetric_falls_fast_rises_slow() {
        let sr = 48_000.0;
        // Slow rise (30 ms), fast fall (2 ms) — the headroom profile.
        let mut s = Smoother::new_asymmetric(1.0, 30.0, 2.0, sr);

        // Duck: the 2 ms fall settles close to target within ~12 ms.
        s.set_target(0.4);
        for _ in 0..(sr * 0.012) as usize {
            s.process();
        }
        assert!(
            s.is_settled(0.01),
            "fast fall should reach target quickly: {}",
            s.value()
        );

        // Restore: the same elapsed time on the slow 30 ms rise must
        // still lag well short of the target — that gap is the asymmetry.
        s.set_target(1.0);
        for _ in 0..(sr * 0.012) as usize {
            s.process();
        }
        assert!(
            s.value() < 0.8,
            "slow rise should lag well behind target: {}",
            s.value()
        );
    }
}
