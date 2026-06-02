// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Utility functions for DSP calculations.
//!
//! Common conversion functions and helpers used throughout the engine.

/// Converts a decibel value to linear gain.
#[must_use]
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Converts a linear gain value to decibels. Returns `f32::NEG_INFINITY`
/// for gain ≤ 0.
#[must_use]
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// Converts milliseconds to samples at a given sample rate.
#[must_use]
#[inline]
pub fn ms_to_samples(ms: f32, sample_rate: f32) -> usize {
    (ms * 0.001 * sample_rate).round() as usize
}

/// Converts samples to milliseconds at a given sample rate.
#[must_use]
#[inline]
pub fn samples_to_ms(samples: usize, sample_rate: f32) -> f32 {
    (samples as f32 / sample_rate) * 1000.0
}

/// Calculates the coefficient for a one-pole lowpass filter given a time
/// constant. Returns 0.0 for `time_ms` ≤ 0.0 (instant response).
#[must_use]
#[inline]
pub fn one_pole_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    if time_ms <= 0.0 {
        return 0.0;
    }
    (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
}

/// Linear interpolation between two values.
#[must_use]
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Clamps a value to a specified range.
#[must_use]
#[inline]
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max)
}

/// Fast polynomial approximation of 2^x — roughly 5-10× faster than
/// `2.0_f32.powf(x)` with error <0.2% for |x| < 4. Used on audio-rate
/// pitch and FM paths.
#[must_use]
#[inline]
pub fn fast_exp2(x: f32) -> f32 {
    #[cfg(any(
        all(target_arch = "x86_64", target_feature = "sse2"),
        target_arch = "aarch64"
    ))]
    {
        let xi = x.floor();
        let xf = x - xi;

        // Cubic polynomial (Correia 2020) tuned for minimal max error
        // over [0, 1]; combined with bit-manipulation for the integer
        // part via f32::from_bits.
        let p = 0.079_441_54 * xf + 0.227_411_28;
        let p = p * xf + 0.696_807_86;
        let p = p * xf + 0.999_846_5;

        let xi_i = xi as i32;
        let pow2_xi = f32::from_bits(((127 + xi_i) as u32) << 23);

        p * pow2_xi
    }

    #[cfg(not(any(
        all(target_arch = "x86_64", target_feature = "sse2"),
        target_arch = "aarch64"
    )))]
    {
        2.0_f32.powf(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    #[test]
    fn db_to_gain_round_trips() {
        assert!((db_to_gain(0.0) - 1.0).abs() < EPSILON);
        assert!((db_to_gain(20.0) - 10.0).abs() < EPSILON);
        assert!((db_to_gain(-20.0) - 0.1).abs() < EPSILON);
    }

    #[test]
    fn gain_to_db_round_trips() {
        assert!((gain_to_db(1.0) - 0.0).abs() < EPSILON);
        assert!((gain_to_db(10.0) - 20.0).abs() < EPSILON);
        assert!(gain_to_db(0.0).is_infinite());
    }

    #[test]
    fn ms_samples_convert() {
        assert_eq!(ms_to_samples(1000.0, 44100.0), 44100);
        assert_eq!(ms_to_samples(1.0, 44100.0), 44);
        assert!((samples_to_ms(44100, 44100.0) - 1000.0).abs() < EPSILON);
    }

    #[test]
    fn lerp_endpoints() {
        assert!((lerp(0.0, 10.0, 0.5) - 5.0).abs() < EPSILON);
        assert!((lerp(0.0, 10.0, 0.0) - 0.0).abs() < EPSILON);
        assert!((lerp(0.0, 10.0, 1.0) - 10.0).abs() < EPSILON);
    }

    #[test]
    fn clamp_bounds() {
        assert!((clamp(5.0, 0.0, 10.0) - 5.0).abs() < EPSILON);
        assert!((clamp(-5.0, 0.0, 10.0) - 0.0).abs() < EPSILON);
        assert!((clamp(15.0, 0.0, 10.0) - 10.0).abs() < EPSILON);
    }

    #[test]
    fn fast_exp2_accuracy() {
        let test_values = [
            -4.0, -3.0, -2.0, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0,
        ];
        for &x in &test_values {
            let expected = 2.0_f32.powf(x);
            let actual = fast_exp2(x);
            let error = (actual - expected).abs() / expected;
            assert!(error < 0.005, "fast_exp2({x}) error {:.4}%", error * 100.0);
        }
    }
}
