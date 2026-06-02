// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Step sequencer for rhythmic modulation patterns.
//!
//! 1-8 steps with configurable trigger source (note-on or beat division).
//! Ported from Formfactor with normalized f32 values instead of u8 CC.

use serde::{Deserialize, Serialize};

const MAX_STEPS: usize = 8;

/// How the sequencer advances to the next step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StepTrigger {
    /// Advance on every note-on event.
    #[default]
    NoteOn,
    /// Advance every beat (quarter note).
    Beat,
    /// Advance every half beat (eighth note).
    HalfBeat,
    /// Advance every quarter beat (sixteenth note).
    QuarterBeat,
}

/// A step sequencer with 1-8 steps.
///
/// Each step holds a normalized value (0.0-1.0). The sequencer
/// advances on note events or beat divisions.
pub struct StepSequencer {
    step_values: [f32; MAX_STEPS],
    num_steps: usize,
    current_step: usize,
    trigger: StepTrigger,
    last_beat_pos: f64,
    initialized: bool,
}

impl StepSequencer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Default: ascending ramp
            step_values: [0.0, 0.14, 0.29, 0.43, 0.57, 0.71, 0.86, 1.0],
            num_steps: 4,
            current_step: 0,
            trigger: StepTrigger::Beat,
            last_beat_pos: 0.0,
            initialized: false,
        }
    }

    pub fn reset(&mut self) {
        self.current_step = 0;
        self.last_beat_pos = 0.0;
        self.initialized = false;
    }

    /// Returns the current step's value (0.0-1.0).
    #[must_use]
    pub fn current_value(&self) -> f32 {
        self.step_values[self.current_step]
    }

    #[must_use]
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn set_num_steps(&mut self, n: usize) {
        self.num_steps = n.clamp(1, MAX_STEPS);
        if self.current_step >= self.num_steps {
            self.current_step = 0;
        }
    }

    pub fn set_step_value(&mut self, index: usize, value: f32) {
        if index < MAX_STEPS {
            self.step_values[index] = value.clamp(0.0, 1.0);
        }
    }

    #[must_use]
    pub fn get_step_value(&self, index: usize) -> f32 {
        if index < MAX_STEPS {
            self.step_values[index]
        } else {
            0.0
        }
    }

    pub fn set_trigger(&mut self, trigger: StepTrigger) {
        self.trigger = trigger;
    }

    /// Check if a note-on should advance the sequencer. Returns true if stepped.
    pub fn check_note_trigger(&mut self) -> bool {
        if self.trigger != StepTrigger::NoteOn {
            return false;
        }
        self.advance();
        true
    }

    /// Check if the beat position should advance the sequencer. Returns true if stepped.
    pub fn check_beat_trigger(&mut self, beat_pos: f64) -> bool {
        if self.trigger == StepTrigger::NoteOn {
            return false;
        }

        if !self.initialized {
            self.last_beat_pos = beat_pos;
            self.initialized = true;
            return false;
        }

        let division = match self.trigger {
            StepTrigger::Beat => 1.0,
            StepTrigger::HalfBeat => 0.5,
            StepTrigger::QuarterBeat => 0.25,
            StepTrigger::NoteOn => return false,
        };

        let prev_div = (self.last_beat_pos / division).floor();
        let curr_div = (beat_pos / division).floor();
        self.last_beat_pos = beat_pos;

        if curr_div > prev_div {
            self.advance();
            return true;
        }

        false
    }

    fn advance(&mut self) {
        self.current_step = (self.current_step + 1) % self.num_steps;
    }
}

impl Default for StepSequencer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_trigger_advances() {
        let mut seq = StepSequencer::new();
        seq.set_num_steps(4);
        seq.set_trigger(StepTrigger::NoteOn);

        assert_eq!(seq.current_step(), 0);
        seq.check_note_trigger();
        assert_eq!(seq.current_step(), 1);
        seq.check_note_trigger();
        assert_eq!(seq.current_step(), 2);
    }

    #[test]
    fn wraps_at_num_steps() {
        let mut seq = StepSequencer::new();
        seq.set_num_steps(3);
        seq.set_trigger(StepTrigger::NoteOn);

        for _ in 0..3 {
            seq.check_note_trigger();
        }
        assert_eq!(seq.current_step(), 0); // wrapped
    }

    #[test]
    fn beat_trigger() {
        let mut seq = StepSequencer::new();
        seq.set_num_steps(4);
        seq.set_trigger(StepTrigger::Beat);

        seq.check_beat_trigger(0.0); // initialize
        assert_eq!(seq.current_step(), 0);

        seq.check_beat_trigger(0.5);
        assert_eq!(seq.current_step(), 0); // not yet a full beat

        seq.check_beat_trigger(1.0);
        assert_eq!(seq.current_step(), 1); // crossed beat boundary

        seq.check_beat_trigger(2.0);
        assert_eq!(seq.current_step(), 2);
    }

    #[test]
    fn half_beat_trigger() {
        let mut seq = StepSequencer::new();
        seq.set_num_steps(8);
        seq.set_trigger(StepTrigger::HalfBeat);

        seq.check_beat_trigger(0.0);
        seq.check_beat_trigger(0.5); // half beat
        assert_eq!(seq.current_step(), 1);
        seq.check_beat_trigger(1.0); // next half beat
        assert_eq!(seq.current_step(), 2);
    }

    #[test]
    fn step_values() {
        let mut seq = StepSequencer::new();
        seq.set_step_value(0, 0.25);
        seq.set_step_value(1, 0.75);
        assert!((seq.get_step_value(0) - 0.25).abs() < f32::EPSILON);
        assert!((seq.get_step_value(1) - 0.75).abs() < f32::EPSILON);
    }
}
