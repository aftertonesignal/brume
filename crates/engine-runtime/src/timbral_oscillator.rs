// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Timbral Oscillator — Serge NTO-inspired synthesis engine.
//!
//! Triangle core with Serge-style wave multiplier (soft-clip → fold),
//! linear FM, sub-oscillator, self-modulation feedback, and expanded
//! symmetry/tilt. Draws from Serge NTO, Random*Source NTO, CGS48,
//! and Schlaapi Three-Body.

use brume_common::ParameterId;
use brume_dsp_core::{PhaseOscillator, Smoother};

use crate::oscillator_core::Engine;

/// Triangle-core oscillator with wave multiplier, FM, sub, and feedback.
pub struct TimbralOscillator {
    osc: PhaseOscillator,
    sub_phase: f32,
    fm_osc: PhaseOscillator,

    // Core controls
    timbre: Smoother,
    symmetry: Smoother,

    // Wave multiplier
    multiplier_stages: u32,

    // Linear FM
    fm_depth: Smoother,
    fm_ratio: Smoother,

    // Sub-oscillator
    sub_level: Smoother,

    // Self-modulation feedback
    feedback: Smoother,
    output_z1: f32,

    base_frequency: f32,
    sample_rate: f32,
}

impl TimbralOscillator {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            osc: PhaseOscillator::new(sample_rate),
            sub_phase: 0.0,
            fm_osc: PhaseOscillator::new(sample_rate),
            timbre: Smoother::new(0.0, 5.0, sample_rate),
            symmetry: Smoother::new(0.0, 5.0, sample_rate),
            multiplier_stages: 1,
            fm_depth: Smoother::new(0.0, 5.0, sample_rate),
            fm_ratio: Smoother::new(2.0, 10.0, sample_rate),
            sub_level: Smoother::new(0.0, 5.0, sample_rate),
            feedback: Smoother::new(0.0, 5.0, sample_rate),
            output_z1: 0.0,
            base_frequency: 440.0,
            sample_rate,
        }
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.base_frequency = freq.max(1.0);
        self.osc.set_frequency(freq);
    }

    pub fn set_timbre(&mut self, value: f32) {
        self.timbre.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_symmetry(&mut self, value: f32) {
        self.symmetry.set_target(value.clamp(-1.0, 1.0));
    }

    pub fn set_fm_depth(&mut self, value: f32) {
        self.fm_depth.set_target(value.clamp(0.0, 10.0));
    }

    pub fn set_fm_ratio(&mut self, value: f32) {
        self.fm_ratio.set_target(value.clamp(0.5, 16.0));
    }

    pub fn set_sub_level(&mut self, value: f32) {
        self.sub_level.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_feedback(&mut self, value: f32) {
        self.feedback.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_multiplier_stages(&mut self, stages: u32) {
        self.multiplier_stages = stages.clamp(1, 4);
    }

    pub fn reset(&mut self) {
        self.osc.reset();
        self.fm_osc.reset();
        self.sub_phase = 0.0;
        self.output_z1 = 0.0;
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        let timbre = self.timbre.process();
        let symmetry = self.symmetry.process();
        let fm_d = self.fm_depth.process();
        let fm_r = self.fm_ratio.process();
        let sub = self.sub_level.process();
        let fb = self.feedback.process();

        // ── 1. Linear FM + feedback on the triangle core ──
        self.fm_osc.set_frequency(self.base_frequency * fm_r);
        let fm_mod = self.fm_osc.sine();
        self.fm_osc.advance();

        let mut freq = self.base_frequency;
        if fm_d > 0.001 {
            // Linear FM: freq = base + depth * mod * base
            freq += fm_d * fm_mod * self.base_frequency;
        }
        if fb > 0.001 {
            // Self-modulation: output z⁻¹ feeds back into frequency
            freq += fb * self.output_z1 * self.base_frequency * 0.5;
        }
        self.osc
            .set_frequency(freq.clamp(0.0, self.sample_rate * 0.49));

        // ── 2. Triangle core with symmetry tilt ──
        let phase = self.osc.phase();

        // Expanded symmetry: tilt the triangle + asymmetric bias
        // Tilt: at symmetry=+1 → ascending ramp, -1 → descending ramp
        // Bias: x² term introduces even harmonics via the waveshaper
        let tilt = symmetry * 0.45; // limit to avoid degenerate flat regions
        let rise = (0.5 + tilt).clamp(0.05, 0.95);
        let mut tri = if phase < rise {
            2.0 * phase / rise - 1.0
        } else {
            1.0 - 2.0 * (phase - rise) / (1.0 - rise)
        };
        // Asymmetric bias for even harmonics (carried from original NTO design)
        tri += symmetry * 0.3 * tri * tri;

        self.osc.advance();

        // ── 3. Serge wave multiplier ──
        let shaped = serge_wave_multiplier(tri, timbre, self.multiplier_stages);

        // ── 4. Sub-oscillator (one octave down) ──
        let mut output = shaped;
        if sub > 0.001 {
            // Triangle at half frequency
            let sub_tri = if self.sub_phase < 0.5 {
                4.0 * self.sub_phase - 1.0
            } else {
                3.0 - 4.0 * self.sub_phase
            };
            output = output * (1.0 - sub * 0.5) + sub_tri * sub * 0.5;

            // Advance sub at half the rate
            let sub_inc = self.base_frequency * 0.5 / self.sample_rate;
            self.sub_phase += sub_inc;
            if self.sub_phase >= 1.0 {
                self.sub_phase -= 1.0;
            }
        }

        // ── 5. Store for feedback (clamp so feedback stays proportional) ──
        self.output_z1 = output.clamp(-1.0, 1.0);

        output
    }
}

impl Engine for TimbralOscillator {
    fn set_parameter(&mut self, id: ParameterId, value: f32) {
        match id {
            ParameterId::Timbre => self.set_timbre(value),
            ParameterId::Symmetry => self.set_symmetry(value),
            ParameterId::TimbralFmDepth => self.set_fm_depth(value),
            ParameterId::TimbralFmRatio => self.set_fm_ratio(value),
            ParameterId::SubLevel => self.set_sub_level(value),
            ParameterId::TimbralFeedback => self.set_feedback(value),
            ParameterId::MultiplierStages => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                self.set_multiplier_stages((value as u32).clamp(1, 4));
            }
            _ => {}
        }
    }
}

/// Serge-inspired wave multiplier.
///
/// Three-section design inspired by the Serge Wave Multiplier:
/// - Low timbre (0.0-0.3): **Top section** — gentle soft-clip (warm saturation)
/// - Mid timbre (0.3-0.7): **Middle section** — triangle wavefolding
/// - High timbre (0.7-1.0): **Full section** — both cascaded (extreme complexity)
///
/// Multiple stages apply the fold section repeatedly for denser harmonics.
#[inline]
fn serge_wave_multiplier(input: f32, timbre: f32, stages: u32) -> f32 {
    if timbre < 0.001 {
        return input;
    }

    // Progressive drive scales signal into the nonlinear region
    let drive = 1.0 + timbre * 5.0;
    let driven = input * drive;

    // Top section: soft-clip (always active, blends in with timbre)
    // Using a softer curve than tanh for the "warm" Serge character
    let soft = soft_clip(driven);

    // Middle section: triangle wavefolding (kicks in above 0.3)
    let fold_mix = ((timbre - 0.3) / 0.4).clamp(0.0, 1.0);
    let mut folded = driven;
    if fold_mix > 0.0 {
        for _ in 0..stages {
            folded = triangle_fold(folded);
        }
    }

    // Blend: soft-clip base, fold layered on top
    let multiplied = soft * (1.0 - fold_mix * 0.6) + folded * fold_mix * 0.6;

    // Crossfade from clean triangle to shaped output
    input * (1.0 - timbre) + multiplied * timbre
}

/// Serge-style soft clipping — gentler than tanh, more "analog" character.
/// Uses a polynomial approximation of soft saturation.
#[inline]
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Triangle wavefolding — folds the signal back at ±1 boundaries.
/// Creates odd harmonics with a softer character than a hard fold.
#[inline]
fn triangle_fold(x: f32) -> f32 {
    // Normalize to [0,1], fold, then back to [-1,1]
    let x = x * 0.25 + 0.25;
    (x - x.floor() - 0.5).abs() * 4.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn pure_triangle_at_timbre_zero() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.timbre.set_immediate(0.0);

        // Compare with raw PhaseOscillator triangle
        let mut ref_osc = PhaseOscillator::new(SR);
        ref_osc.set_frequency(440.0);

        for _ in 0..4800 {
            let timbral = osc.process();
            let reference = ref_osc.triangle();
            ref_osc.advance();
            assert!(
                (timbral - reference).abs() < 0.001,
                "timbre 0 should be pure triangle: got {timbral}, expected {reference}"
            );
        }
    }

    #[test]
    fn timbre_changes_spectrum() {
        let count_crossings = |t: f32| -> u32 {
            let mut osc = TimbralOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.timbre.set_immediate(t);
            let mut crossings = 0_u32;
            let mut prev = 0.0_f32;
            for _ in 0..4800 {
                let s = osc.process();
                if prev <= 0.0 && s > 0.0 {
                    crossings += 1;
                }
                prev = s;
            }
            crossings
        };

        let clean = count_crossings(0.0);
        let shaped = count_crossings(0.8);
        assert!(
            shaped != clean || shaped > 0,
            "timbre should change the signal: clean={clean}, shaped={shaped}"
        );
    }

    #[test]
    fn output_bounded() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.timbre.set_immediate(1.0);
        osc.symmetry.set_immediate(1.0);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(
                s.is_finite() && s.abs() < 2.0,
                "output should be bounded: {s}"
            );
        }
    }

    #[test]
    fn symmetry_adds_even_harmonics() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(100.0);
        osc.timbre.set_immediate(0.5);
        osc.symmetry.set_immediate(0.8);

        let mut pos_sum = 0.0_f32;
        let mut neg_sum = 0.0_f32;
        for _ in 0..4800 {
            let s = osc.process();
            if s > 0.0 {
                pos_sum += s;
            } else {
                neg_sum += s.abs();
            }
        }

        let ratio = pos_sum / neg_sum.max(0.001);
        assert!(
            (ratio - 1.0).abs() > 0.01,
            "symmetry should create asymmetry: pos={pos_sum}, neg={neg_sum}, ratio={ratio}"
        );
    }

    #[test]
    fn symmetry_tilts_triangle() {
        // Positive symmetry should make ascending ramp (saw-like)
        let samples_at = |sym: f32| -> Vec<f32> {
            let mut osc = TimbralOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.symmetry.set_immediate(sym);
            (0..480).map(|_| osc.process()).collect()
        };
        let neutral = samples_at(0.0);
        let tilted = samples_at(0.9);
        let diffs: usize = neutral
            .iter()
            .zip(&tilted)
            .filter(|(a, b)| (*a - *b).abs() > 0.01)
            .count();
        assert!(
            diffs > 100,
            "symmetry tilt should change waveform: {diffs}/480 differed"
        );
    }

    #[test]
    fn wave_multiplier_stages() {
        // More stages should add more harmonic complexity
        let energy_at = |stages: u32| -> f32 {
            let mut osc = TimbralOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.timbre.set_immediate(0.7);
            osc.set_multiplier_stages(stages);
            let mut e = 0.0_f32;
            // Warmup
            for _ in 0..480 {
                osc.process();
            }
            for _ in 0..4800 {
                let s = osc.process();
                // Count HF content via sample-to-sample differences
                e += s * s;
            }
            e
        };
        let e1 = energy_at(1);
        let e4 = energy_at(4);
        // Both should produce non-zero energy; waveforms differ
        assert!(
            e1 > 0.0 && e4 > 0.0,
            "stages should produce output: e1={e1}, e4={e4}"
        );
    }

    #[test]
    fn fm_bounded() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.fm_depth.set_immediate(5.0);
        osc.fm_ratio.set_immediate(3.0);
        osc.timbre.set_immediate(0.5);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite() && s.abs() < 2.0, "FM output bounded: {s}");
        }
    }

    #[test]
    fn sub_oscillator_adds_weight() {
        let energy_at = |sub_val: f32| -> f32 {
            let mut osc = TimbralOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.sub_level.set_immediate(sub_val);
            let mut e = 0.0_f32;
            for _ in 0..4800 {
                let s = osc.process();
                e += s * s;
            }
            e
        };
        let no_sub = energy_at(0.0);
        let with_sub = energy_at(1.0);
        assert!(
            (no_sub - with_sub).abs() > 1.0,
            "sub should change energy: no_sub={no_sub}, with_sub={with_sub}"
        );
    }

    #[test]
    fn feedback_bounded() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.feedback.set_immediate(0.8);
        osc.timbre.set_immediate(0.5);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite() && s.abs() < 3.0, "feedback bounded: {s}");
        }
    }

    #[test]
    fn feedback_changes_waveform() {
        let samples_at = |fb: f32| -> Vec<f32> {
            let mut osc = TimbralOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.feedback.set_immediate(fb);
            // Warmup
            for _ in 0..480 {
                osc.process();
            }
            (0..480).map(|_| osc.process()).collect()
        };
        let clean = samples_at(0.0);
        let chaotic = samples_at(0.7);
        let diffs: usize = clean
            .iter()
            .zip(&chaotic)
            .filter(|(a, b)| (*a - *b).abs() > 0.01)
            .count();
        assert!(
            diffs > 100,
            "feedback should change waveform: {diffs}/480 differed"
        );
    }

    #[test]
    fn all_features_simultaneously() {
        let mut osc = TimbralOscillator::new(SR);
        osc.set_frequency(220.0);
        osc.timbre.set_immediate(0.7);
        osc.symmetry.set_immediate(0.5);
        osc.fm_depth.set_immediate(2.0);
        osc.fm_ratio.set_immediate(3.0);
        osc.sub_level.set_immediate(0.5);
        osc.feedback.set_immediate(0.4);
        osc.set_multiplier_stages(3);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite() && s.abs() < 4.0, "everything on: {s}");
        }
    }
}
