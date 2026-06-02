// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Transition shapes for modulation curves and parameter interpolation.
//!
//! 24 easing curves ported from Formfactor. Used as LFO waveforms,
//! step sequencer transitions, touch zone response curves, and
//! modulation assignment response shaping.

use serde::{Deserialize, Serialize};

/// Shape categories for UI grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeCategory {
    Linear,
    Exponential,
    Circular,
    Bloom,
    SCurve,
    Timing,
    Bounce,
    Chaos,
    Direct,
}

/// 24 transition shapes with creative names.
///
/// Each shape is a pure function: `shape_function(t) -> f32` where
/// input and output are both in the range [0.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransitionShape {
    // Stochastic (default — musically interesting)
    #[default]
    ChaosHeavy,

    // Linear (2)
    LinearUp,
    LinearDown,

    // Exponential (4)
    ExpAttack,
    ExpDecay,
    LogAttack,
    LogDecay,

    // Circular (4)
    CircularIn,
    CircularOut,
    CircularInOut,
    CircularOutIn,

    // Bloom (2)
    BloomIn,
    BloomOut,

    // S-Curves (2)
    SCurveSmooth,
    SCurveSharp,

    // Timing-biased (4)
    SlowStart,
    SlowEnd,
    FastStart,
    FastEnd,

    // Bounce (2)
    BounceIn,
    BounceOut,

    // Stochastic (2)
    ChaosLight,
    Random,

    // Instant (1)
    DC,
}

impl TransitionShape {
    /// Interpolates between `start` and `end` using this shape at position `t`.
    #[must_use]
    pub fn interpolate(&self, t: f32, start: f32, end: f32) -> f32 {
        let shaped_t = self.shape_function(t);
        start + (end - start) * shaped_t
    }

    /// The core shaping function. Input and output are both [0.0, 1.0].
    #[must_use]
    pub fn shape_function(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);

        match self {
            // Linear
            Self::LinearUp => t,
            Self::LinearDown => 1.0 - t,

            // Exponential
            Self::ExpAttack => t * t,
            Self::ExpDecay => 1.0 - (1.0 - t) * (1.0 - t),
            Self::LogAttack => t.sqrt(),
            Self::LogDecay => 1.0 - (1.0 - t).sqrt(),

            // Circular
            Self::CircularIn => 1.0 - (1.0 - t * t).sqrt(),
            Self::CircularOut => (1.0 - (1.0 - t) * (1.0 - t)).sqrt(),
            Self::CircularInOut => {
                if t < 0.5 {
                    (1.0 - (1.0 - 4.0 * t * t).sqrt()) * 0.5
                } else {
                    ((1.0 - (2.0 * t - 2.0).powi(2)).sqrt() + 1.0) * 0.5
                }
            }
            Self::CircularOutIn => {
                if t < 0.5 {
                    (1.0 - (1.0 - t) * (1.0 - t)).sqrt() * 0.5
                } else {
                    0.5 + (1.0 - (1.0 - (t - 0.5) * 2.0).powi(2)).sqrt() * 0.5
                }
            }

            // Bloom (cubic ease — same formula as SlowStart, different semantic)
            #[allow(clippy::match_same_arms)]
            Self::BloomIn => t * t * t,
            Self::BloomOut => {
                let t = t - 1.0;
                t * t * t + 1.0
            }

            // S-Curves
            Self::SCurveSmooth => t * t * (3.0 - 2.0 * t),
            Self::SCurveSharp => t * t * t * (t * (t * 6.0 - 15.0) + 10.0),

            // Timing-biased
            Self::SlowStart => t * t * t,
            Self::SlowEnd => (t * std::f32::consts::FRAC_PI_2).sin(),
            Self::FastStart => 1.0 - 2.0_f32.powf(-10.0 * t),
            Self::FastEnd => t.powi(4),

            // Bounce
            Self::BounceIn => 1.0 - bounce_ease_out(1.0 - t),
            Self::BounceOut => bounce_ease_out(t),

            // Stochastic
            Self::ChaosLight => t + 0.1 * (t * 47.0).sin(),
            Self::ChaosHeavy => t + 0.25 * (t * 31.0).sin() * (t * 17.0).cos(),
            Self::Random => {
                #[allow(clippy::excessive_precision)]
                let hash = ((t * 12345.6789).sin() * 43758.5453).fract();
                t * 0.7 + hash * 0.3
            }

            // Instant
            Self::DC => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
        .clamp(0.0, 1.0)
    }

    /// Returns the UI grouping category for this shape.
    #[must_use]
    pub fn category(&self) -> ShapeCategory {
        match self {
            Self::LinearUp | Self::LinearDown => ShapeCategory::Linear,
            Self::ExpAttack | Self::ExpDecay | Self::LogAttack | Self::LogDecay => {
                ShapeCategory::Exponential
            }
            Self::CircularIn | Self::CircularOut | Self::CircularInOut | Self::CircularOutIn => {
                ShapeCategory::Circular
            }
            Self::BloomIn | Self::BloomOut => ShapeCategory::Bloom,
            Self::SCurveSmooth | Self::SCurveSharp => ShapeCategory::SCurve,
            Self::SlowStart | Self::SlowEnd | Self::FastStart | Self::FastEnd => {
                ShapeCategory::Timing
            }
            Self::BounceIn | Self::BounceOut => ShapeCategory::Bounce,
            Self::ChaosLight | Self::ChaosHeavy | Self::Random => ShapeCategory::Chaos,
            Self::DC => ShapeCategory::Direct,
        }
    }

    /// Display name for the UI.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChaosHeavy => "Caffeinated",
            Self::LinearUp => "Ramp Up",
            Self::LinearDown => "Ramp Down",
            Self::ExpAttack => "Slow Burn",
            Self::ExpDecay => "Freefall",
            Self::LogAttack => "Quick Draw",
            Self::LogDecay => "Long Tail",
            Self::CircularIn => "Molasses",
            Self::CircularOut => "Catapult",
            Self::CircularInOut => "Switchback",
            Self::CircularOutIn => "Foothill",
            Self::BloomIn => "Unfurl",
            Self::BloomOut => "Chunk Wedge",
            Self::SCurveSmooth => "Silk",
            Self::SCurveSharp => "Whiplash",
            Self::SlowStart => "Reluctant",
            Self::SlowEnd => "Lazy Landing",
            Self::FastStart => "Eager Beaver",
            Self::FastEnd => "Coasting",
            Self::BounceIn => "Trampoline",
            Self::BounceOut => "Overshoot",
            Self::ChaosLight => "Jitters",
            Self::Random => "Dice Roll",
            Self::DC => "Guillotine",
        }
    }
}

fn bounce_ease_out(t: f32) -> f32 {
    if t < 1.0 / 2.75 {
        7.5625 * t * t
    } else if t < 2.0 / 2.75 {
        let t = t - 1.5 / 2.75;
        7.5625 * t * t + 0.75
    } else if t < 2.5 / 2.75 {
        let t = t - 2.25 / 2.75;
        7.5625 * t * t + 0.9375
    } else {
        let t = t - 2.625 / 2.75;
        7.5625 * t * t + 0.984_375
    }
}

/// Interpolator that manages smooth transitions between values using shapes.
#[derive(Debug, Clone)]
pub struct ShapeInterpolator {
    shape: TransitionShape,
    start_value: f32,
    end_value: f32,
    duration_samples: u32,
    position: u32,
    active: bool,
    current_value: f32,
}

impl Default for ShapeInterpolator {
    fn default() -> Self {
        Self {
            shape: TransitionShape::LinearUp,
            start_value: 0.0,
            end_value: 0.0,
            duration_samples: 1,
            position: 0,
            active: false,
            current_value: 0.0,
        }
    }
}

impl ShapeInterpolator {
    #[must_use]
    pub fn new(shape: TransitionShape) -> Self {
        Self {
            shape,
            ..Default::default()
        }
    }

    /// Starts a transition from `from` to `to` over `duration_samples`.
    pub fn start_transition(&mut self, from: f32, to: f32, duration_samples: u32) {
        self.start_value = from;
        self.end_value = to;
        self.duration_samples = duration_samples.max(1);
        self.position = 0;
        self.active = true;
        self.current_value = from;
    }

    /// Starts a transition using a specific shape.
    pub fn start_transition_with_shape(
        &mut self,
        from: f32,
        to: f32,
        duration_samples: u32,
        shape: TransitionShape,
    ) {
        self.shape = shape;
        self.start_transition(from, to, duration_samples);
    }

    /// Advances one sample and returns the current interpolated value.
    pub fn process(&mut self) -> f32 {
        if !self.active {
            return self.current_value;
        }

        let t = self.position as f32 / self.duration_samples as f32;
        self.current_value = self.shape.interpolate(t, self.start_value, self.end_value);

        self.position += 1;
        if self.position >= self.duration_samples {
            self.active = false;
            self.current_value = self.end_value;
        }

        self.current_value
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[must_use]
    pub fn value(&self) -> f32 {
        self.current_value
    }

    pub fn set_value(&mut self, value: f32) {
        self.current_value = value;
        self.end_value = value;
        self.active = false;
    }

    pub fn set_shape(&mut self, shape: TransitionShape) {
        self.shape = shape;
    }

    #[must_use]
    pub fn shape(&self) -> TransitionShape {
        self.shape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_shapes_bounded() {
        let shapes = [
            TransitionShape::LinearUp,
            TransitionShape::LinearDown,
            TransitionShape::ExpAttack,
            TransitionShape::ExpDecay,
            TransitionShape::LogAttack,
            TransitionShape::LogDecay,
            TransitionShape::CircularIn,
            TransitionShape::CircularOut,
            TransitionShape::CircularInOut,
            TransitionShape::CircularOutIn,
            TransitionShape::BloomIn,
            TransitionShape::BloomOut,
            TransitionShape::SCurveSmooth,
            TransitionShape::SCurveSharp,
            TransitionShape::SlowStart,
            TransitionShape::SlowEnd,
            TransitionShape::FastStart,
            TransitionShape::FastEnd,
            TransitionShape::BounceIn,
            TransitionShape::BounceOut,
            TransitionShape::ChaosLight,
            TransitionShape::ChaosHeavy,
            TransitionShape::Random,
            TransitionShape::DC,
        ];

        for shape in shapes {
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let result = shape.shape_function(t);
                assert!(
                    (0.0..=1.0).contains(&result),
                    "{shape:?} at t={t} gave {result}"
                );
            }
        }
    }

    #[test]
    fn linear_up_identity() {
        let s = TransitionShape::LinearUp;
        assert!((s.shape_function(0.0) - 0.0).abs() < 0.001);
        assert!((s.shape_function(0.5) - 0.5).abs() < 0.001);
        assert!((s.shape_function(1.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn interpolator_linear_convergence() {
        let mut interp = ShapeInterpolator::new(TransitionShape::LinearUp);
        interp.start_transition(0.0, 1.0, 100);
        assert!(interp.is_active());

        for _ in 0..50 {
            interp.process();
        }
        assert!((interp.value() - 0.5).abs() < 0.02);

        for _ in 0..50 {
            interp.process();
        }
        assert!(!interp.is_active());
        assert!((interp.value() - 1.0).abs() < 0.001);
    }

    #[test]
    fn interpolator_set_value_cancels() {
        let mut interp = ShapeInterpolator::default();
        interp.start_transition(0.0, 1.0, 100);
        interp.set_value(0.75);
        assert!(!interp.is_active());
        assert!((interp.value() - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn dc_is_step_function() {
        let s = TransitionShape::DC;
        assert!((s.shape_function(0.0) - 0.0).abs() < 0.001);
        assert!((s.shape_function(0.5) - 0.0).abs() < 0.001);
        assert!((s.shape_function(0.99) - 0.0).abs() < 0.001);
        assert!((s.shape_function(1.0) - 1.0).abs() < 0.001);
    }
}
