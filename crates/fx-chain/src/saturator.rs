// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Saturator effect slot — wraps brume-dsp-core's Saturator.

use brume_dsp_core::saturation::{SaturationType, Saturator};

use crate::{FxParamDef, FxSlot};

pub struct SaturatorFx {
    sat: Saturator,
    drive: f32,
    mix: f32,
    sat_type: f32, // 0=soft, 0.33=hard, 0.66=tape, 1.0=tube
    param_defs: Vec<FxParamDef>,
}

impl SaturatorFx {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sat: Saturator::new(),
            drive: 0.0,
            mix: 1.0,
            sat_type: 0.0,
            param_defs: vec![
                FxParamDef {
                    name: "drive".into(),
                    label: "DRIVE".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    unit: None,
                },
                FxParamDef {
                    name: "type".into(),
                    label: "TYPE".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    unit: None,
                },
                FxParamDef {
                    name: "mix".into(),
                    label: "MIX".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 1.0,
                    unit: None,
                },
            ],
        }
    }

    fn update_type(&mut self) {
        let t = if self.sat_type < 0.25 {
            SaturationType::Soft
        } else if self.sat_type < 0.5 {
            SaturationType::Hard
        } else if self.sat_type < 0.75 {
            SaturationType::Tape
        } else {
            SaturationType::Tube
        };
        self.sat.set_type(t);
    }
}

impl FxSlot for SaturatorFx {
    fn name(&self) -> &str {
        "Saturator"
    }

    fn params(&self) -> &[FxParamDef] {
        &self.param_defs
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "drive" => {
                self.drive = value.clamp(0.0, 1.0);
                self.sat.set_drive(self.drive);
            }
            "type" => {
                self.sat_type = value.clamp(0.0, 1.0);
                self.update_type();
            }
            "mix" => {
                self.mix = value.clamp(0.0, 1.0);
                self.sat.set_mix(self.mix);
            }
            _ => {}
        }
    }

    fn get_param(&self, name: &str) -> f32 {
        match name {
            "drive" => self.drive,
            "type" => self.sat_type,
            "mix" => self.mix,
            _ => 0.0,
        }
    }

    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.drive < 0.001 {
            return;
        }
        self.sat.process_block(left);
        self.sat.process_block(right);
    }

    fn reset(&mut self) {
        // Saturator is stateless (no delay lines or filters)
    }
}
