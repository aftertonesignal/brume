// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Per-engine signal-flow schematic. One Canvas Program covers all
//! 4 engine modes; the draw logic branches on `mode` and pulls live
//! data (algorithm index, filter cutoff, harmonic levels, etc.) from
//! the shared `param_values` map at view-build time.
//!
//! Each engine has its own `draw_*` function. They share vertical
//! layout constants (PAD_TOP, PAD_BOT, OUT_LABEL_H) but otherwise
//! render bespoke topology — FM's algorithm legend, HARMONIC's bar
//! chart and scan brackets, TIMBRAL's triangle + wavefold trace +
//! self-feedback loop, GRANULAR's grain cloud + morph wave + drift
//! indicator. The per-engine helpers (draw_filter_curve,
//! draw_harmonic_bars, draw_skewed_triangle, draw_grain_cloud, …)
//! aren't data — they're real rendering logic that varies per engine.

use iced::alignment;
use iced::widget::canvas::{
    self, Event, Frame, Geometry, Path, Program, Stroke, Text, event::Status,
};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::tabs::FM_ALGOS;
use crate::{Message, Mode};

// Vertical layout constants shared across the four engine schematics.
// arrow_gap and box_w vary per engine and stay local.
const PAD_TOP: f32 = 12.0;
const PAD_BOT: f32 = 4.0;
const OUT_LABEL_H: f32 = 14.0;

pub struct SignalFlow {
    pub mode: Mode,
    pub mode_color: Color,
    /// FM algorithm index 0..11 (engine value). Used by the FM
    /// schematic to render the currently-selected algorithm name in
    /// the slot above the ALGORITHM box. Ignored by other engines.
    pub algorithm_index: f32,
    /// Filter cutoff position 0..1 — the slider's normalized position,
    /// not the Hz value. Maps directly to horizontal placement of the
    /// knee inside the FILTER box so cutoff=0 → curve at far left
    /// (no audible passband, flat against bottom) and cutoff=1 → knee
    /// at far right (passband fills the box).
    pub filter_cutoff_ratio: f32,
    /// Filter resonance 0..1. Higher → taller resonant peak at cutoff.
    pub filter_resonance: f32,
    // ── HARMONIC fields ──────────────────────────────────────────
    /// H1..H8 levels (engine units, 0..1 each). Drawn as a vertical
    /// bar chart inside the 8 HARMONICS box. Ignored by other engines.
    pub harmonic_levels: [f32; 8],
    /// Scan center 0..1. Amber bracket pair sits at
    /// `center ± width/2` along the SCAN WINDOW box width.
    pub scan_center: f32,
    pub scan_width: f32,
    /// FM modulation depth 0..10 and ratio 0.5..16 — drives the
    /// carrier polyline drawn inside the FM OSC box.
    pub harmonic_fm_depth: f32,
    pub harmonic_fm_ratio: f32,
    // ── TIMBRAL fields ───────────────────────────────────────────
    /// Symmetry tilt -1..1. Drives the skewed triangle wave inside
    /// the TRIANGLE box.
    pub symmetry: f32,
    /// Wavefolder drive 0..1 (Timbre param) and stage count 1..4.
    /// Together drive the wavefold trace inside WAVE MULT — same
    /// inputs `computeFoldPoints` in ui.js takes.
    pub timbre: f32,
    pub multiplier_stages: f32,
    /// Linear-FM depth 0..10 and ratio 0.5..16. Drives the carrier
    /// polyline inside TIMBRAL's FM OSC box.
    pub timbral_fm_depth: f32,
    pub timbral_fm_ratio: f32,
    // ── GRANULAR fields ──────────────────────────────────────────
    /// 0..1 each. Drive the GRAIN CLOUD scatter pattern (count,
    /// vertical spread, dot radius) and the GRAIN ENV envelope shape.
    pub grain_density: f32,
    pub grain_scatter: f32,
    pub grain_size: f32,
    pub grain_shape: f32,
    /// 0..1. Selects the MORPH WAVE four-corner blend
    /// (sine → tri → saw → sq).
    pub granular_morph: f32,
    /// FM sidecar params for the MORPH WAVE box's FM sub-box.
    pub granular_fm_depth: f32,
    pub granular_fm_ratio: f32,
}

impl Program<Message, Theme, Renderer> for SignalFlow {
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
        let stroke = Stroke {
            style: canvas::Style::Solid(self.mode_color),
            width: 1.5,
            ..Stroke::default()
        };

        match self.mode {
            Mode::Fm => self.draw_fm(&mut frame, bounds.size(), &stroke),
            Mode::Harmonic => self.draw_harmonic(&mut frame, bounds.size(), &stroke),
            Mode::Timbral => self.draw_timbral(&mut frame, bounds.size(), &stroke),
            Mode::Granular => self.draw_granular(&mut frame, bounds.size(), &stroke),
        }
        vec![frame.into_geometry()]
    }
}

impl SignalFlow {
    fn draw_fm(&self, frame: &mut Frame, size: Size, stroke: &Stroke) {
        let w = size.width;
        let h = size.height;
        let cx = w / 2.0;

        // Vertical layout budget: pad_top + 4 boxes + 3 inter-arrows
        // + 1 legend slot (between box 0 and box 1) + final OUT slot.
        // box_h is computed from available H so the schematic fills
        // the panel rather than huddling at the top. Top pad is more
        // generous than bottom so the OPS box doesn't crowd the panel
        // header above it.
        let pad_top = PAD_TOP;
        let pad_bot = PAD_BOT;
        let arrow_gap = 14.0;
        // Legend gap is bigger than arrow_gap because the legend text
        // sits inside the gap rather than next to a connector — it
        // needs breathing room above the OPS box and below the legend
        // before the ALGORITHM box.
        let legend_gap = 30.0;
        let out_label_h = OUT_LABEL_H;
        let fixed = pad_top + pad_bot + 3.0 * arrow_gap + legend_gap + arrow_gap + out_label_h;
        let box_h = ((h - fixed) / 4.0).clamp(28.0, 60.0);
        let box_w = (w * 0.66).clamp(120.0, 200.0);
        let box_x = cx - box_w / 2.0;

        // Box top y-positions
        let y0 = pad_top;
        let y1 = y0 + box_h + legend_gap;
        let y2 = y1 + box_h + arrow_gap;
        let y3 = y2 + box_h + arrow_gap;
        let out_y = y3 + box_h + arrow_gap + out_label_h * 0.5;

        // Box 1: 6 OPS / sine operators. FDBK self-loop indicator
        // deferred — drawing it as a horizontal sidecar would cost
        // more panel width than typical sizes can spare. Picked up
        // in the next polish pass.
        self.draw_box(
            frame,
            box_x,
            y0,
            box_w,
            box_h,
            "6 OPS",
            "sine operators",
            stroke,
        );

        // Currently-selected algorithm name in the slot between OPS
        // and ALGORITHM. Just the active variant (e.g. "STACK") — the
        // full list of variants is on the ALGO sub-tab's tap-cycle
        // row, which is the place the user actually picks.
        let idx = (self
            .algorithm_index
            .round()
            .clamp(0.0, FM_ALGOS.len() as f32 - 1.0)) as usize;
        let mut legend = Text::default();
        legend.content = FM_ALGOS[idx].to_string();
        legend.position = Point::new(cx, y0 + box_h + legend_gap * 0.5);
        legend.color = self.mode_color;
        legend.size = Pixels(10.0);
        legend.horizontal_alignment = alignment::Horizontal::Center;
        legend.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(legend);

        // Box 2..4
        self.draw_box(
            frame,
            box_x,
            y1,
            box_w,
            box_h,
            "ALGORITHM",
            "routing",
            stroke,
        );
        self.draw_arrow(frame, cx, y1 + box_h, cx, y2, stroke);
        self.draw_box(
            frame,
            box_x,
            y2,
            box_w,
            box_h,
            "CARRIERS",
            "Σ selected ops",
            stroke,
        );
        self.draw_arrow(frame, cx, y2 + box_h, cx, y3, stroke);

        // FILTER box. Layered rectangle → curve → label. Iced may not
        // strictly preserve insertion order between strokes and text
        // fills, so the curve can occasionally sit over the label
        // when the cutoff sweeps through the box center; we accept
        // that for now (preferred over a visible mask plate behind
        // the text). True text-shape clipping needs a custom mask
        // and lives in Phase 5 polish.
        self.stroke_box_rect(frame, box_x, y3, box_w, box_h, stroke);
        let curve_pad = 4.0;
        self.draw_filter_curve(
            frame,
            box_x + curve_pad,
            y3 + curve_pad,
            box_w - 2.0 * curve_pad,
            box_h - 2.0 * curve_pad,
            stroke,
        );
        self.draw_box_labels(frame, box_x, y3, box_w, box_h, "FILTER", "");

        // OUT label
        self.draw_arrow(frame, cx, y3 + box_h, cx, out_y - 8.0, stroke);
        let mut out = Text::default();
        out.content = "OUT".to_string();
        out.position = Point::new(cx, out_y);
        out.color = self.mode_color;
        out.size = Pixels(9.0);
        out.horizontal_alignment = alignment::Horizontal::Center;
        out.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(out);
    }

    /// HARMONIC engine schematic. Four boxes, top-to-bottom:
    /// FM OSC (carrier polyline trace inside) → 8 HARMONICS (live
    /// H1..H8 bar chart) → SCAN WINDOW (amber bracket pair at
    /// `center ± width/2`) → FILTER (SVF curve, same as FM).
    /// No feedback loop — HARMONIC has no self-modulation.
    fn draw_harmonic(&self, frame: &mut Frame, size: Size, stroke: &Stroke) {
        let w = size.width;
        let h = size.height;
        let cx = w / 2.0;

        // Same vertical layout math as draw_fm — 4 boxes share the
        // panel height after pad / arrows / OUT label budget. There's
        // no algorithm-name slot on HARMONIC, so the boxes spread a
        // bit further apart than FM does.
        let pad_top = PAD_TOP;
        let pad_bot = PAD_BOT;
        let arrow_gap = 14.0;
        let out_label_h = OUT_LABEL_H;
        let fixed = pad_top + pad_bot + 3.0 * arrow_gap + arrow_gap + out_label_h;
        let box_h = ((h - fixed) / 4.0).clamp(28.0, 64.0);
        let box_w = (w * 0.66).clamp(120.0, 200.0);
        let box_x = cx - box_w / 2.0;

        let y0 = pad_top;
        let y1 = y0 + box_h + arrow_gap;
        let y2 = y1 + box_h + arrow_gap;
        let y3 = y2 + box_h + arrow_gap;
        let out_y = y3 + box_h + arrow_gap + out_label_h * 0.5;

        // Box 1: FM OSC. Carrier polyline rendered inside; rectangle
        // stroke + title come last so the trace can't paint over the
        // label (same layered approach as FM's FILTER box).
        self.stroke_box_rect(frame, box_x, y0, box_w, box_h, stroke);
        self.draw_fm_carrier_trace(frame, box_x, y0, box_w, box_h, stroke);
        self.draw_box_labels(frame, box_x, y0, box_w, box_h, "FM OSC", "ratio");

        // Box 2: 8 HARMONICS. Live H1..H8 bars across the box width,
        // with the same amber scan-window rectangle overlaid on top
        // so the user sees which harmonics fall inside the current
        // window. Identical visual to the dedicated SCAN WINDOW box.
        self.draw_arrow(frame, cx, y0 + box_h, cx, y1, stroke);
        self.stroke_box_rect(frame, box_x, y1, box_w, box_h, stroke);
        self.draw_harmonic_bars(frame, box_x, y1, box_w, box_h);
        self.draw_scan_brackets(frame, box_x, y1, box_w, box_h);
        self.draw_box_labels(frame, box_x, y1, box_w, box_h, "8 HARMONICS", "morph");

        // Box 3: SCAN WINDOW. Same scan-window rectangle on its own
        // ground — names the parameter group (center + width).
        self.draw_arrow(frame, cx, y1 + box_h, cx, y2, stroke);
        self.stroke_box_rect(frame, box_x, y2, box_w, box_h, stroke);
        self.draw_scan_brackets(frame, box_x, y2, box_w, box_h);
        self.draw_box_labels(
            frame,
            box_x,
            y2,
            box_w,
            box_h,
            "SCAN WINDOW",
            "center + width",
        );

        // Box 4: FILTER. Same SVF magnitude curve as FM (different
        // cutoff/reso for the harmonic part).
        self.draw_arrow(frame, cx, y2 + box_h, cx, y3, stroke);
        self.stroke_box_rect(frame, box_x, y3, box_w, box_h, stroke);
        let curve_pad = 4.0;
        self.draw_filter_curve(
            frame,
            box_x + curve_pad,
            y3 + curve_pad,
            box_w - 2.0 * curve_pad,
            box_h - 2.0 * curve_pad,
            stroke,
        );
        self.draw_box_labels(frame, box_x, y3, box_w, box_h, "FILTER", "");

        // OUT
        self.draw_arrow(frame, cx, y3 + box_h, cx, out_y - 8.0, stroke);
        let mut out = Text::default();
        out.content = "OUT".to_string();
        out.position = Point::new(cx, out_y);
        out.color = self.mode_color;
        out.size = Pixels(9.0);
        out.horizontal_alignment = alignment::Horizontal::Center;
        out.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(out);
    }

    /// FM-modulated carrier polyline drawn inside the FM OSC box.
    /// At depth=0 we draw a near-flat line so the trace is always
    /// visible (collapsed point would be invisible).
    fn draw_fm_carrier_trace(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: &Stroke,
    ) {
        const N: usize = 128;
        let cycles = self.harmonic_fm_ratio.max(0.5);
        let max_amp = (h - 4.0) / 2.0;
        let amp = if self.harmonic_fm_depth <= 0.0 {
            0.5
        } else {
            (self.harmonic_fm_depth / 10.0).clamp(0.0, 1.0) * max_amp
        };
        let mid_y = y + h * 0.5;
        let span_x = w - 4.0;
        let x0 = x + 2.0;
        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let xi = x0 + t * span_x;
                let yi = mid_y + amp * (std::f32::consts::TAU * cycles * t).sin();
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// 8 vertical bars showing live H1..H8 levels. Bars are spaced
    /// evenly across the box width with small gaps between; bar
    /// height is `level * usable_h`. Filled rectangles in mode color.
    fn draw_harmonic_bars(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32) {
        const N: usize = 8;
        let pad_x = 6.0;
        let pad_y_top = 4.0;
        let pad_y_bot = 4.0;
        let usable_w = (w - 2.0 * pad_x).max(0.0);
        let bar_w = (usable_w / N as f32) * 0.7;
        let stride = usable_w / N as f32;
        let baseline = y + h - pad_y_bot;
        let max_h = (h - pad_y_top - pad_y_bot).max(0.0);
        for (i, &level) in self.harmonic_levels.iter().enumerate() {
            let l = level.clamp(0.0, 1.0);
            if l <= 0.0 {
                continue;
            }
            let bx = x + pad_x + i as f32 * stride + (stride - bar_w) * 0.5;
            let bar_height = l * max_h;
            frame.fill_rectangle(
                Point::new(bx, baseline - bar_height),
                Size::new(bar_w, bar_height),
                self.mode_color,
            );
        }
    }

    /// Amber scan-window indicator: a 4-sided rectangle at
    /// `scan_center ± scan_width/2` horizontally, inset comfortably
    /// from the parent box edges. Used identically inside both the
    /// 8 HARMONICS box (overlaid on the bar chart) and the dedicated
    /// SCAN WINDOW box (standalone). Amber instead of the engine
    /// mode_color (cyan) so the window reads as a distinct UI
    /// element.
    fn draw_scan_brackets(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32) {
        let amber = Color::from_rgba(0.80, 0.53, 0.0, 0.85);
        let pad_x = 8.0;
        let pad_y = 8.0;
        let center = self.scan_center.clamp(0.0, 1.0);
        let half = (self.scan_width.clamp(0.0, 1.0) * 0.5).clamp(0.02, 0.5);
        let left = (center - half).clamp(0.0, 1.0);
        let right = (center + half).clamp(0.0, 1.0);
        let inner_w = (w - 2.0 * pad_x).max(0.0);
        let top = y + pad_y;
        let bot = y + h - pad_y;
        let lx = x + pad_x + left * inner_w;
        let rx = x + pad_x + right * inner_w;

        let stroke_amber = |frame: &mut Frame, p: &Path, width: f32| {
            frame.stroke(
                p,
                canvas::Stroke {
                    style: canvas::Style::Solid(amber),
                    width,
                    ..canvas::Stroke::default()
                },
            );
        };

        let left_line = Path::new(|p| {
            p.move_to(Point::new(lx, top));
            p.line_to(Point::new(lx, bot));
        });
        let right_line = Path::new(|p| {
            p.move_to(Point::new(rx, top));
            p.line_to(Point::new(rx, bot));
        });
        let top_line = Path::new(|p| {
            p.move_to(Point::new(lx, top));
            p.line_to(Point::new(rx, top));
        });
        let bot_line = Path::new(|p| {
            p.move_to(Point::new(lx, bot));
            p.line_to(Point::new(rx, bot));
        });
        stroke_amber(frame, &left_line, 1.5);
        stroke_amber(frame, &right_line, 1.5);
        stroke_amber(frame, &top_line, 1.0);
        stroke_amber(frame, &bot_line, 1.0);
    }

    /// TIMBRAL engine schematic. Five boxes top-to-bottom plus a
    /// self-feedback loop from WAVE MULT back to TRIANGLE. Live
    /// elements: linear-FM carrier polyline, skewed-triangle wave,
    /// wavefolder trace.
    fn draw_timbral(&self, frame: &mut Frame, size: Size, stroke: &Stroke) {
        let w = size.width;
        let h = size.height;
        let cx = w / 2.0;

        // 5 boxes need tighter spacing than 4. Same fixed-budget
        // approach as HARMONIC but with 5 boxes / 5 arrow gaps.
        let pad_top = PAD_TOP;
        let pad_bot = PAD_BOT;
        let arrow_gap = 12.0;
        let out_label_h = OUT_LABEL_H;
        let fixed = pad_top + pad_bot + 4.0 * arrow_gap + arrow_gap + out_label_h;
        let box_h = ((h - fixed) / 5.0).clamp(24.0, 52.0);
        let box_w = (w * 0.62).clamp(120.0, 190.0);
        let box_x = cx - box_w / 2.0;

        let y0 = pad_top;
        let y1 = y0 + box_h + arrow_gap;
        let y2 = y1 + box_h + arrow_gap;
        let y3 = y2 + box_h + arrow_gap;
        let y4 = y3 + box_h + arrow_gap;
        let out_y = y4 + box_h + arrow_gap + out_label_h * 0.5;

        // Box 1: FM OSC / linear (carrier polyline using TIMBRAL's
        // linear FM, hence different label sub from HARMONIC's
        // "ratio").
        self.stroke_box_rect(frame, box_x, y0, box_w, box_h, stroke);
        self.draw_carrier_trace(
            frame,
            box_x,
            y0,
            box_w,
            box_h,
            self.timbral_fm_depth,
            self.timbral_fm_ratio,
            stroke,
        );
        self.draw_box_labels(frame, box_x, y0, box_w, box_h, "FM OSC", "linear");

        // Box 2: TRIANGLE / symmetry tilt (skewed triangle wave).
        self.draw_arrow(frame, cx, y0 + box_h, cx, y1, stroke);
        self.stroke_box_rect(frame, box_x, y1, box_w, box_h, stroke);
        self.draw_skewed_triangle(frame, box_x, y1, box_w, box_h, stroke);
        self.draw_box_labels(frame, box_x, y1, box_w, box_h, "TRIANGLE", "symmetry tilt");

        // Box 3: WAVE MULT / timbre × stages (wavefolder trace).
        self.draw_arrow(frame, cx, y1 + box_h, cx, y2, stroke);
        self.stroke_box_rect(frame, box_x, y2, box_w, box_h, stroke);
        self.draw_wavefold_trace(frame, box_x, y2, box_w, box_h, stroke);
        self.draw_box_labels(
            frame,
            box_x,
            y2,
            box_w,
            box_h,
            "WAVE MULT",
            "timbre × stages",
        );

        // FB feedback loop: WAVE MULT (box 2) → TRIANGLE (box 1).
        // Small dashed-equivalent arrow on the right side of the
        // schematic, with "FB" label. Hand-rolled rather than
        // dashed because iced canvas Stroke doesn't expose dash
        // arrays directly — the visual cue is enough.
        self.draw_timbral_feedback(frame, box_x, box_w, y1, y2, box_h, stroke);

        // Box 4: + SUB OSC / ÷ 2.
        self.draw_arrow(frame, cx, y2 + box_h, cx, y3, stroke);
        self.draw_box(frame, box_x, y3, box_w, box_h, "+ SUB OSC", "÷ 2", stroke);

        // Box 5: FILTER (SVF curve).
        self.draw_arrow(frame, cx, y3 + box_h, cx, y4, stroke);
        self.stroke_box_rect(frame, box_x, y4, box_w, box_h, stroke);
        let curve_pad = 4.0;
        self.draw_filter_curve(
            frame,
            box_x + curve_pad,
            y4 + curve_pad,
            box_w - 2.0 * curve_pad,
            box_h - 2.0 * curve_pad,
            stroke,
        );
        self.draw_box_labels(frame, box_x, y4, box_w, box_h, "FILTER", "");

        // OUT
        self.draw_arrow(frame, cx, y4 + box_h, cx, out_y - 8.0, stroke);
        let mut out = Text::default();
        out.content = "OUT".to_string();
        out.position = Point::new(cx, out_y);
        out.color = self.mode_color;
        out.size = Pixels(9.0);
        out.horizontal_alignment = alignment::Horizontal::Center;
        out.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(out);
    }

    /// Linear-FM carrier polyline. Same shape as
    /// HARMONIC's FM OSC trace but parameterized by depth + ratio
    /// args so both engines can call it without duplicating code.
    fn draw_carrier_trace(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        depth: f32,
        ratio: f32,
        stroke: &Stroke,
    ) {
        const N: usize = 128;
        let cycles = ratio.max(0.5);
        let max_amp = (h - 4.0) / 2.0;
        let amp = if depth <= 0.0 {
            0.5
        } else {
            (depth / 10.0).clamp(0.0, 1.0) * max_amp
        };
        let mid_y = y + h * 0.5;
        let span_x = w - 4.0;
        let x0 = x + 2.0;
        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let xi = x0 + t * span_x;
                let yi = mid_y + amp * (std::f32::consts::TAU * cycles * t).sin();
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// Skewed triangle wave inside the TRIANGLE box. `symmetry` -1..1
    /// shifts the peak position along the box width: -1 = saw-down,
    /// 0 = symmetric triangle, +1 = saw-up.
    fn draw_skewed_triangle(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: &Stroke,
    ) {
        const N: usize = 96;
        let pad = 2.0;
        let inner_w = w - 2.0 * pad;
        let amp = (h - 2.0 * pad) * 0.5 - 0.5;
        let mid_y = y + h * 0.5;
        let x0 = x + pad;
        let sym = self.symmetry.clamp(-1.0, 1.0);
        let peak = (0.5 + sym * 0.5).clamp(0.02, 0.98);

        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let xi = x0 + t * inner_w;
                let v = if t < peak {
                    -1.0 + 2.0 * (t / peak)
                } else {
                    1.0 - 2.0 * (t - peak) / (1.0 - peak)
                };
                let yi = mid_y - v * amp;
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// Wavefolder trace inside WAVE MULT. Ported from
    /// `computeFoldPoints` in ui.js. Two sequential reflection passes
    /// per stage — DELIBERATELY not a unified `while (|v|>1)` loop:
    /// at extreme drive some samples settle slightly outside ±1 and
    /// the trace visibly spills past the box. That overflow is part
    /// of Brume's visual language ("the signal can't be contained")
    /// and is not a bug to be fixed.
    fn draw_wavefold_trace(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: &Stroke,
    ) {
        const N: usize = 96;
        let pad = 2.0;
        let inner_w = w - 2.0 * pad;
        let inner_h = h - 2.0 * pad;
        let mid_y = y + h * 0.5;
        let amp = inner_h * 0.5 - 0.5;
        let x0 = x + pad;
        let amount = self.timbre.clamp(0.0, 1.0);
        let stages = self.multiplier_stages.round().clamp(1.0, 4.0) as i32;
        let drive = 1.0 + amount * 4.0;

        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let xi = x0 + t * inner_w;
                let s = (std::f32::consts::TAU * t).sin();
                let mut v = drive * s;
                for _ in 0..stages {
                    while v > 1.0 {
                        v = 2.0 - v;
                    }
                    while v < -1.0 {
                        v = -2.0 - v;
                    }
                }
                let yi = mid_y - v * amp;
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// Feedback loop indicator. Right-side curved-ish arrow from the
    /// WAVE MULT box's right edge up to the TRIANGLE box's right
    /// edge, with an "FB" label. The connector is right-angle
    /// segments — keeps the path simple and readable in the
    /// horizontal space available next to the boxes.
    fn draw_timbral_feedback(
        &self,
        frame: &mut Frame,
        box_x: f32,
        box_w: f32,
        y_triangle: f32,
        y_wavemult: f32,
        box_h: f32,
        stroke: &Stroke,
    ) {
        let right_edge = box_x + box_w;
        let lane_x = right_edge + 8.0;
        let from_y = y_wavemult + box_h * 0.5;
        let to_y = y_triangle + box_h * 0.5;

        let path = Path::new(|p| {
            p.move_to(Point::new(right_edge, from_y));
            p.line_to(Point::new(lane_x, from_y));
            p.line_to(Point::new(lane_x, to_y));
            p.line_to(Point::new(right_edge + 2.0, to_y));
        });
        frame.stroke(&path, stroke.clone());

        // Small arrow head pointing at TRIANGLE.
        let head = Path::new(|p| {
            p.move_to(Point::new(right_edge + 6.0, to_y - 3.0));
            p.line_to(Point::new(right_edge + 2.0, to_y));
            p.line_to(Point::new(right_edge + 6.0, to_y + 3.0));
            p.close();
        });
        frame.fill(&head, self.mode_color);

        // "FB" label sitting in the lane between the two boxes.
        let mut t = Text::default();
        t.content = "FB".to_string();
        t.position = Point::new(lane_x + 4.0, (from_y + to_y) * 0.5);
        t.color = Color {
            a: 0.7,
            ..self.mode_color
        };
        t.size = Pixels(8.0);
        t.horizontal_alignment = alignment::Horizontal::Left;
        t.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(t);
    }

    /// GRANULAR engine schematic. Five boxes with a GRAIN ENV trace,
    /// a MORPH WAVE blend, and a deterministic grain-dot scatter
    /// inside GRAIN CLOUD. Adds a small FM sidecar to the right of
    /// MORPH WAVE and a DRIFT info indicator dashed off GRAIN CLOUD,
    /// matching FLOW_TOPOLOGY.granular's `sidecars` and `indicators`.
    fn draw_granular(&self, frame: &mut Frame, size: Size, stroke: &Stroke) {
        let w = size.width;
        let h = size.height;
        let cx = w / 2.0;

        let pad_top = PAD_TOP;
        let pad_bot = PAD_BOT;
        let arrow_gap = 12.0;
        let out_label_h = OUT_LABEL_H;
        let fixed = pad_top + pad_bot + 4.0 * arrow_gap + arrow_gap + out_label_h;
        let box_h = ((h - fixed) / 5.0).clamp(24.0, 52.0);
        // Slightly narrower main column to leave space for the
        // sidecar + DRIFT indicator on the right.
        let box_w = (w * 0.56).clamp(110.0, 170.0);
        let box_x = cx - box_w / 2.0;

        let y0 = pad_top;
        let y1 = y0 + box_h + arrow_gap;
        let y2 = y1 + box_h + arrow_gap;
        let y3 = y2 + box_h + arrow_gap;
        let y4 = y3 + box_h + arrow_gap;
        let out_y = y4 + box_h + arrow_gap + out_label_h * 0.5;

        // Box 1: GRAIN CLOUD (deterministic dot scatter inside).
        self.stroke_box_rect(frame, box_x, y0, box_w, box_h, stroke);
        self.draw_grain_cloud(frame, box_x, y0, box_w, box_h);
        self.draw_box_labels(
            frame,
            box_x,
            y0,
            box_w,
            box_h,
            "GRAIN CLOUD",
            "density × scatter",
        );

        // DRIFT indicator: dashed line + label exiting right of GRAIN
        // CLOUD. Per FLOW_TOPOLOGY this is "informational annotation"
        // rather than a signal connection.
        self.draw_drift_indicator(frame, box_x + box_w, y0 + box_h * 0.5, w);

        // Box 2: MORPH WAVE (sine→tri→saw→sq blend). FM sidecar
        // attached to the right.
        self.draw_arrow(frame, cx, y0 + box_h, cx, y1, stroke);
        self.stroke_box_rect(frame, box_x, y1, box_w, box_h, stroke);
        self.draw_morph_wave(frame, box_x, y1, box_w, box_h, stroke);
        self.draw_box_labels(
            frame,
            box_x,
            y1,
            box_w,
            box_h,
            "MORPH WAVE",
            "sin→tri→saw→sq",
        );
        self.draw_fm_sidecar(frame, box_x + box_w, y1, box_h, stroke, w);

        // Box 3: GRAIN ENV (envelope window trace).
        self.draw_arrow(frame, cx, y1 + box_h, cx, y2, stroke);
        self.stroke_box_rect(frame, box_x, y2, box_w, box_h, stroke);
        self.draw_grain_env(frame, box_x, y2, box_w, box_h, stroke);
        self.draw_box_labels(frame, box_x, y2, box_w, box_h, "GRAIN ENV", "size + shape");

        // Box 4: NORMALIZE (no live element).
        self.draw_arrow(frame, cx, y2 + box_h, cx, y3, stroke);
        self.draw_box(frame, box_x, y3, box_w, box_h, "NORMALIZE", "1/√n", stroke);

        // Box 5: FILTER.
        self.draw_arrow(frame, cx, y3 + box_h, cx, y4, stroke);
        self.stroke_box_rect(frame, box_x, y4, box_w, box_h, stroke);
        let curve_pad = 4.0;
        self.draw_filter_curve(
            frame,
            box_x + curve_pad,
            y4 + curve_pad,
            box_w - 2.0 * curve_pad,
            box_h - 2.0 * curve_pad,
            stroke,
        );
        self.draw_box_labels(frame, box_x, y4, box_w, box_h, "FILTER", "");

        // OUT
        self.draw_arrow(frame, cx, y4 + box_h, cx, out_y - 8.0, stroke);
        let mut out = Text::default();
        out.content = "OUT".to_string();
        out.position = Point::new(cx, out_y);
        out.color = self.mode_color;
        out.size = Pixels(9.0);
        out.horizontal_alignment = alignment::Horizontal::Center;
        out.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(out);
    }

    /// Deterministic grain-dot scatter inside GRAIN CLOUD. Fixed
    /// LCG seed (0x5eed) so the dot pattern is stable across
    /// renders and only changes when the param values change.
    /// Density picks how many dots are visible (out of
    /// MAX_GRAINS=28); scatter spreads them vertically; grain_size
    /// scales each dot's radius.
    fn draw_grain_cloud(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32) {
        const MAX_GRAINS: usize = 28;
        let count = (self.grain_density.clamp(0.0, 1.0) * MAX_GRAINS as f32).round() as usize;
        if count == 0 {
            return;
        }
        let pad = 3.0;
        let inner_w = w - 2.0 * pad;
        let inner_h = h - 2.0 * pad;
        let x0 = x + pad;
        let mid_y = y + h * 0.5;
        let r_base = 0.6 + self.grain_size.clamp(0.0, 1.0) * 1.8;
        let sc = self.grain_scatter.clamp(0.0, 1.0);

        // Numerical Recipes LCG. Seed never resets, so the dot
        // positions are stable for fixed param values.
        let mut seed: u32 = 0x5eed;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223) & 0x7fffffff;
            seed as f32 / 0x7fffffff as f32
        };
        for g in 0..count {
            let t = (g as f32 + 0.5) / MAX_GRAINS as f32;
            let cx = x0 + t * inner_w + (rnd() - 0.5) * (inner_w / MAX_GRAINS as f32) * 0.6;
            let cy = mid_y + (rnd() - 0.5) * sc * (inner_h - 2.0);
            let r = r_base * (0.7 + rnd() * 0.6);
            // iced canvas fill_circle — paths are easier; trace a
            // 12-segment polygon as a circle approximation.
            let circle = Path::new(|p| {
                let segments = 12;
                for i in 0..segments {
                    let theta = (i as f32 / segments as f32) * std::f32::consts::TAU;
                    let px = cx + r * theta.cos();
                    let py = cy + r * theta.sin();
                    if i == 0 {
                        p.move_to(Point::new(px, py));
                    } else {
                        p.line_to(Point::new(px, py));
                    }
                }
                p.close();
            });
            frame.fill(&circle, self.mode_color);
        }
    }

    /// MORPH WAVE: four-corner blend (sine → tri → saw → sq) driven
    /// by `granular_morph` 0..1. One cycle drawn across the box.
    fn draw_morph_wave(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, stroke: &Stroke) {
        const N: usize = 128;
        let pad = 2.0;
        let inner_w = w - 2.0 * pad;
        let amp = (h - 2.0 * pad) * 0.5 - 0.5;
        let mid_y = y + h * 0.5;
        let x0 = x + pad;
        let m = self.granular_morph.clamp(0.0, 1.0);
        let seg = m * 3.0;
        let seg_i = (seg.floor() as i32).min(2);
        let f = seg - seg_i as f32;

        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let ph = std::f32::consts::TAU * t;
                let sine = ph.sin();
                let tri = 1.0 - 2.0 * (((t + 0.25) * 2.0).rem_euclid(2.0) - 1.0).abs();
                let saw = 2.0 * (t - (t + 0.5).floor());
                let sq = if t < 0.5 { 1.0 } else { -1.0 };
                let (a, b) = match seg_i {
                    0 => (sine, tri),
                    1 => (tri, saw),
                    _ => (saw, sq),
                };
                let v = a * (1.0 - f) + b * f;
                let xi = x0 + t * inner_w;
                let yi = mid_y - v * amp;
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// GRAIN ENV: window trace blending Hann → Gaussian → Trapezoidal
    /// across `grain_shape` 0..1. `grain_size` rescales the window's
    /// horizontal extent so small grains compress to the box center.
    fn draw_grain_env(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, stroke: &Stroke) {
        const N: usize = 96;
        let pad = 2.0;
        let inner_w = w - 2.0 * pad;
        let x0 = x + pad;
        let y_base = y + h - pad;
        let y_top = y + pad;
        let range = y_base - y_top;
        let sz = self.grain_size.clamp(0.05, 1.0);
        let sh = self.grain_shape.clamp(0.0, 1.0);
        let sh2 = sh * 2.0;
        let seg_i = (sh2.floor() as i32).min(1);
        let f = sh2 - seg_i as f32;

        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let u = (t - 0.5) / sz + 0.5;
                let ww = if !(0.0..=1.0).contains(&u) {
                    0.0
                } else {
                    let hann = 0.5 - 0.5 * (std::f32::consts::TAU * u).cos();
                    let gx = (u - 0.5) * 4.0;
                    let gauss = (-gx * gx).exp();
                    let rise = 0.15;
                    let trap = if u < rise {
                        u / rise
                    } else if u > 1.0 - rise {
                        (1.0 - u) / rise
                    } else {
                        1.0
                    };
                    let (a, b) = if seg_i == 0 {
                        (hann, gauss)
                    } else {
                        (gauss, trap)
                    };
                    a * (1.0 - f) + b * f
                };
                let xi = x0 + t * inner_w;
                let yi = y_base - ww * range;
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }

    /// Small FM sidecar attached to the right of MORPH WAVE.
    /// Half-height box with the FM ratio carrier polyline inside —
    /// signals that the morph wave can be FM'd. Skips drawing
    /// entirely if there isn't horizontal room (narrow panel).
    fn draw_fm_sidecar(
        &self,
        frame: &mut Frame,
        right_edge: f32,
        y: f32,
        box_h: f32,
        stroke: &Stroke,
        w: f32,
    ) {
        let sidecar_w = ((w - right_edge) - 12.0).clamp(0.0, 50.0);
        if sidecar_w < 28.0 {
            return;
        }
        let sx = right_edge + 8.0;
        let sh = box_h * 0.7;
        let sy = y + (box_h - sh) * 0.5;
        // Connector from MORPH WAVE to sidecar.
        let connector = Path::new(|p| {
            p.move_to(Point::new(right_edge, y + box_h * 0.5));
            p.line_to(Point::new(sx, sy + sh * 0.5));
        });
        frame.stroke(&connector, stroke.clone());

        self.stroke_box_rect(frame, sx, sy, sidecar_w, sh, stroke);
        self.draw_carrier_trace(
            frame,
            sx,
            sy,
            sidecar_w,
            sh,
            self.granular_fm_depth,
            self.granular_fm_ratio,
            stroke,
        );
        let mut t = Text::default();
        t.content = "FM".to_string();
        t.position = Point::new(sx + sidecar_w * 0.5, sy + sh * 0.5);
        t.color = self.mode_color;
        t.size = Pixels(8.0);
        t.horizontal_alignment = alignment::Horizontal::Center;
        t.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(t);
    }

    /// DRIFT informational indicator: short dashed line + label
    /// exiting the right edge of GRAIN CLOUD. Not a signal
    /// connection — flags that the drift parameter modulates the
    /// cloud's grain timing without showing where it goes.
    fn draw_drift_indicator(&self, frame: &mut Frame, right_edge: f32, y: f32, w: f32) {
        let lane_end = (right_edge + 30.0).min(w - 4.0);
        if lane_end <= right_edge + 6.0 {
            return;
        }
        // Dashed line via short alternating segments — iced 0.13's
        // canvas Stroke doesn't expose a line_dash array directly,
        // so we draw the dashes as separate sub-paths.
        let mut x = right_edge + 4.0;
        let dash = 4.0;
        let gap = 3.0;
        while x + dash <= lane_end {
            let seg = Path::new(|p| {
                p.move_to(Point::new(x, y));
                p.line_to(Point::new(x + dash, y));
            });
            frame.stroke(
                &seg,
                canvas::Stroke {
                    style: canvas::Style::Solid(Color {
                        a: 0.6,
                        ..self.mode_color
                    }),
                    width: 1.0,
                    ..canvas::Stroke::default()
                },
            );
            x += dash + gap;
        }
        let mut t = Text::default();
        t.content = "DRIFT".to_string();
        t.position = Point::new(lane_end + 2.0, y);
        t.color = Color {
            a: 0.7,
            ..self.mode_color
        };
        t.size = Pixels(8.0);
        t.horizontal_alignment = alignment::Horizontal::Left;
        t.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(t);
    }

    /// Stroke a rectangle and place a title (top half) + sub (bottom
    /// half) centered inside it. Sub may be `""` to omit. Convenience
    /// wrapper that calls `stroke_box_rect` then `draw_box_labels`;
    /// boxes that want a custom layer (e.g. FILTER curve under the
    /// title) call those two halves directly with the curve in
    /// between.
    fn draw_box(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        title: &str,
        sub: &str,
        stroke: &Stroke,
    ) {
        self.stroke_box_rect(frame, x, y, w, h, stroke);
        self.draw_box_labels(frame, x, y, w, h, title, sub);
    }

    fn stroke_box_rect(&self, frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, stroke: &Stroke) {
        let rect = Path::new(|p| {
            p.move_to(Point::new(x, y));
            p.line_to(Point::new(x + w, y));
            p.line_to(Point::new(x + w, y + h));
            p.line_to(Point::new(x, y + h));
            p.close();
        });
        frame.stroke(&rect, stroke.clone());
    }

    fn draw_box_labels(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        title: &str,
        sub: &str,
    ) {
        let cx = x + w / 2.0;
        let title_y = if sub.is_empty() {
            y + h / 2.0
        } else {
            y + h * 0.32
        };
        let mut t = Text::default();
        t.content = title.to_string();
        t.position = Point::new(cx, title_y);
        t.color = self.mode_color;
        t.size = Pixels(10.0);
        t.horizontal_alignment = alignment::Horizontal::Center;
        t.vertical_alignment = alignment::Vertical::Center;
        frame.fill_text(t);

        if !sub.is_empty() {
            let mut s = Text::default();
            s.content = sub.to_string();
            s.position = Point::new(cx, y + h * 0.72);
            s.color = Color {
                a: 0.7,
                ..self.mode_color
            };
            s.size = Pixels(8.0);
            s.horizontal_alignment = alignment::Horizontal::Center;
            s.vertical_alignment = alignment::Vertical::Center;
            frame.fill_text(s);
        }
    }

    /// Draw a vertical-only arrow from (x1,y1) to (x2,y2), with a
    /// small triangular head at the destination.
    fn draw_arrow(&self, frame: &mut Frame, x1: f32, y1: f32, x2: f32, y2: f32, stroke: &Stroke) {
        let line = Path::new(|p| {
            p.move_to(Point::new(x1, y1));
            p.line_to(Point::new(x2, y2));
        });
        frame.stroke(&line, stroke.clone());
        let head = Path::new(|p| {
            p.move_to(Point::new(x2 - 4.0, y2 - 6.0));
            p.line_to(Point::new(x2, y2));
            p.line_to(Point::new(x2 + 4.0, y2 - 6.0));
            p.close();
        });
        frame.fill(&head, self.mode_color);
    }

    /// SVF lowpass magnitude curve. N=64 polyline: flat passband at
    /// unity, resonant bell at the cutoff, 2nd-order rolloff above.
    /// Schematic — visual cue, not a Bode plot. Strictly clamped to
    /// the (x..x+w, y..y+h) rectangle so the stroke can't overflow
    /// the FILTER box.
    fn draw_filter_curve(
        &self,
        frame: &mut Frame,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: &Stroke,
    ) {
        const N: usize = 64;
        // Cutoff is provided as a 0..1 ratio matching the slider's
        // position. Maps directly to the horizontal knee position.
        // Slider=0 → curve sits at the bottom (near-zero passband);
        // slider=1 → knee at far right.
        let cutoff_n = self.filter_cutoff_ratio.clamp(0.0, 1.0);

        // Resonance clamped just below 1.0 so the bell can't go to
        // infinity (full resonance is self-oscillation).
        let r = self.filter_resonance.clamp(0.0, 0.98);
        let q = 0.5 + r * 9.5;
        let peak_boost = 1.0 + r * 3.0;
        let max_mag = peak_boost + 0.2;

        let y_base = y + h;
        let y_top = y;

        let curve = Path::new(|p| {
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let xi = x + t * w;
                let d = t - cutoff_n;
                let bell = peak_boost / (1.0 + (d * (6.0 + q)).powi(2));
                let shelf = if t <= cutoff_n {
                    1.0
                } else {
                    (-(t - cutoff_n) * 5.0).exp()
                };
                let mag = max_mag.min(shelf.max(bell));
                let yi = (y_base - (mag / max_mag) * h).clamp(y_top, y_base);
                if i == 0 {
                    p.move_to(Point::new(xi, yi));
                } else {
                    p.line_to(Point::new(xi, yi));
                }
            }
        });
        frame.stroke(&curve, stroke.clone());
    }
}
