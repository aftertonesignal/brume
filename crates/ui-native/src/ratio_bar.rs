// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Generic 0..1 ratio bar widget. Five sibling bars (parameter
//! sliders, LFO rate, mixer faders, FX params, sequencer steps) all
//! share the same visual — empty rail in the muted accent, filled
//! rectangle plus right-pointing triangular tip in the full accent —
//! and the same touch + mouse drag handling. Only the message they
//! emit on input differs.
//!
//! `RatioBar` parameterises that one differing piece as a closure;
//! the call site supplies whatever per-widget context (part index,
//! `KnobBinding`, FX tab/param indices, …) it needs to construct the
//! right `Message` variant. Five 130-line files collapse to this
//! one.
//!
//! Bipolar bars (`assign_depth_bar`) draw differently and stay
//! separate. The non-interactive scope meter (`scope_bar`) also
//! stays separate — it has no input path.

use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Program, event::Status};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse, touch};

use crate::Message;

const RAIL_HEIGHT: f32 = 14.0;
/// How far the triangular tip cuts back from the filled rail's
/// right edge. The apex sits at the filled-rail value (the
/// "value lands here" marker); the triangle is carved into the
/// right end of the filled rectangle. Stays within `RAIL_HEIGHT`
/// so the tip never overflows above or below the bar.
const TIP_DEPTH: f32 = 8.0;

/// A 0..1 ratio bar. Generic over the closure that converts the
/// current cursor ratio into the `Message` to emit on drag.
pub struct RatioBar<F>
where
    F: Fn(f32) -> Message,
{
    pub ratio: f32,
    pub color: Color,
    pub on_change: F,
}

impl<F> Program<Message, Theme, Renderer> for RatioBar<F>
where
    F: Fn(f32) -> Message,
{
    /// `true` while the user is mid-drag. Lets us keep emitting
    /// updates on cursor motion outside the bar's bounds (so the
    /// user doesn't lose the drag if they wander off the row).
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
        // Both Mouse and Touch input have to be handled. labwc + winit
        // on the CM5 delivers MAGEX touch input as Touch events to
        // canvas widgets (separately from any synthesized Mouse
        // events). The stock iced `slider` widget handles touch
        // internally, but a custom Canvas Program has to deal with
        // both event streams explicitly — without the Touch branch,
        // finger drags fall on the floor and the bar reads as
        // unresponsive on the touchscreen.
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    *dragging = true;
                    let ratio = (pos.x / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some((self.on_change)(ratio)));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                if let Some(pos) = cursor.position() {
                    let ratio = ((pos.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some((self.on_change)(ratio)));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *dragging = false;
            }
            Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                if point_in(position, bounds) {
                    *dragging = true;
                    let ratio = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some((self.on_change)(ratio)));
                }
            }
            Event::Touch(touch::Event::FingerMoved { position, .. }) if *dragging => {
                let ratio = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                return (Status::Captured, Some((self.on_change)(ratio)));
            }
            Event::Touch(touch::Event::FingerLifted { .. })
            | Event::Touch(touch::Event::FingerLost { .. }) => {
                *dragging = false;
            }
            _ => {}
        }
        (Status::Ignored, None)
    }

    fn draw(
        &self,
        _dragging: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());

        let center_y = bounds.height / 2.0;
        let rail_top = center_y - RAIL_HEIGHT / 2.0;
        let filled_w = (self.ratio * bounds.width).clamp(0.0, bounds.width);

        // Empty rail in the muted variant of the accent so the
        // parameter is visible even at value 0.
        frame.fill_rectangle(
            Point::new(0.0, rail_top),
            Size::new(bounds.width, RAIL_HEIGHT),
            Color {
                a: 0.18,
                ..self.color
            },
        );

        // Filled portion: rectangle body to (filled_w - TIP_DEPTH),
        // then a right-pointing triangle whose apex sits at filled_w.
        let body_end = (filled_w - TIP_DEPTH).max(0.0);
        if body_end > 0.0 {
            frame.fill_rectangle(
                Point::new(0.0, rail_top),
                Size::new(body_end, RAIL_HEIGHT),
                self.color,
            );
        }
        if filled_w > 0.0 {
            let rail_bottom = rail_top + RAIL_HEIGHT;
            let tip_path = Path::new(|p| {
                p.move_to(Point::new(body_end, rail_top));
                p.line_to(Point::new(filled_w, center_y));
                p.line_to(Point::new(body_end, rail_bottom));
                p.close();
            });
            frame.fill(&tip_path, self.color);
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _dragging: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

fn point_in(pos: Point, r: Rectangle) -> bool {
    pos.x >= r.x && pos.x <= r.x + r.width && pos.y >= r.y && pos.y <= r.y + r.height
}

/// Build the canvas widget for a ratio bar. `on_change` typically
/// captures the per-widget context (part / lfo / binding / FX
/// indices / step) and returns the appropriate `Message` variant.
pub fn ratio_bar<F>(
    ratio: f32,
    color: Color,
    on_change: F,
) -> canvas::Canvas<RatioBar<F>, Message, Theme, Renderer>
where
    F: Fn(f32) -> Message,
{
    canvas::Canvas::new(RatioBar {
        ratio,
        color,
        on_change,
    })
}
