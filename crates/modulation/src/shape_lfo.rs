// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Shape-based LFO for modulation.
//!
//! Uses transition shapes as waveforms, with tempo-sync or free-running
//! modes. Adapted from Formfactor's `ShapeLfoState`.

use serde::{Deserialize, Serialize};

use crate::shapes::TransitionShape;

/// LFO operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LfoMode {
    /// Free-running loop — phase cycles continuously.
    #[default]
    Loop,
    /// One-shot — triggered on note, plays once then holds at end.
    Trig,
}

/// LFO that uses transition shapes as waveforms.
///
/// In `Loop` mode, the phase cycles from 0→1 continuously.
/// In `Trig` mode, the phase runs once from 0→1 on trigger, then holds.
///
/// The shape function maps phase (0→1) to output (0→1). For bipolar
/// modulation, the downstream routing layer remaps 0→1 to -1→+1.
pub struct ShapeLfo {
    shape: TransitionShape,
    phase: f32,
    rate_hz: f32,
    tempo_sync: bool,
    /// Rate in beats when tempo-synced (e.g. 4.0 = one cycle per 4 beats).
    rate_beats: f32,
    mode: LfoMode,
    value: f32,
    triggered: bool,
    finished: bool,
    sample_rate: f32,
}

impl ShapeLfo {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            shape: TransitionShape::SCurveSmooth,
            phase: 0.0,
            rate_hz: 1.0,
            tempo_sync: false,
            rate_beats: 4.0,
            // Trig by default: one cycle per note_on, then held. Loop mode
            // continuously cycles the LFO shape, which — when routed
            // through the engine's per-block param override (see
            // part.rs apply_modulation_offset) — produces audible
            // discontinuities at phase-wrap where the value steps
            // from the end of one cycle back to the start of the next.
            // Trig avoids the wrap entirely; a musician who wants Loop
            // can opt in from the MOD page.
            mode: LfoMode::Trig,
            value: 0.0,
            triggered: false,
            finished: false,
            sample_rate,
        }
    }

    pub fn set_shape(&mut self, shape: TransitionShape) {
        self.shape = shape;
    }

    pub fn set_rate_hz(&mut self, hz: f32) {
        self.rate_hz = hz.max(0.01);
    }

    pub fn set_rate_beats(&mut self, beats: f32) {
        self.rate_beats = beats.max(0.25);
    }

    pub fn set_tempo_sync(&mut self, sync: bool) {
        self.tempo_sync = sync;
    }

    pub fn set_mode(&mut self, mode: LfoMode) {
        self.mode = mode;
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// Triggers the LFO (resets phase). Used in Trig mode on note-on.
    pub fn trigger(&mut self) {
        self.phase = 0.0;
        self.triggered = true;
        self.finished = false;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.value = 0.0;
        self.triggered = false;
        self.finished = false;
    }

    /// Current output value (0.0-1.0).
    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Advances the LFO by one buffer's worth of samples in free-running mode.
    ///
    /// Call once per audio buffer. `num_samples` is the buffer size.
    pub fn advance_free(&mut self, num_samples: usize) {
        if self.tempo_sync {
            return; // use advance_tempo instead
        }

        if self.mode == LfoMode::Trig && self.finished {
            return; // one-shot completed
        }

        let phase_inc = self.rate_hz * num_samples as f32 / self.sample_rate;
        self.phase += phase_inc;

        match self.mode {
            LfoMode::Loop => {
                if self.phase >= 1.0 {
                    self.phase -= self.phase.floor();
                }
            }
            LfoMode::Trig => {
                if self.phase >= 1.0 {
                    self.phase = 1.0;
                    self.finished = true;
                }
            }
        }

        self.value = self.shape.shape_function(self.phase);
    }

    /// Advances the LFO based on beat position (for tempo-synced mode).
    ///
    /// Call once per audio buffer with the current beat position from transport.
    pub fn advance_tempo(&mut self, beat_pos: f64) {
        if !self.tempo_sync {
            return; // use advance_free instead
        }

        if self.mode == LfoMode::Trig && self.finished {
            return;
        }

        // Phase = fractional position within the cycle
        let cycle_pos = beat_pos as f32 / self.rate_beats;
        self.phase = match self.mode {
            LfoMode::Loop => cycle_pos.fract(),
            LfoMode::Trig => {
                if !self.triggered {
                    return;
                }
                let p = cycle_pos.fract();
                if p < self.phase && self.phase > 0.9 {
                    // Wrapped — one-shot is done
                    self.finished = true;
                    1.0
                } else {
                    p
                }
            }
        };

        self.value = self.shape.shape_function(self.phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_running_cycles() {
        let mut lfo = ShapeLfo::new(48000.0);
        lfo.set_shape(TransitionShape::LinearUp);
        lfo.set_rate_hz(1.0);
        lfo.set_mode(LfoMode::Loop);

        // Advance 48000 samples = 1 full cycle at 1 Hz
        // Do it in 100-sample chunks
        for _ in 0..480 {
            lfo.advance_free(100);
        }
        // Should be back near 0 (wrapped)
        assert!(lfo.value() < 0.1, "should wrap: value={}", lfo.value());
    }

    #[test]
    fn output_bounded() {
        let mut lfo = ShapeLfo::new(48000.0);
        lfo.set_rate_hz(5.0);

        for shape in [
            TransitionShape::SCurveSmooth,
            TransitionShape::BounceOut,
            TransitionShape::ChaosHeavy,
        ] {
            lfo.set_shape(shape);
            lfo.reset();
            for _ in 0..1000 {
                lfo.advance_free(48);
                assert!(
                    (0.0..=1.0).contains(&lfo.value()),
                    "{shape:?}: value {} out of range",
                    lfo.value()
                );
            }
        }
    }

    #[test]
    fn trig_mode_plays_once() {
        let mut lfo = ShapeLfo::new(48000.0);
        lfo.set_shape(TransitionShape::LinearUp);
        lfo.set_rate_hz(10.0); // fast
        lfo.set_mode(LfoMode::Trig);
        lfo.trigger();

        // Advance well past one cycle
        for _ in 0..100 {
            lfo.advance_free(480);
        }

        // Should hold at 1.0 (end of one-shot)
        assert!(
            (lfo.value() - 1.0).abs() < 0.01,
            "trig should hold at end: {}",
            lfo.value()
        );
    }

    #[test]
    fn shape_affects_output() {
        let mut lfo_lin = ShapeLfo::new(48000.0);
        lfo_lin.set_shape(TransitionShape::LinearUp);
        lfo_lin.set_rate_hz(1.0);

        let mut lfo_exp = ShapeLfo::new(48000.0);
        lfo_exp.set_shape(TransitionShape::ExpAttack);
        lfo_exp.set_rate_hz(1.0);

        // Advance both to the same phase (quarter cycle)
        for _ in 0..120 {
            lfo_lin.advance_free(100);
            lfo_exp.advance_free(100);
        }

        // ExpAttack should be below linear at the midpoint
        assert!(
            lfo_exp.value() < lfo_lin.value(),
            "exp should lag linear: exp={}, lin={}",
            lfo_exp.value(),
            lfo_lin.value()
        );
    }
}
