// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Non-interactive Canvas widget for the SCOPE panel's OUT level
//! meter. dB-scaled like a DAW/hi-fi meter: a graduated green → amber
//! → red fill, a peak-hold tick, and a dedicated clip LED at the right
//! edge that latches when a window crossed 0 dBFS. All ballistics
//! (attack/release/hold/clip dwell) live in `NativeUi::advance_meter`;
//! this widget only paints the supplied state.
//!
//! Distinct from [`crate::scope_bar`] — that one is a plain linear
//! `0..value` fill reused by the MODULATION rows, which must not pick
//! up audio-meter zones or scaling.

use iced::widget::canvas::{Event, Frame, Geometry, Program, event::Status};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;

/// Lowest level the meter resolves (left edge), in dBFS.
const DB_FLOOR: f32 = -48.0;
/// Highest level the meter resolves (right edge), in dBFS — matches the
/// engine's +6 dB `ScopeFrame` ceiling.
const DB_CEIL: f32 = 6.0;
/// Top of the green zone / start of amber, in dBFS.
const GREEN_TOP_DB: f32 = -12.0;
/// Top of the amber zone / start of red, in dBFS.
const AMBER_TOP_DB: f32 = -3.0;

const GREEN: Color = Color {
    r: 0.22,
    g: 0.85,
    b: 0.42,
    a: 0.95,
};
const AMBER: Color = Color {
    r: 0.96,
    g: 0.72,
    b: 0.20,
    a: 0.95,
};
const RED: Color = Color {
    r: 0.95,
    g: 0.27,
    b: 0.27,
    a: 0.98,
};

/// Maps a dBFS value to a [0, 1] horizontal fraction across the bar.
fn frac_at_db(db: f32) -> f32 {
    ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0)
}

/// Maps a linear amplitude to its [0, 1] horizontal fraction.
fn frac_at_level(level: f32) -> f32 {
    if level <= 0.0 {
        0.0
    } else {
        frac_at_db(20.0 * level.log10())
    }
}

/// The zone color a given linear level falls in.
fn zone_color(level: f32) -> Color {
    let db = if level <= 0.0 {
        DB_FLOOR
    } else {
        20.0 * level.log10()
    };
    if db < GREEN_TOP_DB {
        GREEN
    } else if db < AMBER_TOP_DB {
        AMBER
    } else {
        RED
    }
}

pub struct LevelMeter {
    /// Ballistic displayed level (linear amplitude, 0..~2.0).
    pub level: f32,
    /// Peak-hold level (linear amplitude).
    pub hold: f32,
    /// Clip latch — true while a recent window crossed 0 dBFS.
    pub clip: bool,
}

impl Program<Message, Theme, Renderer> for LevelMeter {
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
        let w = bounds.width;
        let h = bounds.height;

        // Reserve a clip LED at the right edge; the level bar takes the
        // rest.
        let clip_w = (w * 0.12).clamp(4.0, 8.0);
        let gap = 2.0;
        let bar_w = (w - clip_w - gap).max(1.0);

        // Dim rail behind the bar so it reads as a meter even at silence.
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(bar_w, h),
            Color {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 0.14,
            },
        );

        // dB-scaled fill, painted as up to three zone segments so the lit
        // portion grades green → amber → red as it climbs.
        let xf = frac_at_level(self.level) * bar_w;
        let xg = frac_at_db(GREEN_TOP_DB) * bar_w;
        let xa = frac_at_db(AMBER_TOP_DB) * bar_w;
        let seg = |frame: &mut Frame, x0: f32, x1: f32, color: Color| {
            if x1 > x0 {
                frame.fill_rectangle(Point::new(x0, 0.0), Size::new(x1 - x0, h), color);
            }
        };
        seg(&mut frame, 0.0, xf.min(xg), GREEN);
        if xf > xg {
            seg(&mut frame, xg, xf.min(xa), AMBER);
        }
        if xf > xa {
            seg(&mut frame, xa, xf, RED);
        }

        // Peak-hold tick (2 px) at the held level, in that level's zone
        // color so a held-into-red peak stays visibly red after the bar
        // falls back.
        if self.hold > 0.0 {
            let xh = (frac_at_level(self.hold) * bar_w).clamp(0.0, bar_w - 2.0);
            frame.fill_rectangle(
                Point::new(xh, 0.0),
                Size::new(2.0, h),
                zone_color(self.hold),
            );
        }

        // Clip LED: dim red rail, full red when latched.
        let led_color = if self.clip {
            RED
        } else {
            Color { a: 0.16, ..RED }
        };
        frame.fill_rectangle(
            Point::new(bar_w + gap, 0.0),
            Size::new(clip_w, h),
            led_color,
        );

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_mapping_spans_floor_to_ceil() {
        assert!(frac_at_db(DB_FLOOR).abs() < 1e-6, "floor maps to left edge");
        assert!(
            (frac_at_db(DB_CEIL) - 1.0).abs() < 1e-6,
            "ceil maps to right edge"
        );
        assert!(frac_at_db(-100.0).abs() < 1e-6, "below floor clamps to 0");
        assert!(
            (frac_at_db(100.0) - 1.0).abs() < 1e-6,
            "above ceil clamps to 1"
        );
        let unity = frac_at_db(0.0);
        assert!(
            unity > 0.0 && unity < 1.0,
            "0 dBFS sits inside the bar: {unity}"
        );
    }

    #[test]
    fn silence_maps_to_left_edge() {
        assert!(frac_at_level(0.0).abs() < 1e-9);
        // Unity gain is 0 dBFS.
        assert!((frac_at_level(1.0) - frac_at_db(0.0)).abs() < 1e-6);
    }

    #[test]
    fn zones_grade_green_amber_red_with_level() {
        let green = 10f32.powf((GREEN_TOP_DB - 1.0) / 20.0); // below -12 dB
        let amber = 10f32.powf((GREEN_TOP_DB + 1.0) / 20.0); // -12..-3 dB
        let red = 10f32.powf((AMBER_TOP_DB + 1.0) / 20.0); // above -3 dB
        assert_eq!(zone_color(green), GREEN);
        assert_eq!(zone_color(amber), AMBER);
        assert_eq!(zone_color(red), RED);
    }
}
