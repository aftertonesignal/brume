// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Shared 4096-entry sine lookup table with linear interpolation.
//!
//! `phase.sin()` on a Cortex-A76 (CM5) is roughly 30–40 cycles per call
//! after constant-folding. At Brume's worst case — 8 harmonics × 6
//! voices × one Harmonic part = 48 sins per audio sample, or ~2.3M
//! sins per second at 48 kHz — that's a measurable slice of audio
//! budget for a path that reads as "just play a partial." The LUT
//! lookup is ~5–7 cycles (load + index + interp) and fits in 16 KB,
//! comfortably inside L1.
//!
//! Hosted here in `dsp-core` rather than inside an oscillator crate so
//! the FM and Harmonic engines share one cache-warm copy. The single
//! `OnceLock`-backed table initializes on first call and lives for
//! the life of the process; subsequent calls are a pointer load.

use std::sync::OnceLock;

/// Power of two so `& (SIZE - 1)` replaces a modulo. 4096 entries gives
/// ~0.0015% RMS error vs `f32::sin` after linear interpolation —
/// well below the noise floor of a 16-bit DAC and indistinguishable
/// from `sin` in any audio context that doesn't FFT the result.
const SIZE: usize = 4096;

fn table() -> &'static [f32; SIZE] {
    static TABLE: OnceLock<[f32; SIZE]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0_f32; SIZE];
        for (i, slot) in t.iter_mut().enumerate() {
            let phase = i as f32 / SIZE as f32;
            *slot = (phase * std::f32::consts::TAU).sin();
        }
        t
    })
}

/// Looks up `sin(phase * 2π)` where `phase` is in turns (cycles), not
/// radians. Phase is wrapped to the unit interval, so any value
/// — including negatives or accumulated multi-turn phase — is valid.
///
/// For radians, divide by `TAU` first (or use [`sine_radians`]).
#[inline]
#[must_use]
pub fn sine_normalized(phase: f32) -> f32 {
    let t = table();
    let wrapped = phase - phase.floor();
    let fpos = wrapped * SIZE as f32;
    let i0 = (fpos as usize) & (SIZE - 1);
    let i1 = (i0 + 1) & (SIZE - 1);
    let frac = fpos - fpos.floor();
    t[i0] + frac * (t[i1] - t[i0])
}

/// Looks up `sin(radians)` via the shared LUT. Equivalent to
/// `sine_normalized(radians / TAU)` with one fewer division at
/// the call site.
#[inline]
#[must_use]
pub fn sine_radians(radians: f32) -> f32 {
    sine_normalized(radians * std::f32::consts::FRAC_1_PI * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn matches_sin_within_tolerance() {
        // Linear interpolation of a 4096-entry sine table should track
        // f32::sin to better than 0.001 absolute error across the
        // unit circle. The bound comes from the ~π/2048 spacing
        // between table entries; the derivative of sin (also bounded
        // by 1) sets the worst-case interpolation error at half the
        // sample step.
        const TOLERANCE: f32 = 0.001;
        for i in 0..1000 {
            let phase = i as f32 / 1000.0;
            let lut = sine_normalized(phase);
            let actual = (phase * TAU).sin();
            assert!(
                (lut - actual).abs() < TOLERANCE,
                "phase {phase}: lut {lut} vs actual {actual} (delta {})",
                (lut - actual).abs()
            );
        }
    }

    #[test]
    fn handles_negative_and_multi_turn_phase() {
        // Phase argument is normalized via `phase - phase.floor()` so
        // any value should land on the right table entry. Spot-check
        // the corners.
        let near_zero = sine_normalized(0.0);
        let from_negative = sine_normalized(-1.0);
        let from_double = sine_normalized(2.0);
        assert!((near_zero - from_negative).abs() < 1e-5);
        assert!((near_zero - from_double).abs() < 1e-5);

        let quarter = sine_normalized(0.25);
        let from_neg_three_quarters = sine_normalized(-0.75);
        assert!((quarter - from_neg_three_quarters).abs() < 1e-5);
    }

    #[test]
    fn radians_helper_matches_normalized() {
        for i in 0..100 {
            let phase = i as f32 / 100.0;
            let from_norm = sine_normalized(phase);
            let from_rad = sine_radians(phase * TAU);
            assert!(
                (from_norm - from_rad).abs() < 1e-5,
                "phase {phase}: norm {from_norm} vs rad {from_rad}"
            );
        }
    }
}
