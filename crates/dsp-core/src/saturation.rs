// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Waveshaping and saturation effects.
//!
//! - [`Saturator`] — four curves (soft, hard, tape, tube)
//! - [`Wavefolder`] — triangle-fold distortion for complex harmonics
//!
//! The kit's `BitCrusher` is not vendored — nothing in the Brume code
//! path uses it.

#![allow(clippy::must_use_candidate)]

/// Type of saturation algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaturationType {
    /// Soft saturation using tanh waveshaping — musical and warm.
    #[default]
    Soft,
    /// Hard clipping to [-1, 1] — aggressive, digital character.
    Hard,
    /// Tape-style saturation with asymmetric response.
    Tape,
    /// Tube-style saturation emphasising even harmonics.
    Tube,
}

/// Waveshaping saturator with multiple algorithms. Drive controls both
/// pre-gain and dry/wet blend, giving smooth clean→driven control.
///
/// # Example
///
/// ```
/// use brume_dsp_core::saturation::{Saturator, SaturationType};
///
/// let mut sat = Saturator::new();
/// sat.set_drive(0.5);
/// sat.set_type(SaturationType::Tape);
/// let output = sat.process(0.8);
/// ```
pub struct Saturator {
    sat_type: SaturationType,
    drive: f32,
    mix: f32,
}

impl Saturator {
    /// Creates a new saturator with defaults (Soft, drive=0, mix=1).
    pub fn new() -> Self {
        Self {
            sat_type: SaturationType::Soft,
            drive: 0.0,
            mix: 1.0,
        }
    }

    /// Sets the saturation type.
    pub fn set_type(&mut self, sat_type: SaturationType) {
        self.sat_type = sat_type;
    }

    /// Returns the current saturation type.
    pub fn sat_type(&self) -> SaturationType {
        self.sat_type
    }

    /// Sets the drive amount (0.0 → 1.0). Higher pushes harder into
    /// saturation.
    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(0.0, 1.0);
    }

    /// Returns the current drive amount.
    pub fn drive(&self) -> f32 {
        self.drive
    }

    /// Sets the dry/wet mix (0.0 = dry, 1.0 = wet).
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Returns the current mix.
    pub fn mix(&self) -> f32 {
        self.mix
    }

    /// Processes a single sample.
    pub fn process(&mut self, input: f32) -> f32 {
        if self.drive < 0.001 {
            return input;
        }

        let pre_gain = 1.0 + self.drive * 4.0;
        let driven = input * pre_gain;

        let shaped = match self.sat_type {
            SaturationType::Soft => driven.tanh(),
            SaturationType::Hard => driven.clamp(-1.0, 1.0),
            SaturationType::Tape => Self::tape_saturation(driven),
            SaturationType::Tube => Self::tube_saturation(driven),
        };

        let drive_blend = input * (1.0 - self.drive) + shaped * self.drive;
        input * (1.0 - self.mix) + drive_blend * self.mix
    }

    /// Tape-style saturation — asymmetric soft clipping: softer on
    /// positive peaks, harder on negative.
    fn tape_saturation(x: f32) -> f32 {
        if x >= 0.0 {
            (x * 1.5).tanh() / 1.5_f32.tanh()
        } else {
            -(-x * 2.0).tanh() / 2.0_f32.tanh()
        }
    }

    /// Tube-style saturation emphasising even harmonics via a signed
    /// asymmetric polynomial term.
    fn tube_saturation(x: f32) -> f32 {
        let x_squared = x * x;
        let asymmetry = 0.2 * x_squared * x.signum();
        (x + asymmetry).tanh()
    }

    /// Processes a block of samples in-place.
    pub fn process_block(&mut self, samples: &mut [f32]) {
        if self.drive < 0.001 {
            return;
        }

        let pre_gain = 1.0 + self.drive * 4.0;
        let drive = self.drive;
        let mix = self.mix;
        let sat_type = self.sat_type;

        for sample in samples.iter_mut() {
            let input = *sample;
            let driven = input * pre_gain;

            let shaped = match sat_type {
                SaturationType::Soft => driven.tanh(),
                SaturationType::Hard => driven.clamp(-1.0, 1.0),
                SaturationType::Tape => Self::tape_saturation(driven),
                SaturationType::Tube => Self::tube_saturation(driven),
            };

            let drive_blend = input * (1.0 - drive) + shaped * drive;
            *sample = input * (1.0 - mix) + drive_blend * mix;
        }
    }
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

/// Wavefolding distortion — folds the signal back when it exceeds
/// thresholds, creating complex synth-like harmonic content.
pub struct Wavefolder {
    drive: f32,
    stages: u32,
    mix: f32,
}

impl Wavefolder {
    /// Creates a new wavefolder with defaults (drive=0.5, 1 stage, mix=1).
    pub fn new() -> Self {
        Self {
            drive: 0.5,
            stages: 1,
            mix: 1.0,
        }
    }

    /// Sets the drive amount (0.0 → 1.0).
    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(0.0, 1.0);
    }

    /// Returns the current drive amount.
    pub fn drive(&self) -> f32 {
        self.drive
    }

    /// Sets the number of folding stages (1–4). More stages create
    /// more complex harmonics.
    pub fn set_stages(&mut self, stages: u32) {
        self.stages = stages.clamp(1, 4);
    }

    /// Returns the number of folding stages.
    pub fn stages(&self) -> u32 {
        self.stages
    }

    /// Sets the dry/wet mix.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    /// Processes a single sample.
    pub fn process(&mut self, input: f32) -> f32 {
        if self.drive < 0.001 {
            return input;
        }

        let pre_gain = 1.0 + self.drive * 4.0;
        let mut x = input * pre_gain;

        for _ in 0..self.stages {
            x = Self::fold(x);
        }

        input * (1.0 - self.mix) + x * self.mix
    }

    /// Triangle-wave folding algorithm.
    fn fold(x: f32) -> f32 {
        let x = x * 0.25 + 0.25;
        (x - x.floor() - 0.5).abs() * 4.0 - 1.0
    }

    /// Processes a block of samples in-place.
    pub fn process_block(&mut self, samples: &mut [f32]) {
        if self.drive < 0.001 {
            return;
        }

        let pre_gain = 1.0 + self.drive * 4.0;
        let mix = self.mix;
        let stages = self.stages;

        for sample in samples.iter_mut() {
            let input = *sample;
            let mut x = input * pre_gain;

            for _ in 0..stages {
                x = Self::fold(x);
            }

            *sample = input * (1.0 - mix) + x * mix;
        }
    }
}

impl Default for Wavefolder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturator_zero_drive_passes_through() {
        let mut sat = Saturator::new();
        sat.set_drive(0.0);
        for i in 0..100 {
            let input = (i as f32 * 0.1).sin() * 2.0;
            let output = sat.process(input);
            assert!((input - output).abs() < 0.0001);
        }
    }

    #[test]
    fn soft_clip_bounds_output() {
        let mut sat = Saturator::new();
        sat.set_type(SaturationType::Soft);
        sat.set_drive(1.0);
        let output = sat.process(10.0);
        assert!(output.abs() <= 1.1, "got {output}");
    }

    #[test]
    fn hard_clip_bounds_output() {
        let mut sat = Saturator::new();
        sat.set_type(SaturationType::Hard);
        sat.set_drive(1.0);
        let output = sat.process(10.0);
        assert!(output.abs() <= 1.0, "got {output}");
    }

    #[test]
    fn tape_is_asymmetric() {
        let mut sat = Saturator::new();
        sat.set_type(SaturationType::Tape);
        sat.set_drive(0.8);
        let pos = sat.process(0.5);
        let neg = sat.process(-0.5);
        assert!((pos.abs() - neg.abs()).abs() > 0.001);
    }

    #[test]
    fn wavefolder_modifies_signal() {
        let mut folder = Wavefolder::new();
        folder.set_drive(0.8);
        folder.set_stages(2);
        let input = 0.8;
        let output = folder.process(input);
        assert!((output - input).abs() > 0.01);
    }

    #[test]
    fn wavefolder_output_is_bounded() {
        let mut folder = Wavefolder::new();
        folder.set_drive(1.0);
        folder.set_stages(4);
        for i in 0..100 {
            let input = (i as f32 * 0.1).sin() * 2.0;
            let output = folder.process(input);
            assert!(output.abs() <= 1.5, "got {output}");
        }
    }
}
