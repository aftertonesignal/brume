// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Tiny non-interactive Canvas widget for the SCOPE panel's peak
//! meter. Just paints two filled rectangles — full-width muted rail,
//! plus a `0..peak * width` solid fill in mode color. Reliable
//! regardless of the system's monospace font glyph coverage (the
//! Unicode-block-character version showed as hollow rectangles when
//! the fallback font lacked U+2588).

use iced::widget::canvas::{Event, Frame, Geometry, Program, event::Status};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;

pub struct ScopeBar {
    pub peak: f32,
    pub mode_color: Color,
}

impl Program<Message, Theme, Renderer> for ScopeBar {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        _event: Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
        (Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        // Empty rail: full width, mode color at low alpha so the bar
        // is always visible even at zero peak.
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, bounds.height),
            Color {
                a: 0.18,
                ..self.mode_color
            },
        );
        // Filled portion: 0..peak * width in full mode color.
        let peak = self.peak.clamp(0.0, 1.0);
        let fill_w = peak * bounds.width;
        if fill_w > 0.0 {
            frame.fill_rectangle(
                Point::new(0.0, 0.0),
                Size::new(fill_w, bounds.height),
                self.mode_color,
            );
        }
        vec![frame.into_geometry()]
    }
}
