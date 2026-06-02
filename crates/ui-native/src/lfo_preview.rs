// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! LFO PREVIEW canvas — draws 2 cycles of each LFO's transition
//! shape so the user can see what they're picking before it ever
//! reaches a destination. One trace per LFO, headed by
//! `LFO N · Shape · Rate`.
//!
//! Shape is rendered via `brume_modulation::TransitionShape::shape_function`
//! so the curve here is the same one the engine evaluates per voice.

use brume_modulation::TransitionShape;
use iced::widget::canvas::{self, Frame, Geometry, Path, Program, Stroke, Style, stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};

use crate::Message;

pub struct LfoPreview {
    pub lfo1_shape: TransitionShape,
    pub lfo1_rate_hz: f32,
    pub lfo1_phase: f32,
    pub lfo1_label: &'static str,
    pub lfo1_color: Color,

    pub lfo2_shape: TransitionShape,
    pub lfo2_rate_hz: f32,
    pub lfo2_phase: f32,
    pub lfo2_label: &'static str,
    pub lfo2_color: Color,

    pub label_muted: Color,
    /// Color used for the shape-name segment of the header. Red so
    /// the shape reads as the dominant identifier above the trace,
    /// distinct from the LFO label (mode color) and rate (muted).
    pub shape_name_color: Color,
}

const SAMPLES: usize = 256;
const CYCLES: f32 = 2.0;

impl Program<Message, Theme, Renderer> for LfoPreview {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let half_h = bounds.height / 2.0;

        // LFO 1 in the top half, LFO 2 in the bottom half. Header
        // band reserves ~22 px above each trace for the label.
        let header_h: f32 = 22.0;
        draw_lfo(
            &mut frame,
            self.lfo1_shape,
            self.lfo1_color,
            self.lfo1_label,
            self.lfo1_rate_hz,
            self.lfo1_phase,
            self.label_muted,
            self.shape_name_color,
            Rectangle {
                x: 0.0,
                y: 0.0,
                width: bounds.width,
                height: half_h,
            },
            header_h,
        );
        draw_lfo(
            &mut frame,
            self.lfo2_shape,
            self.lfo2_color,
            self.lfo2_label,
            self.lfo2_rate_hz,
            self.lfo2_phase,
            self.label_muted,
            self.shape_name_color,
            Rectangle {
                x: 0.0,
                y: half_h,
                width: bounds.width,
                height: half_h,
            },
            header_h,
        );

        vec![frame.into_geometry()]
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lfo(
    frame: &mut Frame,
    shape: TransitionShape,
    accent: Color,
    label: &str,
    rate_hz: f32,
    phase: f32,
    label_muted: Color,
    shape_name_color: Color,
    bounds: Rectangle,
    header_h: f32,
) {
    // Header text — `LFO N · Shape · Rate`. Painted on the header
    // band reserved at the top of `bounds`.
    let mut x = bounds.x + 12.0;
    let header_y = bounds.y + 14.0;
    let header_size = 11.0;
    frame.fill_text(canvas::Text {
        content: label.to_string(),
        position: Point::new(x, header_y),
        color: accent,
        size: header_size.into(),
        font: iced::Font::DEFAULT,
        horizontal_alignment: iced::alignment::Horizontal::Left,
        vertical_alignment: iced::alignment::Vertical::Center,
        ..canvas::Text::default()
    });
    x += 60.0;
    frame.fill_text(canvas::Text {
        content: shape_display_name(shape).to_string(),
        position: Point::new(x, header_y),
        color: shape_name_color,
        size: header_size.into(),
        font: iced::Font::DEFAULT,
        horizontal_alignment: iced::alignment::Horizontal::Left,
        vertical_alignment: iced::alignment::Vertical::Center,
        ..canvas::Text::default()
    });
    x += 130.0;
    frame.fill_text(canvas::Text {
        content: format!("{rate_hz:.2} Hz"),
        position: Point::new(x, header_y),
        color: label_muted,
        size: header_size.into(),
        font: iced::Font::MONOSPACE,
        horizontal_alignment: iced::alignment::Horizontal::Left,
        vertical_alignment: iced::alignment::Vertical::Center,
        ..canvas::Text::default()
    });

    // Trace area — below the header band, with 8 px insets on top
    // and bottom so the curve doesn't kiss the lane edges.
    let trace_top = bounds.y + header_h + 8.0;
    let trace_bottom = bounds.y + bounds.height - 8.0;
    let trace_h = (trace_bottom - trace_top).max(0.0);
    if trace_h <= 0.0 || bounds.width <= 0.0 {
        return;
    }

    // Free-running phase scrolls the trace right-to-left at the
    // configured rate so a faster LFO is visibly faster, not just
    // text. Two cycles across the canvas width keeps the shape
    // legible regardless of rate.
    let path = Path::new(|p| {
        for i in 0..=SAMPLES {
            let s = i as f32 / SAMPLES as f32;
            let local_phase = ((s * CYCLES) - phase).rem_euclid(1.0);
            let y_norm = shape.shape_function(local_phase);
            let px = bounds.x + s * bounds.width;
            let py = trace_bottom - y_norm * trace_h;
            if i == 0 {
                p.move_to(Point::new(px, py));
            } else {
                p.line_to(Point::new(px, py));
            }
        }
    });
    frame.stroke(
        &path,
        Stroke {
            style: Style::Solid(accent),
            width: 1.5,
            line_cap: stroke::LineCap::Round,
            line_join: stroke::LineJoin::Round,
            ..Stroke::default()
        },
    );
}

/// Friendly name for a TransitionShape — preview header reads
/// identical to the picker selection.
fn shape_display_name(s: TransitionShape) -> &'static str {
    match s {
        TransitionShape::SCurveSmooth => "Silk",
        TransitionShape::ChaosHeavy => "Caffeinated",
        TransitionShape::LinearUp => "RampUp",
        TransitionShape::LinearDown => "RampDown",
        TransitionShape::ExpAttack => "SlowBurn",
        TransitionShape::ExpDecay => "Freefall",
        TransitionShape::LogAttack => "QuickDraw",
        TransitionShape::LogDecay => "LongTail",
        TransitionShape::CircularIn => "Molasses",
        TransitionShape::CircularOut => "Catapult",
        TransitionShape::BloomIn => "Unfurl",
        TransitionShape::BloomOut => "ChunkWedge",
        TransitionShape::BounceIn => "Trampoline",
        TransitionShape::BounceOut => "Overshoot",
        TransitionShape::ChaosLight => "Jitters",
        TransitionShape::Random => "DiceRoll",
        TransitionShape::SlowStart => "Reluctant",
        TransitionShape::SlowEnd => "LazyLanding",
        TransitionShape::FastStart => "EagerBeaver",
        TransitionShape::FastEnd => "Coasting",
        TransitionShape::CircularInOut => "Switchback",
        TransitionShape::CircularOutIn => "Foothill",
        TransitionShape::SCurveSharp => "Whiplash",
        TransitionShape::DC => "Guillotine",
    }
}

/// Convert a UI shape name (one of `SHAPE_NAMES`) into the engine's
/// `TransitionShape`. Mirrors `engine-runtime`'s `parse_shape`. Falls
/// back to LinearUp on unknown input — should never happen because
/// the picker only emits SHAPE_NAMES values.
pub fn shape_from_name(name: &str) -> TransitionShape {
    match name {
        "Silk" => TransitionShape::SCurveSmooth,
        "Caffeinated" => TransitionShape::ChaosHeavy,
        "RampUp" => TransitionShape::LinearUp,
        "RampDown" => TransitionShape::LinearDown,
        "SlowBurn" => TransitionShape::ExpAttack,
        "Freefall" => TransitionShape::ExpDecay,
        "QuickDraw" => TransitionShape::LogAttack,
        "LongTail" => TransitionShape::LogDecay,
        "Molasses" => TransitionShape::CircularIn,
        "Catapult" => TransitionShape::CircularOut,
        "Unfurl" => TransitionShape::BloomIn,
        "ChunkWedge" => TransitionShape::BloomOut,
        "Trampoline" => TransitionShape::BounceIn,
        "Overshoot" => TransitionShape::BounceOut,
        "Jitters" => TransitionShape::ChaosLight,
        "DiceRoll" => TransitionShape::Random,
        "Reluctant" => TransitionShape::SlowStart,
        "LazyLanding" => TransitionShape::SlowEnd,
        "EagerBeaver" => TransitionShape::FastStart,
        "Coasting" => TransitionShape::FastEnd,
        "Switchback" => TransitionShape::CircularInOut,
        "Foothill" => TransitionShape::CircularOutIn,
        "Whiplash" => TransitionShape::SCurveSharp,
        "Guillotine" => TransitionShape::DC,
        _ => TransitionShape::LinearUp,
    }
}
