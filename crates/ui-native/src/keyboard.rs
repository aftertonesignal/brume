// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Bottom touch keyboard widget. One Canvas program covers all 13
//! keys so multi-touch and glissando are handled with full picture
//! of which finger is on which key. Per-finger state is held in
//! `NativeUi` (so `update()` can dispatch the right NoteOn / NoteOff
//! pair); the canvas just emits high-level press / move / release
//! messages with a finger id.

use iced::alignment;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Program, Text, event::Status};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse, touch};

use crate::Message;

pub const NUM_KEYS: usize = 13;

const KEY_NAMES: [&str; NUM_KEYS] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", "C",
];
const IS_BLACK: [bool; NUM_KEYS] = [
    false, true, false, true, false, false, true, false, true, false, true, false, false,
];

pub struct Keyboard {
    pub white_bg: Color,
    pub black_bg: Color,
    pub border: Color,
    pub label_color: Color,
}

impl Program<Message, Theme, Renderer> for Keyboard {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<Message>) {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position() {
                    if let Some(k) = key_at(pos, bounds) {
                        return (
                            Status::Captured,
                            Some(Message::KeyboardPress {
                                finger: None,
                                key: k,
                            }),
                        );
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position() {
                    if let Some(k) = key_at(pos, bounds) {
                        return (
                            Status::Captured,
                            Some(Message::KeyboardMove {
                                finger: None,
                                key: k,
                            }),
                        );
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                return (
                    Status::Captured,
                    Some(Message::KeyboardRelease { finger: None }),
                );
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                if let Some(k) = key_at(position, bounds) {
                    return (
                        Status::Captured,
                        Some(Message::KeyboardPress {
                            finger: Some(id.0),
                            key: k,
                        }),
                    );
                }
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                if let Some(k) = key_at(position, bounds) {
                    return (
                        Status::Captured,
                        Some(Message::KeyboardMove {
                            finger: Some(id.0),
                            key: k,
                        }),
                    );
                }
            }
            Event::Touch(touch::Event::FingerLifted { id, .. })
            | Event::Touch(touch::Event::FingerLost { id, .. }) => {
                return (
                    Status::Captured,
                    Some(Message::KeyboardRelease { finger: Some(id.0) }),
                );
            }
            _ => {}
        }
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
        let key_w = bounds.width / NUM_KEYS as f32;
        let h = bounds.height;

        for i in 0..NUM_KEYS {
            let x = i as f32 * key_w;
            let bg = if IS_BLACK[i] {
                self.black_bg
            } else {
                self.white_bg
            };
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(key_w, h), bg);

            // 1px right-side divider so adjacent keys don't visually
            // merge under low contrast (especially white-against-white).
            if i < NUM_KEYS - 1 {
                let div = Path::new(|p| {
                    p.move_to(Point::new(x + key_w, 0.0));
                    p.line_to(Point::new(x + key_w, h));
                });
                frame.stroke(
                    &div,
                    canvas::Stroke {
                        style: canvas::Style::Solid(self.border),
                        width: 1.0,
                        ..canvas::Stroke::default()
                    },
                );
            }

            // Key label centered horizontally near the bottom.
            let mut t = Text::default();
            t.content = KEY_NAMES[i].to_string();
            t.position = Point::new(x + key_w / 2.0, h - 8.0);
            t.color = self.label_color;
            t.size = Pixels(11.0);
            t.horizontal_alignment = alignment::Horizontal::Center;
            t.vertical_alignment = alignment::Vertical::Bottom;
            frame.fill_text(t);
        }
        vec![frame.into_geometry()]
    }
}

fn key_at(pos: Point, bounds: Rectangle) -> Option<u8> {
    if pos.x < bounds.x
        || pos.x > bounds.x + bounds.width
        || pos.y < bounds.y
        || pos.y > bounds.y + bounds.height
    {
        return None;
    }
    let key_w = bounds.width / NUM_KEYS as f32;
    let local = pos.x - bounds.x;
    let idx = (local / key_w).floor() as i32;
    if idx < 0 || idx >= NUM_KEYS as i32 {
        None
    } else {
        Some(idx as u8)
    }
}
