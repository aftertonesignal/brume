// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MOD page — LFO + sequencer + routing per part.
//!
//! Side-by-side layout: MODULATION (controls) on the left, LFO PREVIEW
//! (canvas with both shape traces) on the right. Each part has its own
//! LFO/SEQ snapshot and assignment list — `mod_part_strip` switches
//! between parts.
//!
//! Engine has no echo for LFO config or sequencer steps; the local
//! snapshots in `NativeUi.mod_lfo_state`/`mod_seq_state`/`mod_assignments`
//! are the source of truth for the UI, and every mutation here ships
//! a `SetLfo` / `SetSeqStep` / `Add+RemoveAssignment` to the engine
//! to keep the two sides in lockstep.

use brume_app_protocol::UiToEngine;
use brume_common::try_send_or_log;

use iced::widget::{button, column, container, horizontal_rule, pick_list, row, scrollable, text};
use iced::{Alignment, Element, Length, Theme};

use crate::{
    DEST_OPTIONS, FX_TAB_TRAY_BG, LABEL_MUTED, LEARN_RED, RULE_LINE, SHAPE_NAMES, SOURCE_OPTIONS,
    TEXT_BRIGHT, TEXT_DIM, assign_depth_bar, lfo_preview, panel, pick_list_menu_style,
    pick_list_style, quantize_ratio_to_bucket, rate_to_ratio, ratio_bar, ratio_to_rate,
    tab_btn_style,
};
use crate::{Message, ModKnobTarget, Mode, NativeUi};

impl NativeUi {
    /// MOD page body — side-by-side layout: MODULATION (controls)
    /// on the left, LFO PREVIEW (canvas with both shape traces) on
    /// the right.
    pub(crate) fn mod_page(&self) -> Element<'_, Message> {
        let active = self.modulation.active_part;
        // Sections in a single scrollable column — together LFO 1/2
        // + SEQ 1/2 (and later Routing) total ~22 rows, which doesn't
        // fit on a 1024×600 logical screen without scrolling.
        let sections = column![
            self.lfo_section(active, 0, "LFO 1"),
            self.lfo_section(active, 1, "LFO 2"),
            self.seq_section(active, 0, "SEQ 1"),
            self.seq_section(active, 1, "SEQ 2"),
            self.routing_section(active),
        ]
        .spacing(16);
        let mod_body = column![
            self.mod_part_strip(),
            container(scrollable(sections).height(Length::Fill))
                .padding([16, 16])
                .height(Length::Fill),
        ]
        .spacing(0);
        let mod_panel = container(panel("MODULATION", mod_body.into()))
            .width(Length::FillPortion(1))
            .height(Length::Fill);

        // Build the live LFO preview from the active part's snapshot.
        let s1 = self.modulation.lfo_state[active as usize][0];
        let s2 = self.modulation.lfo_state[active as usize][1];
        let preview = lfo_preview::LfoPreview {
            lfo1_shape: lfo_preview::shape_from_name(SHAPE_NAMES[s1.shape_idx]),
            lfo1_rate_hz: s1.rate_hz,
            lfo1_phase: self.modulation.lfo_phase[0],
            lfo1_label: "LFO 1",
            lfo1_color: Mode::Fm.color(),
            lfo2_shape: lfo_preview::shape_from_name(SHAPE_NAMES[s2.shape_idx]),
            lfo2_rate_hz: s2.rate_hz,
            lfo2_phase: self.modulation.lfo_phase[1],
            lfo2_label: "LFO 2",
            lfo2_color: Mode::Harmonic.color(),
            label_muted: LABEL_MUTED,
            shape_name_color: LEARN_RED,
        };
        let preview_canvas = iced::widget::canvas::Canvas::new(preview)
            .width(Length::Fill)
            .height(Length::Fill);
        let preview_panel = container(panel(
            "LFO PREVIEW",
            container(preview_canvas)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        ))
        .width(Length::FillPortion(1))
        .height(Length::Fill);

        row![mod_panel, preview_panel].spacing(8).into()
    }

    /// Part sub-tab strip across the top of the MOD page, mirroring
    /// the CC MAPPING strip's framed look so the two pages read as
    /// siblings.
    fn mod_part_strip(&self) -> Element<'_, Message> {
        let active = self.modulation.active_part;
        let tabs: [(u8, &'static str, iced::Color); 4] = [
            (0, "FM", Mode::Fm.color()),
            (1, "HARMONIC", Mode::Harmonic.color()),
            (2, "TIMBRAL", Mode::Timbral.color()),
            (3, "GRANULAR", Mode::Granular.color()),
        ];
        let strip = tabs
            .iter()
            .fold(row![].spacing(0), |r, (idx, label, color)| {
                let is_active = *idx == active;
                let label_color = if is_active { TEXT_BRIGHT } else { TEXT_DIM };
                let pill_color = *color;
                r.push(
                    button(
                        container(text(*label).size(10).color(label_color))
                            .center_x(Length::Fill)
                            .padding([2, 0]),
                    )
                    .width(Length::Fill)
                    .padding([4, 14])
                    .on_press(Message::SwitchModPart(*idx))
                    .style(move |_t, _s| tab_btn_style(is_active, pill_color)),
                )
            });
        container(strip)
            .width(Length::Fill)
            .padding([2, 12])
            .style(|_t: &Theme| container::Style {
                background: Some(FX_TAB_TRAY_BG.into()),
                border: iced::Border {
                    color: RULE_LINE,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    /// Render one LFO row: section header + SHAPE pick_list, RATE
    /// pick_list, MODE toggle. Each control dispatches a Message
    /// which updates the local snapshot and ships SetLfo (with the
    /// full triple) to the engine.
    fn lfo_section(&self, part: u8, lfo: u8, name: &'static str) -> Element<'_, Message> {
        let s = self.modulation.lfo_state[part as usize][lfo as usize];
        let mode_color = match part {
            0 => Mode::Fm.color(),
            1 => Mode::Harmonic.color(),
            2 => Mode::Timbral.color(),
            _ => Mode::Granular.color(),
        };

        // Section header — small caps muted with a hairline rule on
        // either side, same shape as engine sub-tab section labels.
        let header = container(
            row![
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
                container(text(name).size(10).color(mode_color)).padding([0, 12]),
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
            ]
            .align_y(Alignment::Center),
        )
        .padding([4, 0])
        .width(Length::Fill);

        let label_widget = |t: &'static str| -> Element<'_, Message> {
            text(t)
                .width(Length::Fixed(80.0))
                .size(11)
                .color(LABEL_MUTED)
                .into()
        };

        // SHAPE pick_list — 24 names; reuse the dark pick_list style
        // so it sits inside the panel ground.
        let shape_options: Vec<&'static str> = SHAPE_NAMES.to_vec();
        let shape_selected = SHAPE_NAMES[s.shape_idx];
        let shape_picker = pick_list(
            shape_options,
            Some(shape_selected),
            move |sel: &'static str| {
                let idx = SHAPE_NAMES.iter().position(|n| *n == sel).unwrap_or(0);
                Message::SetLfoShape { part, lfo, idx }
            },
        )
        .text_size(11)
        .padding([3, 10])
        .style(pick_list_style)
        .menu_style(pick_list_menu_style);
        let shape_row = row![label_widget("SHAPE"), shape_picker]
            .spacing(12)
            .align_y(Alignment::Center);

        // RATE — continuous slider, log-mapped over 0.05..16 Hz so
        // the slider feels even from molasses-slow to audio-rate
        // without giving most of the travel to high frequencies.
        let rate_ratio = rate_to_ratio(s.rate_hz);
        let rate_bar = ratio_bar::ratio_bar(rate_ratio, mode_color, move |r| Message::SetLfoRate {
            part,
            lfo,
            ratio: r,
        })
        .width(Length::Fill)
        .height(Length::Fixed(28.0));
        let rate_readout = text(format!("{:.2} Hz", s.rate_hz))
            .size(11)
            .color(TEXT_BRIGHT)
            .font(iced::Font::MONOSPACE)
            .width(Length::Fixed(80.0));
        let rate_row = row![label_widget("RATE"), rate_bar, rate_readout]
            .spacing(12)
            .align_y(Alignment::Center);

        // MODE toggle — same dimensions in both states so the row
        // doesn't reflow on tap.
        let mode_active = s.mode_loop;
        let mode_label = if mode_active { "LOOP" } else { "TRIG" };
        let mode_btn = button(
            container(text(mode_label).size(11).color(if mode_active {
                TEXT_BRIGHT
            } else {
                TEXT_DIM
            }))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(Length::Fixed(120.0))
        .height(Length::Fixed(28.0))
        .padding(0)
        .on_press(Message::ToggleLfoMode { part, lfo })
        .style(move |_t, _s| {
            let bg = if mode_active {
                Some(
                    iced::Color {
                        a: 0.18,
                        ..mode_color
                    }
                    .into(),
                )
            } else {
                Some(iced::Color::TRANSPARENT.into())
            };
            let border_color = if mode_active {
                mode_color
            } else {
                iced::Color {
                    a: 0.30,
                    ..mode_color
                }
            };
            button::Style {
                background: bg,
                text_color: if mode_active { TEXT_BRIGHT } else { TEXT_DIM },
                border: iced::Border {
                    color: border_color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            }
        });
        let mode_row = row![label_widget("MODE"), mode_btn]
            .spacing(12)
            .align_y(Alignment::Center);

        column![header, shape_row, rate_row, mode_row]
            .spacing(8)
            .into()
    }

    /// Round-trip an assignment edit through Remove + Add at the
    /// same index. Engine has no "change source/dest" message and
    /// appends new assignments at the end, so removing index N then
    /// re-adding lands the new entry at the same N (the only entries
    /// past N just slid down by one then back). Depth carries over.
    pub(crate) fn respin_assignment(&self, part: u8, idx: u8) {
        let p = part as usize;
        let i = idx as usize;
        if p >= 4 || i >= self.modulation.assignments[p].len() {
            return;
        }
        let a = self.modulation.assignments[p][i].clone();
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::RemoveAssignment { part, index: idx }
        );
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::AddAssignment {
                part,
                source: SOURCE_OPTIONS[a.source].to_string(),
                dest: DEST_OPTIONS[a.dest].to_string(),
                depth: a.depth,
            }
        );
    }

    /// Render the ROUTING section — header + per-assignment row +
    /// "+ ADD ROUTING" button. Each row is
    /// `[src] > [dest]  [bipolar depth bar]  [value]  [×]`. Tap on
    /// `src` or `dest` cycles the option; drag the bar to set depth;
    /// × removes the row.
    fn routing_section(&self, part: u8) -> Element<'_, Message> {
        let mode_color = match part {
            0 => Mode::Fm.color(),
            1 => Mode::Harmonic.color(),
            2 => Mode::Timbral.color(),
            _ => Mode::Granular.color(),
        };

        let header = container(
            row![
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
                container(text("ROUTING").size(10).color(mode_color)).padding([0, 12]),
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
            ]
            .align_y(Alignment::Center),
        )
        .padding([4, 0])
        .width(Length::Fill);

        let mut col = column![header].spacing(6);
        let assignments = &self.modulation.assignments[part as usize];
        if assignments.is_empty() {
            col = col.push(
                container(
                    text("No routings yet — tap + ADD ROUTING below.")
                        .size(10)
                        .color(LABEL_MUTED),
                )
                .padding([4, 4]),
            );
        }
        for (i, a) in assignments.iter().enumerate() {
            let idx = i as u8;
            let src_label = SOURCE_OPTIONS[a.source];
            let dest_label = DEST_OPTIONS[a.dest];

            let src_btn = button(
                text(src_label)
                    .size(11)
                    .color(TEXT_DIM)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 6])
            .on_press(Message::CycleAssignmentSource { part, idx })
            .style(|_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: TEXT_DIM,
                border: iced::Border {
                    color: iced::Color {
                        a: 0.20,
                        ..iced::Color::WHITE
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });

            let dest_btn = button(
                text(dest_label)
                    .size(11)
                    .color(mode_color)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 6])
            .on_press(Message::CycleAssignmentDest { part, idx })
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: mode_color,
                border: iced::Border {
                    color: iced::Color {
                        a: 0.30,
                        ..mode_color
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });

            let bar = assign_depth_bar::assign_depth_bar(a.depth, mode_color, part, idx)
                .width(Length::Fill)
                .height(Length::Fixed(22.0));

            let value_text = text(format!("{:+.2}", a.depth))
                .size(10)
                .color(TEXT_BRIGHT)
                .font(iced::Font::MONOSPACE)
                .width(Length::Fixed(48.0));

            let remove_color = iced::Color::from_rgb(0.67, 0.20, 0.20);
            let remove_btn = button(
                container(text("×").size(13).color(remove_color))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(20.0))
            .padding(0)
            .on_press(Message::RemoveRouting { part, idx })
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: remove_color,
                border: iced::Border {
                    color: iced::Color {
                        a: 0.40,
                        ..remove_color
                    },
                    width: 1.0,
                    radius: 3.0.into(),
                },
                shadow: iced::Shadow::default(),
            });

            let arrow = text(">")
                .size(12)
                .color(LABEL_MUTED)
                .width(Length::Fixed(14.0))
                .align_x(iced::alignment::Horizontal::Center);

            col = col.push(
                row![src_btn, arrow, dest_btn, bar, value_text, remove_btn]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
        }

        let add_btn = button(
            container(
                text("+ ADD ROUTING")
                    .size(11)
                    .color(mode_color)
                    .font(iced::Font::MONOSPACE),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(28.0))
        .padding(0)
        .on_press(Message::AddRouting)
        .style(move |_t, _s| button::Style {
            background: Some(iced::Color::TRANSPARENT.into()),
            text_color: mode_color,
            border: iced::Border {
                color: iced::Color {
                    a: 0.30,
                    ..mode_color
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });
        col = col.push(add_btn);
        col.into()
    }

    /// Render a sequencer section — header + 8 step sliders. Each
    /// step row is `S{N}  [───slider───]  0.42`; dragging a slider
    /// updates the local snapshot and dispatches SetSeqStep so the
    /// engine sees the change immediately.
    fn seq_section(&self, part: u8, seq: u8, name: &'static str) -> Element<'_, Message> {
        let values = self.modulation.seq_state[part as usize][seq as usize];
        let mode_color = match part {
            0 => Mode::Fm.color(),
            1 => Mode::Harmonic.color(),
            2 => Mode::Timbral.color(),
            _ => Mode::Granular.color(),
        };

        let header = container(
            row![
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
                container(text(name).size(10).color(mode_color)).padding([0, 12]),
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
            ]
            .align_y(Alignment::Center),
        )
        .padding([4, 0])
        .width(Length::Fill);

        let mut col = column![].spacing(6);
        col = col.push(header);
        for step in 0..8u8 {
            let value = values[step as usize];
            let bar = ratio_bar::ratio_bar(value, mode_color, move |r| Message::SetSeqStep {
                part,
                seq,
                step,
                ratio: r,
            })
            .width(Length::Fill)
            .height(Length::Fixed(22.0));
            let row_widget = row![
                text(format!("S{}", step + 1))
                    .size(11)
                    .color(LABEL_MUTED)
                    .width(Length::Fixed(40.0)),
                bar,
                text(format!("{value:.2}"))
                    .size(10)
                    .color(TEXT_BRIGHT)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(48.0)),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            col = col.push(row_widget);
        }
        col.into()
    }

    /// Drive an LFO param from a 0..=1 ratio. LFO1 spans slots 0..2,
    /// LFO2 spans 3..5; slots 6 and 7 are reserved for DEPTH when
    /// it lands.
    ///
    /// | slot | param      |
    /// |------|------------|
    /// |   0  | LFO1 SHAPE |
    /// |   1  | LFO1 RATE  |
    /// |   2  | LFO1 MODE  |
    /// |   3  | LFO2 SHAPE |
    /// |   4  | LFO2 RATE  |
    /// |   5  | LFO2 MODE  |
    /// |   6  | (reserved) |
    /// |   7  | (reserved) |
    ///
    /// Each handler quantizes the ratio onto the relevant index and
    /// only dispatches `SetLfo` when the quantized bucket changes —
    /// same trick as `apply_fx_knob` for tap params.
    pub(crate) fn apply_mod_knob(&mut self, slot: usize, ratio: f32) {
        let r = ratio.clamp(0.0, 1.0);
        let part = self.modulation.active_part;
        let (lfo, what) = match slot {
            0 => (0u8, ModKnobTarget::Shape),
            1 => (0u8, ModKnobTarget::Rate),
            2 => (0u8, ModKnobTarget::Mode),
            3 => (1u8, ModKnobTarget::Shape),
            4 => (1u8, ModKnobTarget::Rate),
            5 => (1u8, ModKnobTarget::Mode),
            _ => return,
        };
        let s = &mut self.modulation.lfo_state[part as usize][lfo as usize];
        let changed = match what {
            ModKnobTarget::Shape => {
                let idx = quantize_ratio_to_bucket(r, SHAPE_NAMES.len());
                if s.shape_idx == idx {
                    false
                } else {
                    s.shape_idx = idx;
                    true
                }
            }
            ModKnobTarget::Rate => {
                // Continuous now — every CC step maps to a distinct
                // rate via the same log curve as the on-screen
                // slider, so screen + controller agree pixel-for-Hz.
                let new_rate = ratio_to_rate(r);
                // Send only when the change is meaningful; tiny CC
                // jitter shouldn't flood the engine with SetLfo.
                let delta_pct = (new_rate - s.rate_hz).abs() / s.rate_hz.max(0.0001);
                if delta_pct < 0.005 {
                    false
                } else {
                    s.rate_hz = new_rate;
                    true
                }
            }
            ModKnobTarget::Mode => {
                let want = r >= 0.5;
                if s.mode_loop == want {
                    false
                } else {
                    s.mode_loop = want;
                    true
                }
            }
        };
        if changed {
            self.dispatch_set_lfo(part, lfo);
        }
    }

    /// Build a `SetLfo` from the local snapshot for `(part, lfo)`
    /// and ship it to the engine. Each LFO change (shape / rate /
    /// mode) calls this so the engine sees the full triple and we
    /// don't have to invent partial-update messages.
    pub(crate) fn dispatch_set_lfo(&self, part: u8, lfo: u8) {
        let s = self.modulation.lfo_state[part as usize][lfo as usize];
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::SetLfo {
                part,
                lfo,
                shape: SHAPE_NAMES[s.shape_idx].to_string(),
                rate: s.rate_hz,
                mode: if s.mode_loop { "loop" } else { "trig" }.to_string(),
            }
        );
    }
}
