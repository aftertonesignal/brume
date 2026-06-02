// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Audio-rate phase accumulator with separated advance and waveform output.
//!
//! `PhaseOscillator` separates phase advance from waveform evaluation,
//! so FM synthesis can read one oscillator's output before computing
//! the other's.

use std::f32::consts::TAU;

/// Audio-rate oscillator with sine and triangle output and phase modulation.
pub struct PhaseOscillator {
    phase: f32,
    phase_increment: f32,
    frequency: f32,
    sample_rate: f32,
}

impl PhaseOscillator {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut osc = Self {
            phase: 0.0,
            phase_increment: 0.0,
            frequency: 440.0,
            sample_rate,
        };
        osc.update_increment();
        osc
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.frequency = freq.max(0.0);
        self.update_increment();
    }

    /// Sets a signed frequency — phase advances backwards when negative.
    /// Needed for through-zero FM, where the modulator drives the
    /// carrier's effective frequency below zero and the defining
    /// sonic character is the carrier playing backwards through the
    /// negative lobe of the modulation. Regular `set_frequency` clamps
    /// to >= 0 as a safety net for pitch-tracking callers who never
    /// want backwards playback; TZFM bypasses that clamp.
    pub fn set_frequency_signed(&mut self, freq: f32) {
        self.frequency = freq;
        self.update_increment();
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update_increment();
    }

    #[must_use]
    pub fn frequency(&self) -> f32 {
        self.frequency
    }

    /// Advances the phase accumulator by one sample. Call once per sample,
    /// after reading the waveform output. Wraps in both directions so
    /// negative phase increments (through-zero FM) are handled.
    #[inline]
    pub fn advance(&mut self) {
        self.phase += self.phase_increment;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        } else if self.phase < 0.0 {
            self.phase += 1.0;
        }
    }

    /// Sine output at the current phase.
    #[inline]
    #[must_use]
    pub fn sine(&self) -> f32 {
        (self.phase * TAU).sin()
    }

    /// Sine output with phase modulation (offset in radians).
    #[inline]
    #[must_use]
    pub fn sine_pm(&self, pm_radians: f32) -> f32 {
        (self.phase * TAU + pm_radians).sin()
    }

    /// Triangle output at the current phase. Range: [-1.0, 1.0].
    #[inline]
    #[must_use]
    pub fn triangle(&self) -> f32 {
        triangle_wave(self.phase)
    }

    /// Triangle output with phase modulation (offset in radians).
    #[inline]
    #[must_use]
    pub fn triangle_pm(&self, pm_radians: f32) -> f32 {
        // Normalize the PM offset from radians to [0, 1) phase
        let p = (self.phase + pm_radians / TAU).rem_euclid(1.0);
        triangle_wave(p)
    }

    /// Saw output at the current phase. Range: [-1.0, 1.0].
    #[inline]
    #[must_use]
    pub fn saw(&self) -> f32 {
        2.0 * self.phase - 1.0
    }

    /// Saw output with phase modulation.
    #[inline]
    #[must_use]
    pub fn saw_pm(&self, pm_radians: f32) -> f32 {
        let p = (self.phase + pm_radians / TAU).rem_euclid(1.0);
        2.0 * p - 1.0
    }

    /// Square/pulse output at the current phase. Range: [-1.0, 1.0].
    /// `pw` is pulse width (0.5 = square, 0.1-0.9 for pulse).
    #[inline]
    #[must_use]
    pub fn square_pm(&self, pm_radians: f32, pw: f32) -> f32 {
        let p = (self.phase + pm_radians / TAU).rem_euclid(1.0);
        if p < pw { 1.0 } else { -1.0 }
    }

    /// Returns the current phase (0.0 to 1.0). Useful for sync detection.
    #[inline]
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Sets the phase to a specific value in [0, 1). Used for grain start offsets.
    #[inline]
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// Resets the phase to zero. Used for hard sync.
    #[inline]
    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn update_increment(&mut self) {
        self.phase_increment = self.frequency / self.sample_rate;
    }
}

/// Piecewise linear triangle wave from normalized phase [0, 1).
#[inline]
fn triangle_wave(phase: f32) -> f32 {
    if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    }
}

/// Continuous morph between sine, triangle, saw, and square.
///
/// `phase` is 0.0-1.0 (oscillator phase). `pm` is phase modulation in radians.
/// `morph` is 0.0-1.0: 0=sine, 0.33=triangle, 0.66=saw, 1.0=square.
#[inline]
pub fn morph_wave(phase: f32, pm: f32, morph: f32) -> f32 {
    let p = (phase + pm / TAU).rem_euclid(1.0);

    if morph <= 0.0 {
        // Pure sine
        (p * TAU).sin()
    } else if morph <= 0.333 {
        // Sine → Triangle
        let t = morph / 0.333;
        let sine = (p * TAU).sin();
        let tri = triangle_wave(p);
        sine * (1.0 - t) + tri * t
    } else if morph <= 0.666 {
        // Triangle → Saw
        let t = (morph - 0.333) / 0.333;
        let tri = triangle_wave(p);
        let saw = 2.0 * p - 1.0;
        tri * (1.0 - t) + saw * t
    } else {
        // Saw → Square
        let t = (morph - 0.666) / 0.334;
        let saw = 2.0 * p - 1.0;
        let square = if p < 0.5 { 1.0 } else { -1.0 };
        saw * (1.0 - t) + square * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_output_range() {
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency(440.0);
        for _ in 0..48000 {
            let s = osc.sine();
            assert!((-1.0..=1.0).contains(&s), "sine out of range: {s}");
            osc.advance();
        }
    }

    #[test]
    fn triangle_output_range() {
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency(440.0);
        for _ in 0..48000 {
            let s = osc.triangle();
            assert!((-1.0..=1.0).contains(&s), "triangle out of range: {s}");
            osc.advance();
        }
    }

    #[test]
    fn triangle_pm_output_range() {
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency(440.0);
        // Sweep PM across a wide range
        for i in 0..48000 {
            let pm = (i as f32 / 48000.0) * TAU * 5.0 - TAU * 2.5;
            let s = osc.triangle_pm(pm);
            assert!(
                (-1.0..=1.0).contains(&s),
                "triangle_pm out of range: {s} at pm={pm}"
            );
            osc.advance();
        }
    }

    #[test]
    fn signed_frequency_runs_phase_backwards() {
        // set_frequency_signed with a negative value must advance the
        // phase backwards — required for through-zero FM. The naive
        // set_frequency clamps to 0, so this behavior only activates
        // via the signed setter. Output should still be in [-1, 1] and
        // the oscillator should count the same number of cycles
        // regardless of sign.
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency_signed(-100.0);
        assert!(osc.frequency() < 0.0, "signed setter should preserve sign");

        // Count negative-going zero crossings for 1 second at -100 Hz.
        // A backwards-running sine still has ~100 crossings per second;
        // they're just in the opposite direction from positive-freq.
        let mut crossings = 0_u32;
        let mut prev = osc.sine();
        osc.advance();
        for _ in 0..48000 {
            let s = osc.sine();
            if prev <= 0.0 && s > 0.0 {
                crossings += 1;
            }
            prev = s;
            osc.advance();
        }
        assert!(
            (99..=101).contains(&crossings),
            "negative 100 Hz should still give ~100 crossings/sec; got {crossings}"
        );
    }

    #[test]
    fn phase_stays_in_unit_interval_when_freq_is_negative() {
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency_signed(-440.0);
        for _ in 0..48000 {
            let p = osc.phase();
            assert!(
                (0.0..1.0).contains(&p),
                "phase escaped [0, 1) with negative freq: {p}"
            );
            osc.advance();
        }
    }

    #[test]
    fn set_frequency_still_clamps_for_safety() {
        // The unsigned setter keeps its >= 0 clamp so pitch-tracking
        // callers (note_on wiring, MIDI handlers) never accidentally
        // enable backwards playback. TZFM opts in via the _signed setter.
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency(-500.0);
        assert_eq!(osc.frequency(), 0.0, "set_frequency must still clamp");
    }

    #[test]
    fn pm_does_not_cause_pitch_drift() {
        // After applying PM for a while, the unmodulated pitch should
        // still be correct — PM displaces phase momentarily without
        // accumulating drift (unlike true FM which modifies frequency).
        let mut osc = PhaseOscillator::new(48000.0);
        osc.set_frequency(100.0);

        // Run with heavy PM for 1 second
        for i in 0..48000 {
            let pm = (i as f32 * 0.01).sin() * TAU * 5.0;
            let _ = osc.sine_pm(pm);
            osc.advance();
        }

        // Now count zero crossings without PM for 1 second
        let mut crossings = 0_u32;
        let mut prev = osc.sine();
        osc.advance();
        for _ in 0..48000 {
            let s = osc.sine(); // no PM
            if prev <= 0.0 && s > 0.0 {
                crossings += 1;
            }
            prev = s;
            osc.advance();
        }
        // At 100 Hz for 1 second, expect ~100 positive-going zero crossings
        assert!(
            (99..=101).contains(&crossings),
            "expected ~100 crossings after PM, got {crossings} — pitch drifted"
        );
    }
}
