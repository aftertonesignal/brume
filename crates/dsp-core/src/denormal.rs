// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Subnormal flushing for filter state values.
//!
//! On ARMv8 (Cortex-A76, the CM5's core) and most x86 cores, arithmetic
//! on subnormal floats is significantly slower than on normal floats —
//! tens of cycles instead of single-digit cycles, depending on the
//! microarchitecture. The hardware fix is to set the FPCR `FZ` bit
//! (or MXCSR `FTZ`+`DAZ` on x86), but doing that requires `unsafe` and
//! the workspace forbids it.
//!
//! Filter state values that decay asymptotically toward zero — a
//! comb filter's feedback after a reverb tail, an SVF integrator with
//! the input gone silent, a DC blocker's output history under a long
//! silent run — eventually drift into the subnormal range and stay
//! there forever, paying the slow-path tax on every subsequent
//! sample. They never quite reach true zero on their own.
//!
//! [`flush_subnormal`] tests the exponent bits via `f32::to_bits` and
//! returns `0.0` when the input is subnormal (exponent bits all
//! zero, including true zero). Branch-predictable: the normal-value
//! path is one bit-AND, one compare, one branch — trivial alongside
//! the per-sample filter math, and the only path the predictor sees
//! during steady-state audio. Subnormals only happen during silent
//! tails, where the cost saving from the flush dwarfs the branch
//! overhead.

/// Returns `0.0` for subnormal (and zero) inputs, the input unchanged
/// otherwise. Used to terminate filter-state decay before it slips
/// into the IEEE 754 subnormal range and triggers slow-path FPU
/// handling on every subsequent sample.
#[inline]
#[must_use]
pub fn flush_subnormal(x: f32) -> f32 {
    // The IEEE 754 single-precision exponent occupies bits 23..30.
    // Normal floats have a non-zero exponent. Subnormals have
    // exponent = 0 and a non-zero mantissa; true zero has exponent
    // = 0 and mantissa = 0. Both subnormal and zero satisfy
    // `bits & 0x7F80_0000 == 0`, so this single test catches both —
    // and "flushing" true zero to zero is a no-op anyway.
    //
    // Inf and NaN have exponent = 0xFF and never satisfy the test.
    if x.to_bits() & 0x7F80_0000 == 0 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_normal_values_through() {
        // -3.14 was here originally; clippy's `approx_constant` lint
        // (gated under -D clippy::correctness in CI) flags any literal
        // close to f32::consts::PI even when the value is just an
        // arbitrary normal float for the test. -3.7 is structurally
        // identical for what this test cares about (a negative normal
        // float in the middle range) and doesn't trip the lint.
        for v in &[
            1.0_f32,
            -1.0,
            0.5,
            -3.7,
            1.0e-30,
            -1.0e-30,
            1.0e30,
            f32::MIN_POSITIVE,
        ] {
            assert_eq!(
                flush_subnormal(*v),
                *v,
                "normal value {v} should pass through unchanged"
            );
        }
    }

    #[test]
    fn flushes_subnormal_to_zero() {
        // f32 subnormal range is 1e-45 to ~1.18e-38. Build a few via
        // bit patterns to be sure we land inside it.
        let smallest_subnormal = f32::from_bits(0x0000_0001);
        let mid_subnormal = f32::from_bits(0x0040_0000);
        let largest_subnormal = f32::from_bits(0x007F_FFFF);
        for v in &[smallest_subnormal, mid_subnormal, largest_subnormal] {
            assert!(!v.is_normal(), "test setup: {v} should be subnormal");
            assert_eq!(
                flush_subnormal(*v),
                0.0,
                "subnormal {v:e} should flush to zero"
            );
        }
        // Negative subnormals too.
        let neg = -f32::from_bits(0x0040_0000);
        assert_eq!(flush_subnormal(neg), 0.0);
    }

    #[test]
    fn passes_zero_through() {
        assert_eq!(flush_subnormal(0.0), 0.0);
        assert_eq!(flush_subnormal(-0.0), 0.0);
    }

    #[test]
    fn passes_inf_and_nan_through() {
        // Inf and NaN have non-zero exponent bits (0xFF), so the test
        // returns false and the value passes through. The filter code
        // will deal with them through its own clamping; flush_subnormal
        // doesn't try to be a numerical-hygiene catch-all.
        assert_eq!(flush_subnormal(f32::INFINITY), f32::INFINITY);
        assert_eq!(flush_subnormal(f32::NEG_INFINITY), f32::NEG_INFINITY);
        assert!(flush_subnormal(f32::NAN).is_nan());
    }
}
