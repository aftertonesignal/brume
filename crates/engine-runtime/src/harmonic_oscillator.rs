// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Harmonic Oscillator — expanded additive synthesis engine.
//!
//! 8 phase-tracked harmonics with continuous waveform morph, Gaussian
//! scanning window, internal FM modulator, phase spread, spectral tilt,
//! odd/even balance, and inharmonicity. Draws from Verbos Harmonic
//! Oscillator (scanning), Kawai K5 (many harmonics, spectral envelopes),
//! Buchla 296 (spectral processing), and Just Friends (organic curves).

use std::f32::consts::TAU;

use brume_common::ParameterId;
use brume_dsp_core::{PhaseOscillator, Smoother, morph_wave, sine_normalized};

use crate::oscillator_core::Engine;

const NUM_HARMONICS: usize = 8;

/// Additive oscillator with 8 harmonics, scanning, morph, and FM.
pub struct HarmonicOscillator {
    phases: [f32; NUM_HARMONICS],
    levels: [f32; NUM_HARMONICS],
    fundamental: f32,
    sample_rate: f32,

    // Existing controls
    tilt: Smoother,
    odd_even: Smoother,
    inharmonicity: Smoother,

    // New: harmonic scanning (Verbos-inspired)
    scan_center: Smoother,
    scan_width: Smoother,

    // New: per-harmonic waveform morph (sine→tri→saw→square)
    harmonic_morph: Smoother,

    // New: FM on the fundamental
    fm_osc: PhaseOscillator,
    fm_depth: Smoother,
    fm_ratio: Smoother,

    // New: phase spread (random phase offsets between harmonics)
    spread: f32,
    phase_offsets: [f32; NUM_HARMONICS],
}

impl HarmonicOscillator {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phases: [0.0; NUM_HARMONICS],
            levels: [1.0, 0.5, 0.3, 0.25, 0.2, 0.15, 0.1, 0.08],
            fundamental: 440.0,
            sample_rate,
            tilt: Smoother::new(0.0, 5.0, sample_rate),
            odd_even: Smoother::new(0.5, 5.0, sample_rate),
            inharmonicity: Smoother::new(0.0, 5.0, sample_rate),
            scan_center: Smoother::new(0.5, 5.0, sample_rate),
            scan_width: Smoother::new(1.0, 5.0, sample_rate),
            harmonic_morph: Smoother::new(0.0, 5.0, sample_rate),
            fm_osc: PhaseOscillator::new(sample_rate),
            fm_depth: Smoother::new(0.0, 5.0, sample_rate),
            fm_ratio: Smoother::new(2.0, 10.0, sample_rate),
            spread: 0.0,
            phase_offsets: [0.0; NUM_HARMONICS],
        }
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.fundamental = freq.max(1.0);
    }

    /// Sets the level for a single harmonic (0-indexed: 0 = fundamental).
    pub fn set_level(&mut self, index: usize, value: f32) {
        if index < NUM_HARMONICS {
            self.levels[index] = value.clamp(0.0, 1.0);
        }
    }

    pub fn set_tilt(&mut self, value: f32) {
        self.tilt.set_target(value.clamp(-1.0, 1.0));
    }

    pub fn set_odd_even(&mut self, value: f32) {
        self.odd_even.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_inharmonicity(&mut self, value: f32) {
        self.inharmonicity.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_scan_center(&mut self, value: f32) {
        self.scan_center.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_scan_width(&mut self, value: f32) {
        self.scan_width.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_harmonic_morph(&mut self, value: f32) {
        self.harmonic_morph.set_target(value.clamp(0.0, 1.0));
    }

    pub fn set_fm_depth(&mut self, value: f32) {
        self.fm_depth.set_target(value.clamp(0.0, 10.0));
    }

    pub fn set_fm_ratio(&mut self, value: f32) {
        self.fm_ratio.set_target(value.clamp(0.5, 16.0));
    }

    pub fn set_spread(&mut self, value: f32) {
        self.spread = value.clamp(0.0, 1.0);
        // Generate deterministic but perceptually random offsets
        // using a simple hash to avoid pulling in a PRNG dependency
        for i in 0..NUM_HARMONICS {
            let seed = (i as f32 + 1.0) * 0.618_034; // golden ratio fractional
            self.phase_offsets[i] = (seed.fract() * self.spread).rem_euclid(1.0);
        }
    }

    pub fn reset(&mut self) {
        self.phases = [0.0; NUM_HARMONICS];
        self.fm_osc.reset();
    }

    /// Gaussian scan window amplitude for harmonic index `i`.
    /// Center is in harmonic-index space (0.0 to 7.0), sigma from width.
    #[inline]
    fn scan_amplitude(i: usize, center: f32, width: f32) -> f32 {
        if width >= 0.99 {
            return 1.0; // all harmonics pass through
        }
        // sigma shrinks as width decreases: at width=0, sigma→very small
        let sigma = 0.3 + width * 3.5; // range: 0.3 (narrow) to 3.8 (wide)
        let dist = i as f32 - center;
        (-0.5 * (dist * dist) / (sigma * sigma)).exp()
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        let tilt = self.tilt.process();
        let odd_even = self.odd_even.process();
        let inharm = self.inharmonicity.process();
        let scan_c = self.scan_center.process();
        let scan_w = self.scan_width.process();
        let morph = self.harmonic_morph.process();
        let fm_d = self.fm_depth.process();
        let fm_r = self.fm_ratio.process();

        // Update FM oscillator frequency
        self.fm_osc.set_frequency(self.fundamental * fm_r);

        // Get FM modulator output (phase mod on the fundamental)
        let fm_pm = if fm_d > 0.001 {
            fm_d * self.fm_osc.sine() * TAU
        } else {
            0.0
        };
        self.fm_osc.advance();

        // Scan center mapped to harmonic index space (0-7)
        let scan_pos = scan_c * (NUM_HARMONICS - 1) as f32;

        let mut output = 0.0_f32;
        let mut total_amp = 0.0_f32;

        for i in 0..NUM_HARMONICS {
            let n = (i + 1) as f32; // harmonic number (1-8)
            let n_idx = i as f32; // 0-indexed

            // Frequency with inharmonicity stretching
            let stretch = (1.0 + inharm * 0.01 * n * n).sqrt();
            let freq = self.fundamental * n * stretch;

            // Skip harmonics above Nyquist
            if freq >= self.sample_rate * 0.49 {
                continue;
            }

            // Base level from user setting
            let base_level = self.levels[i];

            // Apply scan window (Gaussian centered at scan_pos)
            let scan_amp = Self::scan_amplitude(i, scan_pos, scan_w);

            // Apply tilt: negative favors low harmonics, positive favors high
            let tilt_scale = 1.0 + tilt * (n_idx / 7.0 - 0.5);
            let tilted = base_level * tilt_scale.max(0.0);

            // Apply odd/even balance: 0=odd-only, 0.5=balanced (all full), 1=even-only
            let is_odd = (i + 1) % 2 == 1;
            let oe_scale = if i == 0 {
                1.0 // fundamental always present
            } else if is_odd {
                (2.0 * (1.0 - odd_even)).min(1.0) // 0→1, 0.5→1, 1→0
            } else {
                (2.0 * odd_even).min(1.0) // 0→0, 0.5→1, 1→1
            };
            let amplitude = tilted * oe_scale.clamp(0.0, 1.0) * scan_amp;
            total_amp += amplitude;

            // Phase with spread offset + FM phase modulation
            // FM is applied proportionally to harmonic number
            let phase_with_offset = self.phases[i] + self.phase_offsets[i];
            let harmonic_pm = fm_pm * n; // FM affects higher harmonics more

            // Scan-morph coupling: harmonics near scan center get more morph
            let local_morph = morph * scan_amp;

            // Generate waveform: morph or sine depending on settings.
            // The sine paths route through `sine_normalized` (4096-entry
            // LUT in dsp-core) instead of f32::sin — the inner loop hits
            // 8 harmonics × per-sample, and a sin call costs ~30 cycles
            // on the CM5's Cortex-A76 vs ~5–7 for the LUT lookup. PM
            // adds the modulation in turns (radians / TAU) and lets
            // sine_normalized's wrap handle the resulting accumulator.
            let sample = if local_morph > 0.001 {
                morph_wave(phase_with_offset.rem_euclid(1.0), harmonic_pm, local_morph)
            } else if fm_d > 0.001 {
                sine_normalized(phase_with_offset + harmonic_pm * std::f32::consts::FRAC_1_PI * 0.5)
            } else {
                sine_normalized(phase_with_offset)
            };

            output += sample * amplitude;

            // Advance phase
            let phase_inc = freq / self.sample_rate;
            self.phases[i] += phase_inc;
            if self.phases[i] >= 1.0 {
                self.phases[i] -= 1.0;
            }
        }

        // Normalize to prevent clipping
        if total_amp > 1.0 {
            output /= total_amp;
        }

        output
    }
}

impl Engine for HarmonicOscillator {
    fn set_parameter(&mut self, id: ParameterId, value: f32) {
        match id {
            ParameterId::HarmonicLevel1 => self.set_level(0, value),
            ParameterId::HarmonicLevel2 => self.set_level(1, value),
            ParameterId::HarmonicLevel3 => self.set_level(2, value),
            ParameterId::HarmonicLevel4 => self.set_level(3, value),
            ParameterId::HarmonicLevel5 => self.set_level(4, value),
            ParameterId::HarmonicLevel6 => self.set_level(5, value),
            ParameterId::HarmonicLevel7 => self.set_level(6, value),
            ParameterId::HarmonicLevel8 => self.set_level(7, value),
            ParameterId::HarmonicTilt => self.set_tilt(value),
            ParameterId::HarmonicOddEven => self.set_odd_even(value),
            ParameterId::Inharmonicity => self.set_inharmonicity(value),
            ParameterId::ScanCenter => self.set_scan_center(value),
            ParameterId::ScanWidth => self.set_scan_width(value),
            ParameterId::HarmonicMorph => self.set_harmonic_morph(value),
            ParameterId::HarmonicFmDepth => self.set_fm_depth(value),
            ParameterId::HarmonicFmRatio => self.set_fm_ratio(value),
            ParameterId::HarmonicSpread => self.set_spread(value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn output_bounded() {
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(440.0);
        for i in 0..8 {
            osc.set_level(i, 1.0);
        }

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.abs() <= 1.5, "output should be bounded: {s}");
        }
    }

    #[test]
    fn fundamental_only_is_sine() {
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.set_level(0, 1.0);
        for i in 1..8 {
            osc.set_level(i, 0.0);
        }

        for _ in 0..4800 {
            let s = osc.process();
            assert!(
                (-1.01..=1.01).contains(&s),
                "fundamental should be bounded: {s}"
            );
        }
    }

    #[test]
    fn tilt_changes_spectrum() {
        let count_crossings = |tilt_val: f32| -> u32 {
            let mut osc = HarmonicOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.tilt.set_immediate(tilt_val);
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

        let low_tilt = count_crossings(-0.8);
        let high_tilt = count_crossings(0.8);
        assert!(
            high_tilt > low_tilt,
            "positive tilt should have more HF content: low={low_tilt}, high={high_tilt}"
        );
    }

    #[test]
    fn inharmonicity_stretches_partials() {
        let count_crossings = |inharm: f32| -> u32 {
            let mut osc = HarmonicOscillator::new(SR);
            osc.set_frequency(200.0);
            osc.inharmonicity.set_immediate(inharm);
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

        let harmonic = count_crossings(0.0);
        let inharmonic = count_crossings(1.0);
        assert!(
            harmonic != inharmonic,
            "inharmonicity should change spectrum: harmonic={harmonic}, inharmonic={inharmonic}"
        );
    }

    #[test]
    fn scan_isolates_harmonics() {
        // Narrow scan at center=0 should mostly be fundamental
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.scan_center.set_immediate(0.0);
        osc.scan_width.set_immediate(0.0);

        let mut crossings = 0_u32;
        let mut prev = 0.0_f32;
        for _ in 0..4800 {
            let s = osc.process();
            if prev <= 0.0 && s > 0.0 {
                crossings += 1;
            }
            prev = s;
        }
        // Should be near fundamental frequency crossings (~44 at 440Hz / 100ms)
        assert!(
            (38..=50).contains(&crossings),
            "narrow scan at H1 should isolate fundamental: got {crossings}"
        );
    }

    #[test]
    fn scan_at_high_harmonics() {
        // Narrow scan at top should emphasize higher harmonics
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(200.0);
        osc.scan_center.set_immediate(1.0); // center on H8
        osc.scan_width.set_immediate(0.0);

        let mut crossings = 0_u32;
        let mut prev = 0.0_f32;
        for _ in 0..4800 {
            let s = osc.process();
            if prev <= 0.0 && s > 0.0 {
                crossings += 1;
            }
            prev = s;
        }
        // H8 at 200Hz = 1600Hz, 4800 samples = 0.1s, expect ~160 crossings
        assert!(
            crossings > 100,
            "narrow scan at H8 should have many crossings: got {crossings}"
        );
    }

    #[test]
    fn morph_changes_waveform() {
        let energy_at = |morph_val: f32| -> f32 {
            let mut osc = HarmonicOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.set_level(0, 1.0);
            for i in 1..8 {
                osc.set_level(i, 0.0);
            }
            osc.harmonic_morph.set_immediate(morph_val);
            let mut e = 0.0_f32;
            for _ in 0..4800 {
                let s = osc.process();
                e += s * s;
            }
            e
        };
        let sine_e = energy_at(0.0);
        let square_e = energy_at(1.0);
        // Square wave has more energy than sine for same peak amplitude
        assert!(
            (square_e - sine_e).abs() > 10.0,
            "morph should change waveform energy: sine={sine_e}, square={square_e}"
        );
    }

    #[test]
    fn fm_bounded() {
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.fm_depth.set_immediate(5.0);
        osc.fm_ratio.set_immediate(3.0);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(
                s.is_finite() && s.abs() < 2.0,
                "FM output should be bounded: {s}"
            );
        }
    }

    #[test]
    fn spread_changes_waveform() {
        // Same spectrum, different phase relationships should produce different samples
        let samples_at = |spread_val: f32| -> Vec<f32> {
            let mut osc = HarmonicOscillator::new(SR);
            osc.set_frequency(440.0);
            osc.set_spread(spread_val);
            (0..480).map(|_| osc.process()).collect()
        };
        let locked = samples_at(0.0);
        let spread = samples_at(1.0);
        // Count how many samples differ significantly
        let diffs: usize = locked
            .iter()
            .zip(&spread)
            .filter(|(a, b)| (*a - *b).abs() > 0.001)
            .count();
        assert!(
            diffs > 100,
            "spread should change most samples: only {diffs}/480 differed"
        );
    }

    #[test]
    fn scan_morph_coupling() {
        // With morph + narrow scan, only the scanned harmonic should be morphed
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.harmonic_morph.set_immediate(1.0);
        osc.scan_center.set_immediate(0.0);
        osc.scan_width.set_immediate(0.0);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite() && s.abs() < 1.5, "scan+morph bounded: {s}");
        }
    }

    #[test]
    fn all_features_simultaneously() {
        let mut osc = HarmonicOscillator::new(SR);
        osc.set_frequency(220.0);
        osc.tilt.set_immediate(0.5);
        osc.odd_even.set_immediate(0.3);
        osc.inharmonicity.set_immediate(0.4);
        osc.scan_center.set_immediate(0.4);
        osc.scan_width.set_immediate(0.6);
        osc.harmonic_morph.set_immediate(0.5);
        osc.fm_depth.set_immediate(2.0);
        osc.fm_ratio.set_immediate(3.0);
        osc.set_spread(0.5);

        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite() && s.abs() < 3.0, "everything on: {s}");
        }
    }
}
