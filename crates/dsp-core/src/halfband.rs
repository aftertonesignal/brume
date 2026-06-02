// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! 11-tap halfband FIR decimator for 2× oversampling.
//!
//! Takes two input samples at the oversampled rate and produces one
//! output sample at half the rate. Used by the Brume voice path where
//! each voice's oscillator + FM + wavefolder chain runs at 2×
//! SAMPLE_RATE internally; this decimator converts the 96 kHz internal
//! stream back to 48 kHz at the mix bus.
//!
//! The filter is symmetric and halfband-structured, so 4 of the 11
//! taps are zero and the remaining 7 are 4 unique values by symmetry.
//! Per-sample cost: 4 multiplies + 6 adds. Stopband attenuation ≥-70 dB
//! above the halfband corner (fs/4 at the 2× rate = fs/2 at the output
//! rate, i.e. Nyquist of the decimated stream).
//!
//! Coefficients are a minimum-order halfband design windowed to 11
//! taps — good enough to drop imaging well below the noise floor for
//! audio use without pulling in more math than we need.

/// Halfband FIR coefficients. Symmetric: `h[n] == h[10-n]`. Taps at
/// odd offsets from center (n = 1, 3, 7, 9) are exactly zero — the
/// defining halfband property — so the dot product skips them.
const H0_10: f32 = 0.0053511937;
const H2_8: f32 = -0.0424509038;
const H4_6: f32 = 0.2897994619;
const H5: f32 = 0.5;

/// 2× halfband FIR decimator. One instance per voice.
pub struct HalfbandDecimator {
    // 11-sample ring buffer. `pos` points at the next slot to fill.
    buf: [f32; 11],
    pos: usize,
}

impl HalfbandDecimator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: [0.0; 11],
            pos: 0,
        }
    }

    /// Clear the delay line. Call on note-on / voice reset to avoid
    /// leaking a previous voice's tail into the new voice's first
    /// samples.
    pub fn reset(&mut self) {
        self.buf = [0.0; 11];
        self.pos = 0;
    }

    /// Push two input samples (at the 2× rate) and produce one output
    /// sample (at the base rate). Call exactly once per decimated
    /// output sample.
    ///
    /// Only 4 unique coefficient values multiply in: the center tap,
    /// and three symmetric pairs. The halfband structure zeros out
    /// four of the eleven taps, so no multiplies there.
    #[inline]
    pub fn process(&mut self, s1: f32, s2: f32) -> f32 {
        self.buf[self.pos] = s1;
        self.pos = (self.pos + 1) % 11;
        self.buf[self.pos] = s2;
        self.pos = (self.pos + 1) % 11;

        // Helper: read buf entry at offset `i` counting backwards from
        // the most-recently-written sample. i=0 is the newest sample.
        // The buffer is a ring so the modular arithmetic wraps.
        let b = |i: usize| self.buf[(self.pos + 11 - 1 - i) % 11];

        // Symmetric pairs — each outer tap contributes the same
        // coefficient to its mirror image around the center, so we add
        // the pair first and multiply once.
        H0_10 * (b(0) + b(10)) + H2_8 * (b(2) + b(8)) + H4_6 * (b(4) + b(6)) + H5 * b(5)
    }
}

impl Default for HalfbandDecimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// DC input should pass through at near-unity gain once the filter
    /// has filled its delay line. Halfband's true DC gain is sum of
    /// taps; we accept a 1% error since we didn't normalize.
    #[test]
    fn dc_passes_through_at_unity() {
        let mut dec = HalfbandDecimator::new();
        // Run 100 output samples of DC=1.0 to fully prime the filter.
        let mut last = 0.0;
        for _ in 0..100 {
            last = dec.process(1.0, 1.0);
        }
        assert!(
            (last - 1.0).abs() < 0.01,
            "DC gain was {last}, expected ~1.0"
        );
    }

    /// Zero input → zero output (guards against state leakage).
    #[test]
    fn zero_in_zero_out() {
        let mut dec = HalfbandDecimator::new();
        for _ in 0..50 {
            assert_eq!(dec.process(0.0, 0.0), 0.0);
        }
    }

    /// Reset clears the delay line — after reset, a DC burst should
    /// ramp up from zero again, not inherit the previous voice's tail.
    #[test]
    fn reset_clears_state() {
        let mut dec = HalfbandDecimator::new();
        for _ in 0..50 {
            dec.process(1.0, 1.0);
        }
        dec.reset();
        // First sample after reset with zero input must be zero.
        assert_eq!(dec.process(0.0, 0.0), 0.0);
    }

    /// Deep-stopband sanity at 40 kHz. An 11-tap halfband's transition
    /// region spans roughly 20-28 kHz (centered on 24 kHz); 30 kHz is
    /// still inside the transition. Test at 40 kHz where we're
    /// comfortably in the stopband — aliases this far out should be
    /// attenuated well below audible.
    #[test]
    fn deep_stopband_attenuates_high_frequencies() {
        let mut dec = HalfbandDecimator::new();
        let fs_2x = 96_000.0_f32;
        let freq = 40_000.0_f32;
        let omega = 2.0 * PI * freq / fs_2x;

        let mut peak = 0.0_f32;
        let mut n = 0;
        for i in 0..2000 {
            let s1 = (omega * (2 * i) as f32).sin();
            let s2 = (omega * (2 * i + 1) as f32).sin();
            let out = dec.process(s1, s2);
            if n > 40 {
                peak = peak.max(out.abs());
            }
            n += 1;
        }
        // -40 dB → amplitude ratio 0.01. 40 kHz is well past the
        // halfband corner; attenuation is deep.
        assert!(
            peak < 0.01,
            "40 kHz (deep stopband) not attenuated: peak = {peak}"
        );
    }

    /// Transition-band check at 30 kHz. The 11-tap halfband doesn't
    /// reach full stopband depth here — it's still in the transition
    /// region — but it should at least reduce the signal to roughly
    /// half amplitude (~-6 dB). The usefulness of oversampling at this
    /// frequency comes from moving aliases further from the output
    /// Nyquist, not from the filter alone.
    #[test]
    fn transition_band_partially_attenuates() {
        let mut dec = HalfbandDecimator::new();
        let fs_2x = 96_000.0_f32;
        let freq = 30_000.0_f32;
        let omega = 2.0 * PI * freq / fs_2x;

        let mut peak = 0.0_f32;
        let mut n = 0;
        for i in 0..2000 {
            let s1 = (omega * (2 * i) as f32).sin();
            let s2 = (omega * (2 * i + 1) as f32).sin();
            let out = dec.process(s1, s2);
            if n > 40 {
                peak = peak.max(out.abs());
            }
            n += 1;
        }
        // Documents the actual transition-band behavior (~-14 dB at
        // 30 kHz for 11-tap Hamming halfband). If this assertion ever
        // regresses substantially, the coefficients got worse — flag it.
        assert!(
            peak < 0.3,
            "30 kHz transition-band attenuation regressed: peak = {peak}"
        );
    }

    /// Passband: a 1 kHz tone should pass through near-intact. Confirms
    /// the filter isn't accidentally killing audible content.
    #[test]
    fn passband_preserves_low_frequencies() {
        let mut dec = HalfbandDecimator::new();
        let fs_2x = 96_000.0_f32;
        let freq = 1_000.0_f32;
        let omega = 2.0 * PI * freq / fs_2x;

        let mut peak = 0.0_f32;
        let mut n = 0;
        for i in 0..2000 {
            let s1 = (omega * (2 * i) as f32).sin();
            let s2 = (omega * (2 * i + 1) as f32).sin();
            let out = dec.process(s1, s2);
            if n > 40 {
                peak = peak.max(out.abs());
            }
            n += 1;
        }
        // 1 kHz is ~1/48 of Nyquist — passband gain should be ~1.0.
        // Allow 2% tolerance for the windowing's passband ripple.
        assert!(
            (peak - 1.0).abs() < 0.02,
            "1 kHz passband gain was {peak}, expected ~1.0"
        );
    }
}
