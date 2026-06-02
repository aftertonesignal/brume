// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Engine page (FM / HARMONIC / TIMBRAL / GRANULAR).
//!
//! 1/3-2/3 split. Left CONTROLS panel hosts sub-tab strip,
//! parameter rows, transport row, and the inline keyboard. Right
//! column stacks SIGNAL FLOW / SCOPE / MODULATION.
//!
//! The keyboard, transport row, and toast overlay live on
//! `NativeUi` itself (in `lib.rs`) since they're shared chrome.

use brume_common::ParameterId;

use iced::widget::{Space, button, container, horizontal_rule, horizontal_space, mouse_area};
use iced::widget::{column, row, text};
use iced::{Alignment, Element, Length, Theme};

use crate::tabs::ParamSpec;
use crate::{
    BRAND_AMBER, LABEL_MUTED, Message, Mode, NativeUi, RULE_LINE, TEXT_BRIGHT, TEXT_DIM, format_db,
    format_value, level_meter, panel, panel_with_action, ratio_bar, scope_bar, signal_flow,
    tab_btn_style,
};

impl NativeUi {
    /// Engine page: 1/3-2/3 split. Left CONTROLS panel hosts sub-tab
    /// strip, scrollable param list, transport row, and the inline
    /// keyboard. Right column stacks SIGNAL FLOW / SCOPE / MODULATION.
    pub(crate) fn engine_page(&self, mode: Mode) -> Element<'_, Message> {
        let left = self.controls_panel(mode);
        let right = column![
            container(panel("SIGNAL FLOW", self.signal_flow_canvas(mode)))
                .height(Length::FillPortion(5)),
            container(panel("SCOPE", self.scope_body(mode))).height(Length::FillPortion(2)),
            container(panel("MODULATION", self.modulation_body())).height(Length::FillPortion(4)),
        ]
        .spacing(8);
        row![
            container(left).width(Length::FillPortion(2)),
            container(right).width(Length::FillPortion(1)),
        ]
        .spacing(8)
        .into()
    }

    /// Left column on engine pages: CONTROLS panel containing sub-tab
    /// strip, parameter rows for the active sub-tab, the
    /// HOLD/RETRIG/CHORD/octave row, and the inline keyboard.
    fn controls_panel(&self, mode: Mode) -> Element<'_, Message> {
        let body = column![
            self.sub_tabs(mode),
            container(self.engine_params(mode))
                .height(Length::Fill)
                .padding([8, 12]),
            // Hairline rule separates the param list from the
            // transport row — both share the same fill color, so
            // without a rule they read as one continuous block.
            // Painted as a 1 px container background rather than
            // `horizontal_rule` because the rule widget collapses
            // visually next to a `Length::Fill` sibling above.
            container(Space::with_width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(|_t: &Theme| container::Style {
                    background: Some(
                        iced::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.10
                        }
                        .into(),
                    ),
                    ..Default::default()
                }),
            container(self.transport_row()).padding([4, 8]),
            // Keyboard runs nearly edge-to-edge of the CONTROLS panel.
            // 1px horizontal inset so the canvas doesn't paint over
            // the panel's 1px border (full edge-to-edge clipped the
            // border visually).
            container(self.keyboard()).padding([0, 1]),
        ]
        .spacing(0);
        panel_with_action(
            "CONTROLS",
            "PANIC",
            BRAND_AMBER,
            Message::Panic(mode),
            body.into(),
        )
    }

    /// Sub-tab strip — one button per (engine, sub-tab) pair. Active
    /// tab gets a low-alpha mode-color pill backing; same font size /
    /// weight / position in both states (no layout shift on tap).
    fn sub_tabs(&self, mode: Mode) -> Element<'_, Message> {
        let tabs = mode.tabs();
        let active_idx = self.active_sub_tab_for(mode);
        let mode_color = mode.color();

        let strip = tabs
            .iter()
            .enumerate()
            .fold(row![].spacing(0), |r, (i, (label, _))| {
                let active = i == active_idx;
                // Same rationale as the engine top tabs — active label
                // goes white so it doesn't blend with the mode-color pill
                // background.
                let label_color = if active { TEXT_BRIGHT } else { TEXT_DIM };
                r.push(
                    button(
                        container(text(*label).size(10).color(label_color))
                            .center_x(Length::Fill)
                            .padding([2, 0]),
                    )
                    .width(Length::Fill)
                    .padding([4, 14])
                    .on_press(Message::SwitchSubTab(mode, i))
                    .style(move |_t, _s| tab_btn_style(active, mode_color)),
                )
            });

        container(column![
            strip,
            horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }),
        ])
        .into()
    }

    /// Render parameter rows for the active sub-tab of `mode`.
    fn engine_params(&self, mode: Mode) -> Element<'_, Message> {
        let params = self.current_params(mode);
        let mode_color = mode.color();
        let (label, _) = mode.tabs()[self.active_sub_tab_for(mode)];

        let rows = params.iter().fold(column![].spacing(8), |col, spec| {
            col.push(self.param_row(spec, mode_color))
        });

        column![
            // Section label above the params: small-caps muted,
            // with hairline rules on either side.
            container(
                row![
                    horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                        color: RULE_LINE,
                        width: 1,
                        radius: 0.0.into(),
                        fill_mode: iced::widget::rule::FillMode::Full,
                    }),
                    container(text(label).size(9).color(LABEL_MUTED)).padding([0, 12]),
                    horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                        color: RULE_LINE,
                        width: 1,
                        radius: 0.0.into(),
                        fill_mode: iced::widget::rule::FillMode::Full,
                    }),
                ]
                .align_y(Alignment::Center)
            )
            .padding([4, 0])
            .width(Length::Fill),
            rows,
        ]
        .spacing(8)
        .into()
    }

    /// One row of the parameter list. Tap-enum params (Algorithm and
    /// any future selector type) render as `[label] [value-name]` with
    /// no slider — the row is wrapped in a `mouse_area` so a tap on
    /// the row cycles to the next variant. Continuous params render as
    /// `[label] [slider] [numeric]`. Both row variants share the same
    /// label-column width (110 px) so the labels align across rows.
    fn param_row(&self, spec: &ParamSpec, mode_color: iced::Color) -> Element<'_, Message> {
        let value = self.value_for(spec);
        let binding = spec.binding;
        let label_widget = text(spec.label)
            .width(Length::Fixed(110.0))
            .size(11)
            .color(LABEL_MUTED);

        if let Some(names) = spec.tap {
            let idx = (value.round().clamp(0.0, names.len() as f32 - 1.0)) as usize;
            let name = names[idx];
            let row_widget = row![
                label_widget,
                text(name).size(11).color(mode_color),
                horizontal_space().width(Length::Fill),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            mouse_area(row_widget)
                .on_press(Message::TapEnum(binding, names))
                .into()
        } else {
            let ratio = binding.unapply(value);
            row![
                label_widget,
                ratio_bar::ratio_bar(ratio, mode_color, move |r| {
                    Message::SliderChanged(binding, r)
                })
                .width(Length::Fill)
                .height(Length::Fixed(20.0)),
                text(format_value(value))
                    .width(Length::Fixed(80.0))
                    .size(11)
                    .color(TEXT_BRIGHT),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        }
    }

    /// Build the per-engine SIGNAL FLOW Canvas widget. Param values
    /// are pulled from `param_values` at view-build time so the
    /// schematic redraws when the engine echoes new values. The
    /// filter cutoff is unapplied via the engine's binding so the
    /// curve uses the slider's 0..1 ratio (matching what the user
    /// sees on the FILTER sub-tab) rather than absolute Hz.
    fn signal_flow_canvas(&self, mode: Mode) -> Element<'_, Message> {
        let part = mode.part();
        let lookup = |id: ParameterId, default: f32| -> f32 {
            self.param_values
                .get(&(part, id))
                .copied()
                .unwrap_or(default)
        };
        // Find the cutoff ParamSpec on this engine's FILTER sub-tab so
        // we can use the binding's `unapply` for the same log-mapping
        // the slider uses.
        let cutoff_value = lookup(ParameterId::FilterCutoff, 8000.0);
        let cutoff_ratio = mode
            .tabs()
            .iter()
            .flat_map(|(_, params)| params.iter())
            .find(|p| p.binding.id == ParameterId::FilterCutoff)
            .map_or(0.5, |p| p.binding.unapply(cutoff_value));

        let harmonic_levels = [
            lookup(ParameterId::HarmonicLevel1, 1.0),
            lookup(ParameterId::HarmonicLevel2, 0.5),
            lookup(ParameterId::HarmonicLevel3, 0.3),
            lookup(ParameterId::HarmonicLevel4, 0.25),
            lookup(ParameterId::HarmonicLevel5, 0.2),
            lookup(ParameterId::HarmonicLevel6, 0.15),
            lookup(ParameterId::HarmonicLevel7, 0.1),
            lookup(ParameterId::HarmonicLevel8, 0.08),
        ];

        let prog = signal_flow::SignalFlow {
            mode,
            mode_color: mode.color(),
            algorithm_index: lookup(ParameterId::Algorithm, 0.0),
            filter_cutoff_ratio: cutoff_ratio,
            filter_resonance: lookup(ParameterId::FilterResonance, 0.1),
            harmonic_levels,
            scan_center: lookup(ParameterId::ScanCenter, 0.5),
            scan_width: lookup(ParameterId::ScanWidth, 1.0),
            harmonic_fm_depth: lookup(ParameterId::HarmonicFmDepth, 0.0),
            harmonic_fm_ratio: lookup(ParameterId::HarmonicFmRatio, 2.0),
            symmetry: lookup(ParameterId::Symmetry, 0.0),
            timbre: lookup(ParameterId::Timbre, 0.0),
            multiplier_stages: lookup(ParameterId::MultiplierStages, 1.0),
            timbral_fm_depth: lookup(ParameterId::TimbralFmDepth, 0.0),
            timbral_fm_ratio: lookup(ParameterId::TimbralFmRatio, 2.0),
            grain_density: lookup(ParameterId::GranularDensity, 0.3),
            grain_scatter: lookup(ParameterId::GranularScatter, 0.0),
            grain_size: lookup(ParameterId::GranularGrainSize, 0.2),
            grain_shape: lookup(ParameterId::GranularGrainShape, 0.0),
            granular_morph: lookup(ParameterId::GranularMorph, 0.0),
            granular_fm_depth: lookup(ParameterId::GranularFmDepth, 0.0),
            granular_fm_ratio: lookup(ParameterId::GranularFmRatio, 2.0),
        };
        iced::widget::canvas::Canvas::new(prog)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// SCOPE panel body. Renders the ballistic dB level meter (green →
    /// amber → red zones, peak-hold tick, clip LED — see `level_meter`)
    /// plus PEAK (held) / RMS dB readouts. Fed by `EngineToUi::ScopeFrame`
    /// echoes for whichever part the UI has registered via `WatchPart`,
    /// with ballistics advanced on Tick in `NativeUi::advance_meter`.
    fn scope_body(&self, mode: Mode) -> Element<'_, Message> {
        // Soft, distinct colors per label so the scope panel doesn't
        // read as a wall of gray. OUT gets the active engine's
        // accent (ties the panel to the active mode); PEAK takes
        // amber, RMS takes cyan — both at ~0.6 alpha so they read
        // as labels, not data.
        let out_label = iced::Color {
            a: 0.6,
            ..mode.color()
        };
        let peak_label = iced::Color {
            a: 0.6,
            ..Mode::Timbral.color()
        }; // soft amber
        let rms_label = iced::Color {
            a: 0.6,
            ..Mode::Harmonic.color()
        }; // soft cyan

        let bar = iced::widget::canvas::Canvas::new(level_meter::LevelMeter {
            level: self.audio.meter_level,
            hold: self.audio.meter_hold,
            clip: self.audio.meter_clip_remaining > 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fixed(12.0));

        container(
            column![
                row![
                    text("OUT").size(10).color(out_label),
                    Space::with_width(Length::Fixed(8.0)),
                    bar,
                ]
                .align_y(Alignment::Center),
                // dB readouts are fixed-width + monospace so the
                // column doesn't dance around as the value goes from
                // "−∞ dB" (5 chars) to "−12.4 dB" (8 chars) to "0.0 dB"
                // (6 chars). Without this, PEAK and RMS positions
                // jitter on every signal change.
                row![
                    text("PEAK").size(9).color(peak_label),
                    Space::with_width(Length::Fixed(8.0)),
                    text(format_db(self.audio.meter_hold))
                        .size(10)
                        .color(TEXT_BRIGHT)
                        .font(iced::Font::MONOSPACE)
                        .width(Length::Fixed(70.0))
                        .align_x(iced::alignment::Horizontal::Left),
                    horizontal_space().width(Length::Fill),
                    text("RMS").size(9).color(rms_label),
                    Space::with_width(Length::Fixed(8.0)),
                    text(format_db(self.audio.scope_rms))
                        .size(10)
                        .color(TEXT_BRIGHT)
                        .font(iced::Font::MONOSPACE)
                        .width(Length::Fixed(70.0))
                        .align_x(iced::alignment::Horizontal::Left),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .padding([12, 14])
        .into()
    }

    /// MODULATION panel body. Four rows — LFO1, LFO2, SEQ1, SEQ2 —
    /// each with a distinct source-glyph + accent color, a live
    /// Canvas-painted bar driven by `EngineToUi::ModFrame`, the
    /// numeric value, and the destination string ("(UNROUTED)" when
    /// no assignment).
    fn modulation_body(&self) -> Element<'_, Message> {
        // Source glyphs — distinct shape per row so the panel reads
        // as four different sources at a glance, not four traffic-
        // light dots. ∿ / ≈ hint at oscillating sources, ⋮ / ⋯ at
        // stepped sources. All rendered at the source accent color
        // without a phosphor halo (the rectangular halo from earlier
        // looked like signal boxes around the glyphs).
        let row = |glyph: &'static str,
                   label: &'static str,
                   value: f32,
                   color: iced::Color|
         -> Element<'_, Message> {
            let bar = iced::widget::canvas::Canvas::new(scope_bar::ScopeBar {
                peak: value,
                mode_color: color,
            })
            .width(Length::Fixed(96.0))
            .height(Length::Fixed(8.0));
            row![
                text(glyph).size(11).color(color).width(Length::Fixed(14.0)),
                text(label).size(10).color(color).width(Length::Fixed(40.0)),
                bar,
                Space::with_width(Length::Fixed(8.0)),
                text(format!("{value:.2}"))
                    .size(10)
                    .color(TEXT_BRIGHT)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(40.0)),
                text("(UNROUTED)").size(9).color(LABEL_MUTED),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        };
        container(
            column![
                row("∿", "LFO1", self.modulation.lfo1, Mode::Fm.color()),
                row("≈", "LFO2", self.modulation.lfo2, Mode::Harmonic.color()),
                row("⋮", "SEQ1", self.modulation.seq1, Mode::Timbral.color()),
                row("⋯", "SEQ2", self.modulation.seq2, Mode::Granular.color()),
            ]
            .spacing(6),
        )
        .padding([12, 14])
        .into()
    }
}

/// Idle SCOPE placeholder for code paths that build the panel before
/// the audio thread has produced its first frame. Currently unused;
/// kept as the canonical "no signal" view to drop in if the live
/// `scope_body` ever needs a fallback (engine teardown, missing
/// `WatchPart` ack, etc.).
#[allow(dead_code)]
pub(super) fn scope_stub<'a>() -> Element<'a, Message> {
    container(
        column![
            text("OUT").size(10).color(LABEL_MUTED),
            row![
                text("PEAK").size(9).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(8.0)),
                text("−∞ DB").size(10).color(TEXT_DIM),
                horizontal_space().width(Length::Fill),
                text("RMS").size(9).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(8.0)),
                text("−∞ DB").size(10).color(TEXT_DIM),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(10),
    )
    .padding([12, 14])
    .into()
}
