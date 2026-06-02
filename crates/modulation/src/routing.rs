// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Modulation routing — connects sources to destinations.
//!
//! The routing table is a flat list of assignments, each mapping a
//! modulation source to a parameter destination with depth and shape.

use brume_common::ParameterId;
use serde::{Deserialize, Serialize};

use crate::shape_lfo::ShapeLfo;
use crate::shapes::TransitionShape;
use crate::step_sequencer::StepSequencer;

/// Identifies a modulation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModSource {
    Lfo1,
    Lfo2,
    Seq1,
    Seq2,
    // Future: FilterEnv, AmpEnv, Analysis, TouchZone
}

/// A single modulation assignment: source → destination with depth and shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationAssignment {
    pub source: ModSource,
    pub destination: ParameterId,
    /// Bipolar depth: -1.0 to 1.0. Negative inverts the modulation.
    pub depth: f32,
    /// Response shape applied to the source signal before scaling.
    pub shape: TransitionShape,
    pub enabled: bool,
}

/// The modulation router: owns sources, evaluates the routing table per buffer.
pub struct ModulationRouter {
    pub lfo1: ShapeLfo,
    pub lfo2: ShapeLfo,
    pub seq1: StepSequencer,
    pub seq2: StepSequencer,
    assignments: Vec<ModulationAssignment>,
    /// Accumulated modulation offsets per destination, computed per buffer.
    offsets: Vec<(ParameterId, f32)>,
}

impl ModulationRouter {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            lfo1: ShapeLfo::new(sample_rate),
            lfo2: ShapeLfo::new(sample_rate),
            seq1: StepSequencer::new(),
            seq2: StepSequencer::new(),
            assignments: Vec::new(),
            offsets: Vec::new(),
        }
    }

    /// Adds a modulation assignment.
    pub fn add_assignment(&mut self, assignment: ModulationAssignment) {
        self.assignments.push(assignment);
    }

    /// Removes all assignments.
    pub fn clear_assignments(&mut self) {
        self.assignments.clear();
    }

    /// Returns the current assignments.
    #[must_use]
    pub fn assignments(&self) -> &[ModulationAssignment] {
        &self.assignments
    }

    /// Returns a mutable reference to the assignments vector.
    pub fn assignments_mut(&mut self) -> &mut Vec<ModulationAssignment> {
        &mut self.assignments
    }

    /// Advances all modulation sources by one buffer.
    ///
    /// Call once per audio buffer from the engine's process loop.
    pub fn advance(&mut self, num_samples: usize, beat_pos: Option<f64>) {
        // Advance LFOs
        if let Some(bp) = beat_pos {
            self.lfo1.advance_tempo(bp);
            self.lfo2.advance_tempo(bp);
            self.seq1.check_beat_trigger(bp);
            self.seq2.check_beat_trigger(bp);
        }
        self.lfo1.advance_free(num_samples);
        self.lfo2.advance_free(num_samples);
    }

    /// Notifies modulation sources of a note-on event.
    pub fn note_on(&mut self) {
        self.lfo1.trigger();
        self.lfo2.trigger();
        self.seq1.check_note_trigger();
        self.seq2.check_note_trigger();
    }

    /// Evaluates all assignments and returns modulation offsets per destination.
    ///
    /// Each offset should be added to the destination parameter's base value.
    /// The caller is responsible for clamping to the parameter's valid range.
    pub fn evaluate(&mut self) -> &[(ParameterId, f32)] {
        self.offsets.clear();

        for assignment in &self.assignments {
            if !assignment.enabled {
                continue;
            }

            // Read source value (0.0-1.0)
            let raw = match assignment.source {
                ModSource::Lfo1 => self.lfo1.value(),
                ModSource::Lfo2 => self.lfo2.value(),
                ModSource::Seq1 => self.seq1.current_value(),
                ModSource::Seq2 => self.seq2.current_value(),
            };

            // Apply response shape
            let shaped = assignment.shape.shape_function(raw);

            // Center at 0.0 (convert 0-1 unipolar to -0.5..+0.5 bipolar)
            let centered = shaped - 0.5;

            // Scale by depth (-1 to +1)
            let offset = centered * assignment.depth * 2.0;

            self.offsets.push((assignment.destination, offset));
        }

        &self.offsets
    }

    pub fn reset(&mut self) {
        self.lfo1.reset();
        self.lfo2.reset();
        self.seq1.reset();
        self.seq2.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_router_produces_no_offsets() {
        let mut router = ModulationRouter::new(48000.0);
        router.advance(512, None);
        assert!(router.evaluate().is_empty());
    }

    #[test]
    fn lfo_assignment_produces_offset() {
        let mut router = ModulationRouter::new(48000.0);
        router.lfo1.set_shape(TransitionShape::LinearUp);
        router.lfo1.set_rate_hz(1.0);

        router.add_assignment(ModulationAssignment {
            source: ModSource::Lfo1,
            destination: ParameterId::FilterCutoff,
            depth: 1.0,
            shape: TransitionShape::LinearUp,
            enabled: true,
        });

        // Advance partway through the LFO cycle
        for _ in 0..100 {
            router.advance(480, None);
        }

        let offsets = router.evaluate();
        assert_eq!(offsets.len(), 1);
        assert_eq!(offsets[0].0, ParameterId::FilterCutoff);
        // The offset should be non-zero (LFO is partway through its cycle)
        assert!(
            offsets[0].1.abs() > 0.001,
            "offset should be non-zero: {}",
            offsets[0].1
        );
    }

    #[test]
    fn disabled_assignment_skipped() {
        let mut router = ModulationRouter::new(48000.0);
        router.add_assignment(ModulationAssignment {
            source: ModSource::Lfo1,
            destination: ParameterId::FilterCutoff,
            depth: 1.0,
            shape: TransitionShape::LinearUp,
            enabled: false,
        });

        router.advance(512, None);
        assert!(router.evaluate().is_empty());
    }

    #[test]
    fn negative_depth_inverts() {
        let mut router = ModulationRouter::new(48000.0);
        router.lfo1.set_shape(TransitionShape::LinearUp);
        router.lfo1.set_rate_hz(1.0);

        router.add_assignment(ModulationAssignment {
            source: ModSource::Lfo1,
            destination: ParameterId::FilterCutoff,
            depth: 1.0,
            shape: TransitionShape::LinearUp,
            enabled: true,
        });

        // Advance to mid-cycle
        for _ in 0..50 {
            router.advance(480, None);
        }
        let pos_offset = router.evaluate()[0].1;

        // Same but with negative depth
        let mut router_neg = ModulationRouter::new(48000.0);
        router_neg.lfo1.set_shape(TransitionShape::LinearUp);
        router_neg.lfo1.set_rate_hz(1.0);
        router_neg.add_assignment(ModulationAssignment {
            source: ModSource::Lfo1,
            destination: ParameterId::FilterCutoff,
            depth: -1.0,
            shape: TransitionShape::LinearUp,
            enabled: true,
        });

        for _ in 0..50 {
            router_neg.advance(480, None);
        }
        let neg_offset = router_neg.evaluate()[0].1;

        assert!(
            (pos_offset + neg_offset).abs() < 0.01,
            "negative depth should invert: pos={pos_offset}, neg={neg_offset}"
        );
    }
}
