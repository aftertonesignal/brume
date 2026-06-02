// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Stereo chorus — two modulated delay lines with offset LFO phases.

use brume_dsp_core::delay::ModulatedDelay;

use crate::{FxParamDef, FxSlot};

pub struct ChorusFx {
    voice_l: ModulatedDelay,
    voice_r: ModulatedDelay,
    rate: f32,
    depth: f32,
    mix: f32,
    param_defs: Vec<FxParamDef>,
}

impl ChorusFx {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        // ~30ms max delay for chorus range
        let max_samples = (sample_rate * 0.04) as usize;

        let mut voice_l = ModulatedDelay::new(max_samples, sample_rate);
        voice_l.set_base_delay_ms(8.0);
        voice_l.set_depth_ms(2.0);
        voice_l.set_rate_hz(0.5);
        voice_l.set_feedback(0.05);

        let mut voice_r = ModulatedDelay::new(max_samples, sample_rate);
        voice_r.set_base_delay_ms(10.0); // offset for stereo width
        voice_r.set_depth_ms(2.5);
        voice_r.set_rate_hz(0.5);
        voice_r.set_feedback(0.05);

        Self {
            voice_l,
            voice_r,
            rate: 0.5,
            depth: 0.3,
            mix: 0.0,
            param_defs: vec![
                FxParamDef {
                    name: "rate".into(),
                    label: "RATE".into(),
                    min: 0.1,
                    max: 5.0,
                    default: 0.5,
                    unit: Some("Hz".into()),
                },
                FxParamDef {
                    name: "depth".into(),
                    label: "DEPTH".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.3,
                    unit: None,
                },
                FxParamDef {
                    name: "mix".into(),
                    label: "MIX".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    unit: None,
                },
            ],
        }
    }

    fn update_voices(&mut self) {
        let depth_ms = self.depth * 5.0; // 0-5ms modulation depth
        self.voice_l.set_rate_hz(self.rate);
        self.voice_l.set_depth_ms(depth_ms);
        self.voice_r.set_rate_hz(self.rate * 1.1); // slight rate offset for wider stereo
        self.voice_r.set_depth_ms(depth_ms * 1.15);
    }
}

impl FxSlot for ChorusFx {
    fn name(&self) -> &str {
        "Chorus"
    }

    fn params(&self) -> &[FxParamDef] {
        &self.param_defs
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "rate" => {
                self.rate = value.clamp(0.1, 5.0);
                self.update_voices();
            }
            "depth" => {
                self.depth = value.clamp(0.0, 1.0);
                self.update_voices();
            }
            "mix" => {
                self.mix = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_param(&self, name: &str) -> f32 {
        match name {
            "rate" => self.rate,
            "depth" => self.depth,
            "mix" => self.mix,
            _ => 0.0,
        }
    }

    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.mix < 0.001 {
            return;
        }

        let mix = self.mix;
        for i in 0..left.len() {
            let dry_l = left[i];
            let dry_r = right[i];

            let wet_l = self.voice_l.process(dry_l);
            let wet_r = self.voice_r.process(dry_r);

            left[i] = dry_l * (1.0 - mix) + wet_l * mix;
            right[i] = dry_r * (1.0 - mix) + wet_r * mix;
        }
    }

    fn reset(&mut self) {
        self.voice_l.reset();
        self.voice_r.reset();
    }
}
