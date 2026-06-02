// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Stereo delay with ping-pong feedback and damping filter.

use brume_dsp_core::delay::DelayLine;
use brume_dsp_core::filter::OnePole;

use crate::{FxParamDef, FxSlot};

/// Maximum delay time: 2 seconds at 48kHz.
const MAX_DELAY_SAMPLES: usize = 96000;

pub struct StereoDelayFx {
    delay_l: DelayLine,
    delay_r: DelayLine,
    damp_l: OnePole,
    damp_r: OnePole,
    sample_rate: f32,
    time_ms: f32,
    feedback: f32,
    damping: f32,
    mix: f32,
    param_defs: Vec<FxParamDef>,
}

impl StereoDelayFx {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut damp_l = OnePole::lowpass(sample_rate);
        damp_l.set_cutoff(8000.0);
        let mut damp_r = OnePole::lowpass(sample_rate);
        damp_r.set_cutoff(8000.0);

        let mut s = Self {
            delay_l: DelayLine::new(MAX_DELAY_SAMPLES),
            delay_r: DelayLine::new(MAX_DELAY_SAMPLES),
            damp_l,
            damp_r,
            sample_rate,
            time_ms: 300.0,
            feedback: 0.3,
            damping: 0.3,
            mix: 0.0,
            param_defs: vec![
                FxParamDef {
                    name: "time".into(),
                    label: "TIME".into(),
                    min: 10.0,
                    max: 2000.0,
                    default: 300.0,
                    unit: Some("ms".into()),
                },
                FxParamDef {
                    name: "feedback".into(),
                    label: "FEEDBACK".into(),
                    min: 0.0,
                    max: 0.95,
                    default: 0.3,
                    unit: None,
                },
                FxParamDef {
                    name: "damping".into(),
                    label: "DAMPING".into(),
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
        };
        s.update_delay_time();
        s
    }

    fn update_delay_time(&mut self) {
        let samples = self.time_ms * self.sample_rate / 1000.0;
        self.delay_l.set_delay_samples(samples);
        // Right channel offset by golden ratio for ping-pong stereo spread
        let r_samples = (samples * 0.75).min((MAX_DELAY_SAMPLES - 1) as f32);
        self.delay_r.set_delay_samples(r_samples);
    }

    fn update_damping(&mut self) {
        // damping 0 = bright (20kHz), damping 1 = dark (800Hz)
        let cutoff = 20000.0 * (1.0 - self.damping * 0.96);
        self.damp_l.set_cutoff(cutoff);
        self.damp_r.set_cutoff(cutoff);
    }
}

impl FxSlot for StereoDelayFx {
    fn name(&self) -> &str {
        "Delay"
    }

    fn params(&self) -> &[FxParamDef] {
        &self.param_defs
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "time" => {
                self.time_ms = value.clamp(10.0, 2000.0);
                self.update_delay_time();
            }
            "feedback" => {
                self.feedback = value.clamp(0.0, 0.95);
            }
            "damping" => {
                self.damping = value.clamp(0.0, 1.0);
                self.update_damping();
            }
            "mix" => {
                self.mix = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_param(&self, name: &str) -> f32 {
        match name {
            "time" => self.time_ms,
            "feedback" => self.feedback,
            "damping" => self.damping,
            "mix" => self.mix,
            _ => 0.0,
        }
    }

    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.mix < 0.001 {
            return;
        }

        let fb = self.feedback;
        let mix = self.mix;

        for i in 0..left.len() {
            let dry_l = left[i];
            let dry_r = right[i];

            // Read from delay lines
            let tap_l = self
                .delay_l
                .process(dry_l + self.damp_r.process(right[i].clamp(-2.0, 2.0)) * fb);
            let tap_r = self
                .delay_r
                .process(dry_r + self.damp_l.process(left[i].clamp(-2.0, 2.0)) * fb);

            // Damp the delayed signal
            let wet_l = self.damp_l.process(tap_l.clamp(-2.0, 2.0));
            let wet_r = self.damp_r.process(tap_r.clamp(-2.0, 2.0));

            left[i] = dry_l * (1.0 - mix) + wet_l * mix;
            right[i] = dry_r * (1.0 - mix) + wet_r * mix;
        }
    }

    fn reset(&mut self) {
        self.delay_l.reset();
        self.delay_r.reset();
        self.damp_l.reset();
        self.damp_r.reset();
    }
}
