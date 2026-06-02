// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Modulation engine for Brume.
//!
//! Provides modulation sources (shape LFOs, step sequencers) and a
//! routing layer that connects them to voice and global parameters.
//! Transition shapes serve as both LFO waveforms and response curves.

pub mod ring_buffer;
pub mod routing;
pub mod shape_lfo;
pub mod shapes;
pub mod step_sequencer;

pub use ring_buffer::RingBuffer;
pub use routing::{ModSource, ModulationAssignment, ModulationRouter};
pub use shape_lfo::{LfoMode, ShapeLfo};
pub use shapes::{ShapeCategory, ShapeInterpolator, TransitionShape};
pub use step_sequencer::{StepSequencer, StepTrigger};
