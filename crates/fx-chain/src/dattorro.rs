// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Multi-algorithm stereo reverb.
//!
//! Four reverb types selectable at runtime:
//! - **Plate** (Dattorro): 4 input allpass diffusers → 4 parallel combs → L/R output allpass
//! - **Room**: shorter delays, fewer diffusers, clearer early reflections
//! - **Hall**: long delays, dense diffusion, expansive tail
//! - **Spring**: allpass cascade with modulated comb, metallic resonance

use brume_dsp_core::delay::{AllpassDelay, DelayLine};
use brume_dsp_core::filter::OnePole;
use brume_dsp_core::flush_subnormal;

use crate::{FxParamDef, FxSlot};

/// Comb filter with one-pole damping in the feedback loop.
struct DampedComb {
    delay: DelayLine,
    damping: OnePole,
    fb_sample: f32,
}

impl DampedComb {
    fn new(max_samples: usize, sample_rate: f32) -> Self {
        let mut delay = DelayLine::new(max_samples);
        delay.set_delay_samples((max_samples - 1) as f32);
        let mut damping = OnePole::lowpass(sample_rate);
        damping.set_cutoff(8000.0);
        Self {
            delay,
            damping,
            fb_sample: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damping_freq: f32) -> f32 {
        self.damping.set_cutoff(damping_freq);
        let delayed = self.delay.process(input + self.fb_sample);
        let fb = self.damping.process(delayed) * feedback;
        // Flush subnormal feedback. The damped comb's tail decays
        // asymptotically: after a reverb tail finishes, fb_sample
        // hovers in the 1e-30 range and slips into IEEE 754
        // subnormals, where every subsequent multiply pays a 10×
        // FPU penalty even at silence. The flush terminates the
        // decay at exact zero. See dsp-core::denormal.
        self.fb_sample = flush_subnormal(fb.clamp(-2.0, 2.0));
        delayed
    }

    fn reset(&mut self) {
        self.delay.reset();
        self.damping.reset();
        self.fb_sample = 0.0;
    }
}

fn make_ap(base: f32, rate_scale: f32, gain: f32) -> AllpassDelay {
    let max = (base * rate_scale) as usize + 2;
    let mut ap = AllpassDelay::new(max);
    ap.set_delay_samples(base * rate_scale);
    ap.set_gain(gain);
    ap
}

fn make_comb(base: f32, rate_scale: f32, sample_rate: f32) -> DampedComb {
    let max = (base * rate_scale) as usize + 2;
    DampedComb::new(max, sample_rate)
}

// ── Plate (Dattorro) topology ──
struct PlateState {
    input_aps: [AllpassDelay; 4],
    combs: [DampedComb; 4],
    output_aps_l: [AllpassDelay; 2],
    output_aps_r: [AllpassDelay; 2],
}

impl PlateState {
    fn new(sample_rate: f32) -> Self {
        let rs = sample_rate / 48000.0;
        Self {
            input_aps: [
                make_ap(1051.0, rs, 0.5),
                make_ap(1327.0, rs, 0.5),
                make_ap(1583.0, rs, 0.5),
                make_ap(1877.0, rs, 0.5),
            ],
            combs: [
                make_comb(4409.0, rs, sample_rate),
                make_comb(5101.0, rs, sample_rate),
                make_comb(5801.0, rs, sample_rate),
                make_comb(6607.0, rs, sample_rate),
            ],
            output_aps_l: [make_ap(1013.0, rs, 0.5), make_ap(1367.0, rs, 0.5)],
            output_aps_r: [make_ap(1151.0, rs, 0.5), make_ap(1481.0, rs, 0.5)],
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp_freq: f32) -> (f32, f32) {
        let mut d = self.input_aps[0].process(input * 0.4);
        d = self.input_aps[1].process(d).clamp(-2.0, 2.0);
        d = self.input_aps[2].process(d).clamp(-2.0, 2.0);
        d = self.input_aps[3].process(d).clamp(-2.0, 2.0);

        let c0 = self.combs[0].process(d, feedback, damp_freq);
        let c1 = self.combs[1].process(d, feedback, damp_freq);
        let c2 = self.combs[2].process(d, feedback, damp_freq);
        let c3 = self.combs[3].process(d, feedback, damp_freq);

        let late_l = ((c0 + c1) * 0.45).clamp(-2.0, 2.0);
        let late_r = ((c2 + c3) * 0.45).clamp(-2.0, 2.0);

        let mut wl = self.output_aps_l[0].process(late_l);
        wl = self.output_aps_l[1].process(wl).clamp(-2.0, 2.0);
        let mut wr = self.output_aps_r[0].process(late_r);
        wr = self.output_aps_r[1].process(wr).clamp(-2.0, 2.0);
        (wl, wr)
    }

    fn reset(&mut self) {
        for ap in &mut self.input_aps {
            ap.reset();
        }
        for c in &mut self.combs {
            c.reset();
        }
        for ap in &mut self.output_aps_l {
            ap.reset();
        }
        for ap in &mut self.output_aps_r {
            ap.reset();
        }
    }
}

// ── Room topology: shorter, clearer ──
struct RoomState {
    input_aps: [AllpassDelay; 2],
    combs: [DampedComb; 4],
    output_aps_l: [AllpassDelay; 1],
    output_aps_r: [AllpassDelay; 1],
}

impl RoomState {
    fn new(sample_rate: f32) -> Self {
        let rs = sample_rate / 48000.0;
        Self {
            input_aps: [make_ap(443.0, rs, 0.4), make_ap(631.0, rs, 0.4)],
            combs: [
                make_comb(1787.0, rs, sample_rate),
                make_comb(2053.0, rs, sample_rate),
                make_comb(2311.0, rs, sample_rate),
                make_comb(2677.0, rs, sample_rate),
            ],
            output_aps_l: [make_ap(547.0, rs, 0.35)],
            output_aps_r: [make_ap(613.0, rs, 0.35)],
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp_freq: f32) -> (f32, f32) {
        let mut d = self.input_aps[0].process(input * 0.5);
        d = self.input_aps[1].process(d).clamp(-2.0, 2.0);

        let c0 = self.combs[0].process(d, feedback, damp_freq);
        let c1 = self.combs[1].process(d, feedback, damp_freq);
        let c2 = self.combs[2].process(d, feedback, damp_freq);
        let c3 = self.combs[3].process(d, feedback, damp_freq);

        let late_l = ((c0 + c1) * 0.5).clamp(-2.0, 2.0);
        let late_r = ((c2 + c3) * 0.5).clamp(-2.0, 2.0);

        let wl = self.output_aps_l[0].process(late_l).clamp(-2.0, 2.0);
        let wr = self.output_aps_r[0].process(late_r).clamp(-2.0, 2.0);
        (wl, wr)
    }

    fn reset(&mut self) {
        for ap in &mut self.input_aps {
            ap.reset();
        }
        for c in &mut self.combs {
            c.reset();
        }
        for ap in &mut self.output_aps_l {
            ap.reset();
        }
        for ap in &mut self.output_aps_r {
            ap.reset();
        }
    }
}

// ── Hall topology: long, dense ──
struct HallState {
    input_aps: [AllpassDelay; 6],
    combs: [DampedComb; 4],
    output_aps_l: [AllpassDelay; 2],
    output_aps_r: [AllpassDelay; 2],
}

impl HallState {
    fn new(sample_rate: f32) -> Self {
        let rs = sample_rate / 48000.0;
        Self {
            input_aps: [
                make_ap(1433.0, rs, 0.55),
                make_ap(1811.0, rs, 0.55),
                make_ap(2179.0, rs, 0.55),
                make_ap(2591.0, rs, 0.5),
                make_ap(3001.0, rs, 0.5),
                make_ap(3371.0, rs, 0.45),
            ],
            combs: [
                make_comb(7919.0, rs, sample_rate),
                make_comb(8627.0, rs, sample_rate),
                make_comb(9341.0, rs, sample_rate),
                make_comb(10007.0, rs, sample_rate),
            ],
            output_aps_l: [make_ap(1699.0, rs, 0.45), make_ap(2131.0, rs, 0.45)],
            output_aps_r: [make_ap(1871.0, rs, 0.45), make_ap(2293.0, rs, 0.45)],
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp_freq: f32) -> (f32, f32) {
        let mut d = self.input_aps[0].process(input * 0.3);
        for ap in &mut self.input_aps[1..] {
            d = ap.process(d).clamp(-2.0, 2.0);
        }

        let c0 = self.combs[0].process(d, feedback, damp_freq);
        let c1 = self.combs[1].process(d, feedback, damp_freq);
        let c2 = self.combs[2].process(d, feedback, damp_freq);
        let c3 = self.combs[3].process(d, feedback, damp_freq);

        let late_l = ((c0 + c1) * 0.4).clamp(-2.0, 2.0);
        let late_r = ((c2 + c3) * 0.4).clamp(-2.0, 2.0);

        let mut wl = self.output_aps_l[0].process(late_l);
        wl = self.output_aps_l[1].process(wl).clamp(-2.0, 2.0);
        let mut wr = self.output_aps_r[0].process(late_r);
        wr = self.output_aps_r[1].process(wr).clamp(-2.0, 2.0);
        (wl, wr)
    }

    fn reset(&mut self) {
        for ap in &mut self.input_aps {
            ap.reset();
        }
        for c in &mut self.combs {
            c.reset();
        }
        for ap in &mut self.output_aps_l {
            ap.reset();
        }
        for ap in &mut self.output_aps_r {
            ap.reset();
        }
    }
}

// ── Spring topology: allpass cascade with resonant character ──
struct SpringState {
    aps: [AllpassDelay; 6],
    comb_l: DampedComb,
    comb_r: DampedComb,
    chirp: AllpassDelay, // short modulated allpass for spring chirp
}

impl SpringState {
    fn new(sample_rate: f32) -> Self {
        let rs = sample_rate / 48000.0;
        Self {
            aps: [
                make_ap(311.0, rs, 0.6),
                make_ap(509.0, rs, 0.6),
                make_ap(787.0, rs, 0.55),
                make_ap(1103.0, rs, 0.55),
                make_ap(1597.0, rs, 0.5),
                make_ap(2203.0, rs, 0.5),
            ],
            comb_l: make_comb(3307.0, rs, sample_rate),
            comb_r: make_comb(3571.0, rs, sample_rate),
            chirp: make_ap(149.0, rs, 0.7), // high gain short AP for metallic chirp
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp_freq: f32) -> (f32, f32) {
        // Spring character: long allpass chain creates dispersion
        let mut d = self.chirp.process(input * 0.5).clamp(-2.0, 2.0);
        for ap in &mut self.aps {
            d = ap.process(d).clamp(-2.0, 2.0);
        }

        // Lower damping frequency for spring's darker character
        let spring_damp = (damp_freq * 0.6).max(800.0);
        let wl = self
            .comb_l
            .process(d, feedback * 0.9, spring_damp)
            .clamp(-2.0, 2.0);
        let wr = self
            .comb_r
            .process(d, feedback * 0.9, spring_damp)
            .clamp(-2.0, 2.0);
        (wl * 0.7, wr * 0.7)
    }

    fn reset(&mut self) {
        for ap in &mut self.aps {
            ap.reset();
        }
        self.comb_l.reset();
        self.comb_r.reset();
        self.chirp.reset();
    }
}

// ── Public reverb with type selection ──

#[derive(Clone, Copy, PartialEq)]
pub enum ReverbType {
    Plate,
    Room,
    Hall,
    Spring,
}

pub struct DattorroReverb {
    plate: PlateState,
    room: RoomState,
    hall: HallState,
    spring: SpringState,
    predelay: DelayLine,
    active_type: ReverbType,
    predelay_ms: f32,
    sample_rate: f32,
    decay: f32,
    damping: f32,
    mix: f32,
    param_defs: Vec<FxParamDef>,
}

impl DattorroReverb {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        // Max predelay: 200ms
        let max_predelay = (sample_rate * 0.2) as usize + 1;
        let mut predelay = DelayLine::new(max_predelay);
        predelay.set_delay_samples(sample_rate * 0.02); // 20ms default

        Self {
            plate: PlateState::new(sample_rate),
            room: RoomState::new(sample_rate),
            hall: HallState::new(sample_rate),
            spring: SpringState::new(sample_rate),
            predelay,
            active_type: ReverbType::Plate,
            predelay_ms: 20.0,
            sample_rate,
            decay: 2.0,
            damping: 0.4,
            mix: 0.0,
            param_defs: vec![
                FxParamDef {
                    name: "type".into(),
                    label: "TYPE".into(),
                    min: 0.0,
                    max: 3.0,
                    default: 0.0,
                    unit: None,
                },
                FxParamDef {
                    name: "predelay".into(),
                    label: "PREDELAY".into(),
                    min: 0.0,
                    max: 200.0,
                    default: 20.0,
                    unit: Some("ms".into()),
                },
                FxParamDef {
                    name: "decay".into(),
                    label: "DECAY".into(),
                    min: 0.1,
                    max: 8.0,
                    default: 2.0,
                    unit: None,
                },
                FxParamDef {
                    name: "damping".into(),
                    label: "DAMPING".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.4,
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

    fn process_sample(&mut self, input: f32) -> (f32, f32) {
        let damp_freq = 2000.0 + self.damping * 14000.0;
        let feedback = (0.4 + self.decay * 0.07).min(0.93);
        let safe = self.predelay.process(input.clamp(-2.0, 2.0));

        match self.active_type {
            ReverbType::Plate => self.plate.process(safe, feedback, damp_freq),
            ReverbType::Room => self.room.process(safe, feedback * 0.85, damp_freq),
            ReverbType::Hall => self.hall.process(safe, feedback, damp_freq * 0.8),
            ReverbType::Spring => self.spring.process(safe, feedback, damp_freq),
        }
    }
}

impl FxSlot for DattorroReverb {
    fn name(&self) -> &str {
        "Reverb"
    }
    fn params(&self) -> &[FxParamDef] {
        &self.param_defs
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "type" => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let t = value as u32;
                self.active_type = match t {
                    0 => ReverbType::Plate,
                    1 => ReverbType::Room,
                    2 => ReverbType::Hall,
                    _ => ReverbType::Spring,
                };
            }
            "predelay" => {
                self.predelay_ms = value.clamp(0.0, 200.0);
                self.predelay
                    .set_delay_samples(self.predelay_ms * self.sample_rate / 1000.0);
            }
            "decay" => {
                self.decay = value.clamp(0.1, 8.0);
            }
            "damping" => {
                self.damping = value.clamp(0.0, 1.0);
            }
            "mix" => {
                self.mix = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_param(&self, name: &str) -> f32 {
        match name {
            "type" => self.active_type as u32 as f32,
            "predelay" => self.predelay_ms,
            "decay" => self.decay,
            "damping" => self.damping,
            "mix" => self.mix,
            _ => 0.0,
        }
    }

    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.mix < 0.001 {
            return;
        }
        let mix = self.mix;
        let dry = 1.0 - mix;
        for i in 0..left.len() {
            let mono = (left[i] + right[i]) * 0.5;
            let (wl, wr) = self.process_sample(mono);
            left[i] = left[i] * dry + wl * mix;
            right[i] = right[i] * dry + wr * mix;
        }
    }

    fn reset(&mut self) {
        self.plate.reset();
        self.room.reset();
        self.hall.reset();
        self.spring.reset();
        self.predelay.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverb_bounded() {
        let mut rev = DattorroReverb::new(48000.0);
        rev.set_param("mix", 0.5);
        rev.set_param("decay", 5.0);
        let mut left = vec![0.0_f32; 48000];
        let mut right = vec![0.0_f32; 48000];
        left[0] = 1.0;
        right[0] = 1.0;
        rev.process_stereo(&mut left, &mut right);
        for (i, (&l, &r)) in left.iter().zip(&right).enumerate() {
            assert!(l.is_finite() && l.abs() < 3.0, "L unbounded at {i}: {l}");
            assert!(r.is_finite() && r.abs() < 3.0, "R unbounded at {i}: {r}");
        }
    }

    #[test]
    fn reverb_produces_tail() {
        let mut rev = DattorroReverb::new(48000.0);
        rev.set_param("mix", 1.0);
        rev.set_param("decay", 3.0);
        let mut left = vec![0.0_f32; 48000];
        let mut right = vec![0.0_f32; 48000];
        left[0] = 1.0;
        right[0] = 1.0;
        rev.process_stereo(&mut left, &mut right);
        let late: f32 = left[24000..].iter().map(|s| s * s).sum();
        assert!(late > 0.0001, "reverb should have a tail: {late}");
    }

    #[test]
    fn all_types_bounded() {
        for t in 0..4 {
            let mut rev = DattorroReverb::new(48000.0);
            rev.set_param("type", t as f32);
            rev.set_param("mix", 0.8);
            rev.set_param("decay", 5.0);
            let mut left = vec![0.0_f32; 24000];
            let mut right = vec![0.0_f32; 24000];
            left[0] = 1.0;
            right[0] = 1.0;
            rev.process_stereo(&mut left, &mut right);
            for (&l, &r) in left.iter().zip(&right) {
                assert!(l.is_finite() && l.abs() < 3.0, "type {t} L unbounded: {l}");
                assert!(r.is_finite() && r.abs() < 3.0, "type {t} R unbounded: {r}");
            }
        }
    }

    #[test]
    fn types_sound_different() {
        let energy_at = |t: u32| -> f32 {
            let mut rev = DattorroReverb::new(48000.0);
            rev.set_param("type", t as f32);
            rev.set_param("mix", 1.0);
            rev.set_param("decay", 3.0);
            let mut left = vec![0.0_f32; 24000];
            let mut right = vec![0.0_f32; 24000];
            left[0] = 1.0;
            right[0] = 1.0;
            rev.process_stereo(&mut left, &mut right);
            left.iter().map(|s| s * s).sum()
        };
        let plate = energy_at(0);
        let room = energy_at(1);
        let hall = energy_at(2);
        let spring = energy_at(3);
        // Each type should have different energy profiles
        assert!(plate > 0.0 && room > 0.0 && hall > 0.0 && spring > 0.0);
        assert!(
            (plate - room).abs() > 0.01 || (plate - hall).abs() > 0.01,
            "types should differ: plate={plate}, room={room}, hall={hall}, spring={spring}"
        );
    }
}
