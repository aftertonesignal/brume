// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Bipolar depth slider for the MOD page's ROUTING table. Same
//! input handling as the unipolar bars but draws a fill that grows
//! from the bar's center toward the current depth value — so a
//! depth of -1 fills the left half, +1 the right, 0 leaves an
//! empty rail.
//!
//! Emits `Message::SetAssignmentDepth { part, idx, ratio }` where
//! `ratio` is the 0..1 cursor position. The handler maps that onto
//! a depth in -1..1 via `ratio * 2 - 1`.

use iced::widget::canvas::{self, Event, Frame, Geometry, Program, event::Status};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse, touch};

use crate::Message;

const RAIL_HEIGHT: f32 = 14.0;

pub struct AssignDepthBar {
    /// Depth in [-1, 1].
    pub depth: f32,
    pub color: Color,
    pub part: u8,
    pub idx: u8,
}

impl Program<Message, Theme, Renderer> for AssignDepthBar {
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
        let emit = |ratio: f32| Message::SetAssignmentDepth {
            part: self.part,
            idx: self.idx,
            ratio,
        };
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    *dragging = true;
                    let ratio = (pos.x / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some(emit(ratio)));
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                if let Some(pos) = cursor.position() {
                    let ratio = ((pos.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some(emit(ratio)));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *dragging = false;
            }
            Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                if point_in(position, bounds) {
                    *dragging = true;
                    let ratio = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                    return (Status::Captured, Some(emit(ratio)));
                }
            }
            Event::Touch(touch::Event::FingerMoved { position, .. }) if *dragging => {
                let ratio = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                return (Status::Captured, Some(emit(ratio)));
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

        // Empty rail across the full width — same muted accent as
        // the unipolar bars so the family reads consistently.
        frame.fill_rectangle(
            Point::new(0.0, rail_top),
            Size::new(bounds.width, RAIL_HEIGHT),
            Color {
                a: 0.18,
                ..self.color
            },
        );

        let center_x = bounds.width / 2.0;
        let depth = self.depth.clamp(-1.0, 1.0);
        if depth.abs() > 1e-3 {
            let half_w = bounds.width / 2.0;
            let span = depth.abs() * half_w;
            let (x, w) = if depth >= 0.0 {
                (center_x, span)
            } else {
                (center_x - span, span)
            };
            frame.fill_rectangle(
                Point::new(x, rail_top),
                Size::new(w, RAIL_HEIGHT),
                self.color,
            );
        }

        // Center tick — 1 px vertical line at the bar's midpoint so
        // a zero depth still has a visual anchor.
        frame.fill_rectangle(
            Point::new(center_x - 0.5, rail_top - 2.0),
            Size::new(1.0, RAIL_HEIGHT + 4.0),
            Color {
                a: 0.45,
                ..Color::WHITE
            },
        );

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

pub fn assign_depth_bar(
    depth: f32,
    color: Color,
    part: u8,
    idx: u8,
) -> canvas::Canvas<AssignDepthBar, Message, Theme, Renderer> {
    canvas::Canvas::new(AssignDepthBar {
        depth,
        color,
        part,
        idx,
    })
}
