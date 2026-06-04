// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! 6-operator FM oscillator — Digitone/DX7-style digital synthesis.
//!
//! Six sine operators arranged via a data-driven algorithm table. Each
//! algorithm describes which operators modulate which (phase modulation)
//! and which operators contribute to final output (carriers). Algorithm
//! data is expressed as `mod_into: [u8; 6]` + `carriers: u8` bitmasks —
//! deliberately table-driven so a future Lua API can push user-defined
//! algorithms into the same array the engine reads from.
//!
//! Evaluation convention: operators are indexed 0..6 (UI-facing as
//! OP1..OP6). Operator `i` is evaluated after operators `j > i`, so
//! higher-numbered ops can modulate lower-numbered ops. Self-feedback
//! uses the operator's previous-sample output (`z⁻¹`). This matches
//! the classic DX7 constraint and keeps the graph trivially schedulable.

use brume_common::ParameterId;
use brume_dsp_core::{Smoother, sine_normalized};

use crate::oscillator_core::Engine;

/// Number of operators per voice.
pub const NUM_OPS: usize = 6;

/// A single FM operator: sine oscillator with phase state, ratio to
/// the voice's base frequency, and output level.
pub struct FmOperator {
    /// Phase in 0..1 (one full cycle).
    phase: f32,
    /// Per-sample phase increment.
    phase_inc: f32,
    /// Frequency ratio: op freq = base_freq × ratio.
    ratio: Smoother,
    /// Output level — used both as modulator depth and carrier gain.
    level: Smoother,
    /// Previous output sample, used for self-feedback.
    prev_output: f32,
}

impl FmOperator {
    fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            phase_inc: 0.0,
            ratio: Smoother::new(1.0, 5.0, sample_rate),
            level: Smoother::new(0.0, 5.0, sample_rate),
            prev_output: 0.0,
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.prev_output = 0.0;
    }
}

/// One FM algorithm: a routing description.
///
/// `mod_into[i]` is a bitmask of which operators feed the phase of op `i`.
/// Bit `j` set ⇒ op `j`'s output phase-modulates op `i`. A bit set where
/// `j == i` indicates self-feedback (uses op `i`'s previous output,
/// scaled by the global feedback amount).
///
/// `carriers` is a bitmask of which operators contribute to the final
/// output.
#[derive(Debug, Clone, Copy)]
pub struct Algorithm {
    pub name: &'static str,
    pub mod_into: [u8; NUM_OPS],
    pub carriers: u8,
}

/// Curated starting set. Eight algorithms covering the most musically
/// useful routings: from clean bells (stack) through paired carriers
/// (dual FM voices) to pure additive (all carriers).
///
/// **Feedback convention.** Every algorithm sets OP6's self-loop bit
/// (bit 5 of `mod_into[5]`), so the global FDBK knob is active no
/// matter which algorithm is selected. At FDBK=0 the self-feedback
/// contribution is 0 and the algorithm behaves as "plain" routing.
/// At nonzero FDBK, OP6 self-modulates — the classic source of
/// DX-era "digital grit" that separates polite bells from bitier
/// FM saws. On ADDITIVE (OP6 is a carrier, not a modulator),
/// feedback still warms OP6's own sine into a richer waveform.
pub const ALGORITHMS: [Algorithm; 12] = [
    // 1. STACK — OP6→OP5→OP4→OP3→OP2→OP1, OP1 is carrier.
    //    Classic bright FM: one carrier, five modulators in a deep
    //    stack. Produces rich, bell-like spectra.
    Algorithm {
        name: "STACK",
        mod_into: [
            0b000010, // OP1 ← OP2
            0b000100, // OP2 ← OP3
            0b001000, // OP3 ← OP4
            0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback (FDBK)
        ],
        carriers: 0b000001, // OP1
    },
    // 2. DOUBLE — two parallel 3-op stacks, both feeding OP1.
    //    (OP6→OP5→OP4) + (OP3→OP2) → OP1. Richer sideband structure
    //    than single STACK via two independent modulator chains.
    Algorithm {
        name: "DOUBLE",
        mod_into: [
            0b001100, // OP1 ← OP3 + OP4
            0b000100, // OP2 ← OP3
            0, 0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b000001, // OP1
    },
    // 3. TWIN — two parallel 2-op pairs, each feeding its own carrier.
    //    (OP6→OP5)→OP1 and (OP4→OP3)→OP2. Two carriers with
    //    independent modulation sources — good for layered timbres.
    Algorithm {
        name: "TWIN",
        mod_into: [
            0b010000, // OP1 ← OP5
            0b000100, // OP2 ← OP3
            0b001000, // OP3 ← OP4
            0, 0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b000011, // OP1, OP2
    },
    // 4. TRIO — three parallel pairs. OP6→OP3, OP5→OP2, OP4→OP1.
    //    Three carriers each with a single modulator. Clean, chord-
    //    like textures; good for pads and sustained tones.
    Algorithm {
        name: "TRIO",
        mod_into: [
            0b001000, // OP1 ← OP4
            0b010000, // OP2 ← OP5
            0b100000, // OP3 ← OP6
            0, 0, 0b100000, // OP6 self-feedback
        ],
        carriers: 0b000111, // OP1, OP2, OP3
    },
    // 5. FAN-IN — OP2..OP6 all modulate OP1.
    //    Five modulators converging on one carrier. Produces the most
    //    complex harmonic content of any algorithm — dense, metallic,
    //    inharmonic when ratios are non-integer.
    Algorithm {
        name: "FAN-IN",
        mod_into: [
            0b111110, // OP1 ← OP2, OP3, OP4, OP5, OP6
            0, 0, 0, 0, 0b100000, // OP6 self-feedback
        ],
        carriers: 0b000001,
    },
    // 6. BRANCH — OP6→OP5, then OP5 shared into OP4, OP3, OP2, OP1.
    //    One shared modulator drives four parallel carriers — creates
    //    coherent pitched-then-detuned chorusing when carrier ratios
    //    are staggered.
    Algorithm {
        name: "BRANCH",
        mod_into: [
            0b010000, // OP1 ← OP5
            0b010000, // OP2 ← OP5
            0b010000, // OP3 ← OP5
            0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b001111, // OP1..OP4
    },
    // 7. HYBRID — OP6→OP5→OP4 as a modulated carrier, plus OP3, OP2,
    //    OP1 as independent additive carriers. Mixes a bell-like
    //    FM voice with simple additive partials. Good for layered
    //    electric-piano-style patches.
    Algorithm {
        name: "HYBRID",
        mod_into: [
            0, 0, 0, 0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b001111, // OP1, OP2, OP3, OP4
    },
    // 8. ADDITIVE — no modulation between ops, all six as parallel
    //    carriers. OP6's self-feedback is still active so FDBK warms
    //    its own sine (useful creative edge, not a bypass). Organ-
    //    like and bell-choir textures, ratios become harmonic series.
    Algorithm {
        name: "ADDITIVE",
        mod_into: [0, 0, 0, 0, 0, 0b100000],
        carriers: 0b111111,
    },
    // 9. PAIRS — three independent 2-op stacks, each with its
    //    modulator directly on a carrier: (OP6→OP5), (OP4→OP3),
    //    (OP2→OP1). All three carriers sum. Each stack has its own
    //    modulator ratio, so the three timbres can be dialed in
    //    independently — classic e-piano / bell choir territory.
    Algorithm {
        name: "PAIRS",
        mod_into: [
            0b000010, // OP1 ← OP2
            0, 0b001000, // OP3 ← OP4
            0, 0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b010101, // OP1, OP3, OP5
    },
    // 10. TOWER — deep 4-op stack OP6→OP5→OP4→OP3 with OP3 as the
    //     FM-driven carrier, plus OP1 and OP2 as independent pure
    //     sine carriers. The tall stack gives OP3 DX7-level bite;
    //     OP1/OP2 are clean partials adding body underneath.
    Algorithm {
        name: "TOWER",
        mod_into: [
            0, 0, 0b001000, // OP3 ← OP4
            0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b000111, // OP1, OP2, OP3
    },
    // 11. FUNNEL — OP5 and OP6 both modulate OP4, which modulates
    //     OP1. OP2 and OP3 are independent carriers. Two mods
    //     converging on one mod converging on one carrier yields
    //     complex, evolving inharmonic spectra — inorganic and
    //     digital, unlike PAIRS' organic stacks.
    Algorithm {
        name: "FUNNEL",
        mod_into: [
            0b001000, // OP1 ← OP4
            0, 0, 0b110000, // OP4 ← OP5, OP6
            0, 0b100000, // OP6 self-feedback
        ],
        carriers: 0b000111, // OP1, OP2, OP3
    },
    // 12. CHAIN — 5-op stack OP6→OP5→OP4→OP3→OP2 with OP2 as the
    //     deep-FM carrier, plus OP1 as an independent carrier.
    //     Similar depth to STACK but shifts the carrier position
    //     so OP1's ratio becomes a harmonic reference against
    //     OP2's FM content — useful for bass patches that want a
    //     clean fundamental under a bright overtone cluster.
    Algorithm {
        name: "CHAIN",
        mod_into: [
            0, 0b000100, // OP2 ← OP3
            0b001000, // OP3 ← OP4
            0b010000, // OP4 ← OP5
            0b100000, // OP5 ← OP6
            0b100000, // OP6 self-feedback
        ],
        carriers: 0b000011, // OP1, OP2
    },
];

#[derive(Debug, Clone, Copy)]
struct ModRoute {
    inputs: [usize; NUM_OPS],
    input_count: usize,
    self_feedback: bool,
}

const EMPTY_MOD_ROUTE: ModRoute = ModRoute {
    inputs: [0; NUM_OPS],
    input_count: 0,
    self_feedback: false,
};

#[derive(Debug, Clone, Copy)]
struct AlgorithmSchedule {
    mod_routes: [ModRoute; NUM_OPS],
    carriers: [usize; NUM_OPS],
    carrier_count: usize,
}

const EMPTY_ALGORITHM_SCHEDULE: AlgorithmSchedule = AlgorithmSchedule {
    mod_routes: [EMPTY_MOD_ROUTE; NUM_OPS],
    carriers: [0; NUM_OPS],
    carrier_count: 0,
};

const fn build_algorithm_schedule(algorithm: Algorithm) -> AlgorithmSchedule {
    let mut schedule = EMPTY_ALGORITHM_SCHEDULE;
    let mut i = 0;

    while i < NUM_OPS {
        let mask = algorithm.mod_into[i];
        let mut j = 0;

        while j < NUM_OPS {
            if mask & (1u8 << j) != 0 {
                if j == i {
                    schedule.mod_routes[i].self_feedback = true;
                } else {
                    let count = schedule.mod_routes[i].input_count;
                    schedule.mod_routes[i].inputs[count] = j;
                    schedule.mod_routes[i].input_count = count + 1;
                }
            }
            j += 1;
        }

        i += 1;
    }

    let mut i = 0;
    while i < NUM_OPS {
        if algorithm.carriers & (1u8 << i) != 0 {
            let count = schedule.carrier_count;
            schedule.carriers[count] = i;
            schedule.carrier_count = count + 1;
        }
        i += 1;
    }

    schedule
}

const fn build_algorithm_schedules() -> [AlgorithmSchedule; ALGORITHMS.len()] {
    let mut schedules = [EMPTY_ALGORITHM_SCHEDULE; ALGORITHMS.len()];
    let mut i = 0;

    while i < ALGORITHMS.len() {
        schedules[i] = build_algorithm_schedule(ALGORITHMS[i]);
        i += 1;
    }

    schedules
}

const ALGORITHM_SCHEDULES: [AlgorithmSchedule; ALGORITHMS.len()] = build_algorithm_schedules();

// Sine LUT moved to brume_dsp_core::sine_lut to share with the
// Harmonic oscillator. Both engines now hit one cache-warm copy of
// the 4096-entry table instead of duplicating it per crate.

/// The FM oscillator: 6 operators + one algorithm.
pub struct FmOscillator {
    ops: [FmOperator; NUM_OPS],
    algorithm_idx: u8,
    feedback: Smoother,
    // fm_index is a plain f32, not a Smoother. The voice layer
    // already hands us a per-sample smoothed value (fm_index_base +
    // FmIndexEnv contribution + mod-router contribution), so
    // wrapping that in another Smoother here stacks two 5 ms
    // filters in series — producing a ~40 ms perceptible lag when
    // the user sweeps the FmIndex knob. The input is already
    // glitch-safe; just store and use it.
    fm_index: f32,
    base_frequency: f32,
    sample_rate: f32,
}

impl FmOscillator {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            ops: [
                FmOperator::new(sample_rate),
                FmOperator::new(sample_rate),
                FmOperator::new(sample_rate),
                FmOperator::new(sample_rate),
                FmOperator::new(sample_rate),
                FmOperator::new(sample_rate),
            ],
            algorithm_idx: 0,
            feedback: Smoother::new(0.0, 5.0, sample_rate),
            fm_index: 0.5,
            base_frequency: 440.0,
            sample_rate,
        };
        // Default ratios 1, 2, 3, 4, 5, 6 — harmonic series.
        let ratios = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        for (op, &r) in s.ops.iter_mut().zip(ratios.iter()) {
            op.ratio.set_immediate(r);
        }
        // Stack-algorithm modulators compound: OP6 modulates OP5,
        // which modulates OP4, and so on down to OP1. Each stage's
        // contribution multiplies into the next. A flat 0.5 across
        // all modulators pushes the cascade into noise territory
        // once the filter opens; this tapered descent keeps the
        // bell bright without shrieking. Carrier (OP1) stays loud.
        let defaults = [0.8, 0.4, 0.3, 0.25, 0.2, 0.15];
        for (op, &l) in s.ops.iter_mut().zip(defaults.iter()) {
            op.level.set_immediate(l);
        }
        s
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.base_frequency = freq;
        self.update_op_frequencies();
    }

    pub fn set_algorithm(&mut self, idx: u8) {
        self.algorithm_idx = idx.min(ALGORITHMS.len() as u8 - 1);
    }

    pub fn set_feedback(&mut self, v: f32) {
        self.feedback.set_target(v.clamp(0.0, 1.0));
    }

    pub fn set_fm_index(&mut self, v: f32) {
        self.fm_index = v.clamp(0.0, 10.0);
    }

    pub fn set_op_ratio(&mut self, op: usize, v: f32) {
        if op < NUM_OPS {
            // 0.25 .. 16 matches DX7-era ratio range.
            self.ops[op].ratio.set_target(v.clamp(0.25, 16.0));
        }
    }

    pub fn set_op_level(&mut self, op: usize, v: f32) {
        if op < NUM_OPS {
            self.ops[op].level.set_target(v.clamp(0.0, 1.0));
        }
    }

    pub fn reset(&mut self) {
        for op in &mut self.ops {
            op.reset();
        }
    }

    fn update_op_frequencies(&mut self) {
        for op in &mut self.ops {
            let r = op.ratio.value();
            op.phase_inc = (self.base_frequency * r) / self.sample_rate;
        }
    }

    /// Generates one output sample.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let schedule = &ALGORITHM_SCHEDULES[self.algorithm_idx as usize];
        let fb = self.feedback.process();
        let fm_idx = self.fm_index;

        // Refresh per-op phase increments in case ratio smoothers moved.
        for op in &mut self.ops {
            let r = op.ratio.process();
            op.phase_inc = (self.base_frequency * r) / self.sample_rate;
            // Advance level smoother too so carrier gain tracks target.
            op.level.process();
        }

        // Evaluate OP6 → OP1 (index 5..=0). Under our DAG convention,
        // op i can only be modulated by ops j where j > i, so by the
        // time we reach op i, all its modulation inputs are already
        // computed for this sample.
        let mut op_out = [0.0_f32; NUM_OPS];

        for i in (0..NUM_OPS).rev() {
            let route = &schedule.mod_routes[i];
            let mut pm = 0.0_f32;

            if route.self_feedback {
                // Self-feedback — use last sample's output of this op,
                // scaled by global feedback.
                pm += self.ops[i].prev_output * fb;
            }
            for input_idx in 0..route.input_count {
                let j = route.inputs[input_idx];
                // Normal: j > i guaranteed by convention.
                pm += op_out[j] * self.ops[j].level.value();
            }

            // Scale modulation input by the global FM index. All
            // modulator contributions pass through this knob so users
            // get one intuitive "depth" control across the engine.
            let phase = self.ops[i].phase + pm * fm_idx;
            let sample = sine_normalized(phase);

            self.ops[i].prev_output = sample;
            op_out[i] = sample;

            self.ops[i].phase += self.ops[i].phase_inc;
            if self.ops[i].phase >= 1.0 {
                self.ops[i].phase -= self.ops[i].phase.floor();
            }
        }

        // Sum carriers. No normalization by carrier count — users tune
        // per-op levels to balance. Multi-carrier algorithms will be
        // louder by design; that's honest output, not a bug.
        let mut out = 0.0_f32;
        for carrier_idx in 0..schedule.carrier_count {
            let i = schedule.carriers[carrier_idx];
            out += op_out[i] * self.ops[i].level.value();
        }
        out
    }
}

impl Engine for FmOscillator {
    fn set_parameter(&mut self, id: ParameterId, value: f32) {
        // Note: FmIndex is intentionally not handled here. The voice
        // owns the static FmIndex base smoother and pushes the
        // (base + env + mod) sum into the oscillator per sample via
        // `set_fm_index` — going through `set_parameter` instead would
        // bypass that smoother and break per-voice envelope decoupling.
        match id {
            ParameterId::Algorithm => {
                // value is a raw algorithm index (0..ALGORITHMS.len()-1)
                // — same convention as every other ParameterId. Round to
                // tolerate fractional values from continuous controllers
                // (e.g. a knob → binding.apply ratio mapping).
                let max_idx = (ALGORITHMS.len() - 1) as f32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = value.round().clamp(0.0, max_idx) as u8;
                self.set_algorithm(idx);
            }
            ParameterId::FmFeedback => self.set_feedback(value),
            ParameterId::Op1Ratio => self.set_op_ratio(0, value),
            ParameterId::Op2Ratio => self.set_op_ratio(1, value),
            ParameterId::Op3Ratio => self.set_op_ratio(2, value),
            ParameterId::Op4Ratio => self.set_op_ratio(3, value),
            ParameterId::Op5Ratio => self.set_op_ratio(4, value),
            ParameterId::Op6Ratio => self.set_op_ratio(5, value),
            ParameterId::Op1Level => self.set_op_level(0, value),
            ParameterId::Op2Level => self.set_op_level(1, value),
            ParameterId::Op3Level => self.set_op_level(2, value),
            ParameterId::Op4Level => self.set_op_level(3, value),
            ParameterId::Op5Level => self.set_op_level(4, value),
            ParameterId::Op6Level => self.set_op_level(5, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn silent_when_all_levels_zero() {
        let mut osc = FmOscillator::new(SR);
        osc.set_frequency(440.0);
        for i in 0..NUM_OPS {
            osc.ops[i].level.set_immediate(0.0);
        }
        // Run past smoother transient
        for _ in 0..4800 {
            osc.process();
        }
        let mut peak = 0.0_f32;
        for _ in 0..4800 {
            peak = peak.max(osc.process().abs());
        }
        assert!(peak < 1e-4, "all-zero levels should be silent, peak={peak}");
    }

    #[test]
    fn pure_sine_on_additive_single_op() {
        // Algorithm 8 (ADDITIVE), only OP1 at level 1, rest at 0 —
        // should produce a clean 440 Hz sine.
        let mut osc = FmOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.set_algorithm(7); // ADDITIVE
        osc.ops[0].level.set_immediate(1.0);
        for i in 1..NUM_OPS {
            osc.ops[i].level.set_immediate(0.0);
        }
        osc.ops[0].ratio.set_immediate(1.0);

        // Count zero crossings over 1 s — should be ~880 (one up, one
        // down per cycle × 440 Hz)
        let mut crossings = 0u32;
        let mut prev = 0.0_f32;
        for _ in 0..48000 {
            let s = osc.process();
            assert!((-1.01..=1.01).contains(&s), "pure sine bounded, got {s}");
            if prev <= 0.0 && s > 0.0 {
                crossings += 1;
            }
            prev = s;
        }
        // Allow ±5 tolerance for phase alignment at window edges
        assert!(
            (435..=445).contains(&crossings),
            "expected ~440 crossings, got {crossings}"
        );
    }

    #[test]
    fn all_algorithms_produce_bounded_output() {
        for alg_idx in 0..ALGORITHMS.len() as u8 {
            let mut osc = FmOscillator::new(SR);
            osc.set_frequency(220.0);
            osc.set_algorithm(alg_idx);
            osc.set_fm_index(3.0);
            osc.set_feedback(0.4);
            for i in 0..NUM_OPS {
                osc.ops[i].level.set_immediate(0.6);
            }
            for _ in 0..48000 {
                let s = osc.process();
                assert!(
                    s.is_finite() && s.abs() < 8.0,
                    "alg {} ({}): unbounded output {s}",
                    alg_idx,
                    ALGORITHMS[alg_idx as usize].name
                );
            }
        }
    }

    #[test]
    fn feedback_does_not_diverge() {
        // Feedback at max with heavy modulation — self-feedback must
        // remain stable (prev_output × feedback × fm_index can blow up
        // if not clamped implicitly by the sine's ±1 bound).
        let mut osc = FmOscillator::new(SR);
        osc.set_frequency(220.0);
        osc.set_algorithm(1); // STACK+FB (has OP6 self-feedback)
        osc.set_fm_index(10.0);
        osc.set_feedback(1.0);
        for i in 0..NUM_OPS {
            osc.ops[i].level.set_immediate(1.0);
        }
        for _ in 0..48000 {
            let s = osc.process();
            assert!(s.is_finite(), "feedback NaN'd");
            // Sine output bounded ±1 per op; with max fb the feedback
            // signal is bounded by the sine itself. Pessimistic cap
            // for the full chain.
            assert!(s.abs() < 8.0, "feedback ran away: {s}");
        }
    }

    #[test]
    fn different_algorithms_produce_different_spectra() {
        // STACK (deep FM) should have more high-frequency content
        // than ADDITIVE (single sine carrier). We measure this via
        // zero-crossing count: a bright FM spectrum has many more
        // crossings than a clean fundamental.
        let count = |alg: u8, op_levels: [f32; NUM_OPS]| -> u32 {
            let mut osc = FmOscillator::new(SR);
            osc.set_frequency(220.0);
            osc.set_algorithm(alg);
            osc.set_fm_index(5.0);
            for (i, &l) in op_levels.iter().enumerate() {
                osc.ops[i].level.set_immediate(l);
            }
            // settle
            for _ in 0..4800 {
                osc.process();
            }
            let mut c = 0u32;
            let mut prev = 0.0_f32;
            for _ in 0..48000 {
                let s = osc.process();
                if prev <= 0.0 && s > 0.0 {
                    c += 1;
                }
                prev = s;
            }
            c
        };

        let stack = count(0, [0.7, 0.8, 0.8, 0.8, 0.8, 0.8]);
        let additive = count(7, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

        // STACK's crossings should be well above 220 (the
        // fundamental) because deep FM adds sidebands. ADDITIVE with
        // one op should be near 220.
        assert!(
            (215..=225).contains(&additive),
            "additive single op ≈ 220 crossings, got {additive}"
        );
        assert!(
            stack > additive + 50,
            "STACK should have more crossings than ADDITIVE: stack={stack}, additive={additive}"
        );
    }

    #[test]
    fn ratio_shifts_frequency_additive() {
        // In ADDITIVE mode with only OP2 at level 1 and ratio 2.0,
        // output should be a 880 Hz sine (~1760 crossings/s).
        let mut osc = FmOscillator::new(SR);
        osc.set_frequency(440.0);
        osc.set_algorithm(7); // ADDITIVE
        for i in 0..NUM_OPS {
            osc.ops[i].level.set_immediate(0.0);
        }
        osc.ops[1].level.set_immediate(1.0);
        osc.ops[1].ratio.set_immediate(2.0);
        osc.set_frequency(440.0); // recompute phase_inc

        let mut c = 0u32;
        let mut prev = 0.0_f32;
        for _ in 0..48000 {
            let s = osc.process();
            if prev <= 0.0 && s > 0.0 {
                c += 1;
            }
            prev = s;
        }
        assert!(
            (875..=885).contains(&c),
            "OP2 at ratio 2 = ~880 Hz, got {c} crossings"
        );
    }

    #[test]
    fn routing_schedules_match_algorithm_masks() {
        for (alg_idx, algorithm) in ALGORITHMS.iter().enumerate() {
            let schedule = &ALGORITHM_SCHEDULES[alg_idx];

            for i in 0..NUM_OPS {
                let route = &schedule.mod_routes[i];
                let mut mask = if route.self_feedback { 1u8 << i } else { 0 };

                for input_idx in 0..route.input_count {
                    let j = route.inputs[input_idx];
                    assert_ne!(j, i, "self feedback should use the bool path");
                    mask |= 1u8 << j;
                }

                assert_eq!(
                    mask, algorithm.mod_into[i],
                    "algorithm {} ({}) op {} schedule drifted",
                    alg_idx, algorithm.name, i
                );
            }

            let mut carrier_mask = 0_u8;
            for carrier_idx in 0..schedule.carrier_count {
                carrier_mask |= 1u8 << schedule.carriers[carrier_idx];
            }

            assert_eq!(
                carrier_mask, algorithm.carriers,
                "algorithm {} ({}) carrier schedule drifted",
                alg_idx, algorithm.name
            );
        }
    }
}
