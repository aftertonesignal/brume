// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MIDI page — channel routing + CC MAPPING + Learn flow.
//!
//! Side-by-side layout: MIDI ACTIVITY (compact 4-row table) on the
//! left, CC MAPPING (sub-tabs + binding list + LEARN flow) on the
//! right. The activity panel mirrors `MidiActivity` and the
//! `ControlMatrix.channel_map`; the mapping panel reads + mutates
//! `ControlMatrix.bindings` and `ControlMatrix.fx_bindings` directly.

use iced::widget::{Space, button, column, container, horizontal_rule, pick_list, row, text};
use iced::{Alignment, Element, Length, Theme};

use crate::tabs;
use crate::{
    CLOCK_SOURCE_LABELS, FX_TAB_TRAY_BG, LABEL_MUTED, LEARN_AMBER, LEARN_GREEN, LEARN_RED,
    RULE_LINE, TEXT_BRIGHT, TEXT_DIM, menu_btn_style, mix, panel, pick_list_menu_style,
    pick_list_style, ratio_bar, tab_btn_style,
};
use crate::{LearnPhase, Message, Mode, NativeUi};

/// Options shown in the MIDI page's channel pick_list, in display
/// order. Index 0 = "NONE"; indices 1..=16 are 1-indexed MIDI channels.
/// `parse_channel_option` is the inverse mapping.
const CHANNEL_OPTIONS: [&str; 17] = [
    "NONE", "CH 1", "CH 2", "CH 3", "CH 4", "CH 5", "CH 6", "CH 7", "CH 8", "CH 9", "CH 10",
    "CH 11", "CH 12", "CH 13", "CH 14", "CH 15", "CH 16",
];

/// Convert a `CHANNEL_OPTIONS` label back to its 0-indexed MIDI
/// channel (`Some(0)` for "CH 1", `Some(15)` for "CH 16") or `None`
/// for the "NONE" entry / any unrecognized input. The pick_list
/// callback runs this on the user's selection before dispatching
/// `Message::SetChannelForPart`.
fn parse_channel_option(label: &str) -> Option<u8> {
    label
        .strip_prefix("CH ")
        .and_then(|n| n.parse::<u8>().ok())
        .and_then(|n| {
            if (1..=16).contains(&n) {
                Some(n - 1)
            } else {
                None
            }
        })
}

impl NativeUi {
    /// MIDI page body. Header shows the connected device (or "NO MIDI
    /// DEVICE" while idle); below sits one row per part with the
    /// assigned channel pill and a live note-count readout drawn
    /// from `MidiActivity::channels[ch].note_velocities`.
    pub(crate) fn midi_page(&self) -> Element<'_, Message> {
        let device = if self.midi_device_name.is_empty() {
            "NO MIDI DEVICE".to_string()
        } else {
            self.midi_device_name.clone()
        };
        let connected = !self.midi_device_name.is_empty();
        let dot_color = if connected {
            iced::Color::from_rgb(0.0, 0.80, 0.40)
        } else {
            LABEL_MUTED
        };
        let dot = container(Space::with_width(Length::Fill))
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(move |_t: &Theme| container::Style {
                background: Some(dot_color.into()),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            });
        let header = container(
            row![
                dot,
                Space::with_width(Length::Fixed(10.0)),
                text(device)
                    .size(11)
                    .color(if connected { TEXT_BRIGHT } else { LABEL_MUTED }),
            ]
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .width(Length::Fill);

        let column_headers = container(
            row![
                text("ENGINE")
                    .size(9)
                    .color(LABEL_MUTED)
                    .width(Length::Fixed(120.0)),
                text("MIDI CH")
                    .size(9)
                    .color(LABEL_MUTED)
                    .width(Length::Fixed(110.0)),
                text("STATUS")
                    .size(9)
                    .color(LABEL_MUTED)
                    .width(Length::Fill),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([4, 14]);

        let rows = column![
            self.midi_part_row(Mode::Fm),
            self.midi_part_row(Mode::Harmonic),
            self.midi_part_row(Mode::Timbral),
            self.midi_part_row(Mode::Granular),
        ]
        .spacing(2);

        let activity_body = column![
            header,
            container(horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }))
            .padding([0, 0]),
            column_headers,
            container(rows).padding([4, 14]),
            container(horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }))
            .padding(iced::Padding {
                top: 8.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            }),
            container(self.clock_section()).padding(iced::Padding {
                top: 12.0,
                right: 14.0,
                bottom: 8.0,
                left: 14.0,
            }),
        ]
        .spacing(0);
        // Side-by-side reads better than the stacked layout: ACTIVITY
        // is a compact 4-row table that left enormous vertical
        // whitespace below it; CC MAPPING wants the vertical room.
        // MIDI page split: ACTIVITY left, CC MAPPING right.
        let activity_panel = container(panel("MIDI", activity_body.into()))
            .width(Length::FillPortion(1))
            .height(Length::Fill);
        let cc_panel = container(panel("CC MAPPING", self.cc_mapping_body()))
            .width(Length::FillPortion(2))
            .height(Length::Fill);
        row![activity_panel, cc_panel].spacing(8).into()
    }

    /// Body of the CC MAPPING panel: sub-tab strip on top, the LEARN
    /// card (when armed or after capture) above the binding list,
    /// and a LEARN button toolbar at the bottom.
    fn cc_mapping_body(&self) -> Element<'_, Message> {
        let mut list_col = column![].spacing(8);
        if !matches!(self.learn.phase, LearnPhase::Idle) {
            list_col = list_col.push(self.learn_card());
        }
        list_col = list_col.push(self.cc_binding_list());

        column![
            self.cc_tabs_strip(),
            container(list_col).height(Length::Fill).padding([8, 12]),
            self.cc_toolbar(),
        ]
        .spacing(0)
        .into()
    }

    /// Toolbar at the bottom of CC MAPPING. LEARN on the left,
    /// RESET on the right.
    ///
    /// LEARN border + glyph flip to red while armed so the user has
    /// an unambiguous "actively listening" cue distinct from the
    /// amber banner above; default state is green to read as
    /// "ready".
    ///
    /// RESET uses a two-tap pattern: first tap fills the button
    /// solid red with "TAP AGAIN"; second tap (within
    /// `RESET_DISARM`) calls `reset_to_defaults`. Outer container
    /// has extra bottom padding so the buttons lift off the panel
    /// border instead of sitting flush.
    fn cc_toolbar(&self) -> Element<'_, Message> {
        let armed = !matches!(self.learn.phase, LearnPhase::Idle);
        let learn_label = if armed { "LISTENING…" } else { "LEARN" };
        let learn_accent = if armed { LEARN_RED } else { LEARN_GREEN };
        let learn_btn = button(
            container(text(learn_label).size(11).color(learn_accent))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(32.0))
        .padding(0)
        .on_press(Message::ToggleLearn)
        .style(move |_t, _s| button::Style {
            background: Some(iced::Color::TRANSPARENT.into()),
            text_color: learn_accent,
            border: iced::Border {
                color: learn_accent,
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });

        let reset_armed = self.learn.reset_armed_at.is_some();
        let reset_label = if reset_armed { "TAP AGAIN" } else { "RESET" };
        let (reset_text_color, reset_bg, reset_border): (
            iced::Color,
            Option<iced::Background>,
            iced::Color,
        ) = if reset_armed {
            (
                iced::Color::WHITE,
                Some(LEARN_RED.into()),
                iced::Color::from_rgb(1.0, 0.40, 0.40),
            )
        } else {
            (LEARN_RED, Some(iced::Color::TRANSPARENT.into()), LEARN_RED)
        };
        let reset_btn = button(
            container(text(reset_label).size(11).color(reset_text_color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(32.0))
        .padding(0)
        .on_press(Message::ResetTap)
        .style(move |_t, _s| button::Style {
            background: reset_bg,
            text_color: reset_text_color,
            border: iced::Border {
                color: reset_border,
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });

        // Asymmetric padding (more on bottom) so the buttons lift
        // off the panel's bottom border instead of sitting flush
        // against it. iced::Padding's `From` impls only cover
        // single value and `[vertical, horizontal]`, so the four-
        // sided shape is built explicitly.
        container(
            row![learn_btn, reset_btn]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding(iced::Padding {
            top: 8.0,
            right: 12.0,
            bottom: 14.0,
            left: 12.0,
        })
        .width(Length::Fill)
        .into()
    }

    /// Render the LEARN card. Two states:
    ///   - Listening: amber banner with the prompt copy.
    ///   - Detected: amber-bordered card with the captured CC summary,
    ///     a destination grid, and CANCEL / SAVE BINDING buttons.
    fn learn_card(&self) -> Element<'_, Message> {
        match self.learn.phase {
            LearnPhase::Idle => container(text("")).into(),
            LearnPhase::Listening => self.learn_listening_card(),
            LearnPhase::Detected { channel, cc, value } => {
                self.learn_detected_card(channel, cc, value)
            }
        }
    }

    fn learn_listening_card(&self) -> Element<'_, Message> {
        let dot = text("●").size(11).color(LEARN_AMBER);
        let label = text("LISTENING FOR MIDI CC… (turn a knob or tap LEARN to cancel)")
            .size(11)
            .color(LEARN_AMBER);
        container(
            row![dot, Space::with_width(Length::Fixed(10.0)), label].align_y(Alignment::Center),
        )
        .padding([10, 12])
        .width(Length::Fill)
        .style(|_t: &Theme| container::Style {
            background: Some(
                iced::Color {
                    a: 0.05,
                    ..LEARN_AMBER
                }
                .into(),
            ),
            border: iced::Border {
                color: LEARN_AMBER,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn learn_detected_card(&self, channel: u8, cc: u8, value: u8) -> Element<'_, Message> {
        // Summary row: DETECTED tag + CH N + CC NN + val V.
        let summary = row![
            text("DETECTED").size(11).color(LEARN_AMBER),
            Space::with_width(Length::Fixed(14.0)),
            text(format!("CH {}", channel + 1)).size(12).color(TEXT_DIM),
            Space::with_width(Length::Fixed(14.0)),
            text(format!("CC {cc}")).size(12).color(TEXT_BRIGHT),
            Space::with_width(Length::Fixed(14.0)),
            text(format!("val {value}")).size(11).color(LABEL_MUTED),
        ]
        .align_y(Alignment::Center);

        // Destination grid — 4 pills per row so a typical engine list
        // (~12-15 dests) fits in 3-4 rows. Pills inherit the active
        // tab's accent on the selected option.
        let dests = self.cc_destinations_for_active_tab();
        let accent = self.cc_tab_accent();
        let selected = self.learn.selected_dest.clone();
        const PER_ROW: usize = 4;
        let mut grid = column![].spacing(4);
        for chunk in dests.chunks(PER_ROW) {
            let mut r = row![].spacing(4);
            for d in chunk {
                let d_owned = d.clone();
                let active = selected.as_deref() == Some(d.as_str());
                let pill_color = if active { accent } else { TEXT_DIM };
                let border_color = if active {
                    accent
                } else {
                    iced::Color {
                        a: 0.20,
                        ..iced::Color::WHITE
                    }
                };
                r = r.push(
                    button(
                        container(text(d.clone()).size(10).color(pill_color))
                            .center_x(Length::Fill)
                            .padding([2, 0]),
                    )
                    .width(Length::Fill)
                    .padding([4, 8])
                    .on_press(Message::SelectLearnDest(d_owned))
                    .style(move |_t, _s| button::Style {
                        background: Some(iced::Color::TRANSPARENT.into()),
                        text_color: pill_color,
                        border: iced::Border {
                            color: border_color,
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        shadow: iced::Shadow::default(),
                    }),
                );
            }
            // Pad the last row with empty space so partial chunks
            // don't stretch their pills to fill the row width.
            for _ in chunk.len()..PER_ROW {
                r = r.push(Space::with_width(Length::Fill));
            }
            grid = grid.push(r);
        }

        let cancel_btn = button(
            container(text("CANCEL").size(10).color(LABEL_MUTED))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(28.0))
        .padding(0)
        .on_press(Message::CancelLearn)
        .style(|_t, _s| button::Style {
            background: Some(iced::Color::TRANSPARENT.into()),
            text_color: LABEL_MUTED,
            border: iced::Border {
                color: iced::Color {
                    a: 0.30,
                    ..LABEL_MUTED
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });
        let can_save = self.learn.selected_dest.is_some();
        let save_color = if can_save { accent } else { LABEL_MUTED };
        let save_border = if can_save {
            accent
        } else {
            iced::Color {
                a: 0.20,
                ..LABEL_MUTED
            }
        };
        let mut save_btn = button(
            container(text("SAVE BINDING").size(10).color(save_color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(28.0))
        .padding(0)
        .style(move |_t, _s| button::Style {
            background: Some(iced::Color::TRANSPARENT.into()),
            text_color: save_color,
            border: iced::Border {
                color: save_border,
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });
        if can_save {
            save_btn = save_btn.on_press(Message::SaveLearnBinding);
        }

        let actions = row![cancel_btn, save_btn].spacing(8);

        let body = column![
            summary,
            Space::with_width(Length::Fixed(8.0)),
            text("DESTINATION").size(9).color(LABEL_MUTED),
            grid,
            Space::with_width(Length::Fixed(4.0)),
            actions,
        ]
        .spacing(6);

        container(body)
            .padding([10, 12])
            .width(Length::Fill)
            .style(|_t: &Theme| container::Style {
                background: Some(
                    iced::Color {
                        a: 0.05,
                        ..LEARN_AMBER
                    }
                    .into(),
                ),
                border: iced::Border {
                    color: LEARN_AMBER,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    /// Bindable destinations for the active CC MAPPING tab. Engine
    /// tabs walk that engine's ParamSpec lists in `tabs.rs` and
    /// collect unique `ParameterId` variant names. The FX tab walks
    /// `mix::FX_TABS` and emits `Slot:param` strings for every slider
    /// param (tap params aren't usefully driven by a continuous CC).
    fn cc_destinations_for_active_tab(&self) -> Vec<String> {
        match self.active_cc_tab {
            0 | 1 | 2 | 3 => {
                let tabs = match self.active_cc_tab {
                    0 => tabs::FM_TABS,
                    1 => tabs::HARMONIC_TABS,
                    2 => tabs::TIMBRAL_TABS,
                    3 => tabs::GRANULAR_TABS,
                    _ => return vec![],
                };
                let mut seen = std::collections::HashSet::new();
                let mut out = vec![];
                for (_label, specs) in tabs {
                    for spec in *specs {
                        let name = format!("{:?}", spec.binding.id);
                        if seen.insert(name.clone()) {
                            out.push(name);
                        }
                    }
                }
                out
            }
            _ => {
                // FX tab.
                let mut out = vec![];
                for tab in mix::FX_TABS {
                    if let Some(slot) = tab.slot {
                        for spec in tab.params {
                            if matches!(spec.kind, mix::FxParamKind::Slider { .. }) {
                                out.push(format!("{slot}:{}", spec.id));
                            }
                        }
                    }
                }
                out
            }
        }
    }

    /// Accent color for the active CC tab — used for selected
    /// destination pill + SAVE BINDING button border.
    fn cc_tab_accent(&self) -> iced::Color {
        match self.active_cc_tab {
            0 => Mode::Fm.color(),
            1 => Mode::Harmonic.color(),
            2 => Mode::Timbral.color(),
            3 => Mode::Granular.color(),
            _ => iced::Color {
                r: 0.80,
                g: 0.20,
                b: 0.53,
                a: 1.0,
            },
        }
    }

    fn cc_tabs_strip(&self) -> Element<'_, Message> {
        let active = self.active_cc_tab;
        let tabs: [(usize, &'static str, iced::Color); 5] = [
            (0, "FM", Mode::Fm.color()),
            (1, "HARMONIC", Mode::Harmonic.color()),
            (2, "TIMBRAL", Mode::Timbral.color()),
            (3, "GRANULAR", Mode::Granular.color()),
            (
                4,
                "FX",
                iced::Color {
                    r: 0.80,
                    g: 0.20,
                    b: 0.53,
                    a: 1.0,
                },
            ),
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
                    .on_press(Message::SwitchCcTab(*idx))
                    .style(move |_t, _s| tab_btn_style(is_active, pill_color)),
                )
            });
        // Hairline framing on all four sides so the active pill no
        // longer floats untethered against the panel chrome. The
        // border replaces the prior bottom-only `horizontal_rule` —
        // doubling them up made the bottom edge read as 2 px instead
        // of 1.
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

    /// Render the binding rows for the active CC tab. Reads from
    /// `control_matrix` directly each frame — small Vec, cheap iter,
    /// avoids a duplicate cache that could go stale relative to the
    /// shared matrix midi-io is also reading from.
    fn cc_binding_list(&self) -> Element<'_, Message> {
        let Ok(matrix) = self.control_matrix.read() else {
            return text("(matrix unavailable)")
                .size(10)
                .color(LABEL_MUTED)
                .into();
        };
        let active = self.active_cc_tab;
        let mut col = column![].spacing(4);
        let mut shown = 0usize;
        if active < 4 {
            // Engine sub-tab — filter `bindings` by channel_filter
            // (which carries the part index by convention; see the
            // AddCcBinding handler).
            let part = active as u8;
            let mode_color = match active {
                0 => Mode::Fm.color(),
                1 => Mode::Harmonic.color(),
                2 => Mode::Timbral.color(),
                3 => Mode::Granular.color(),
                _ => TEXT_DIM,
            };
            for (idx, b) in matrix.bindings.iter().enumerate() {
                if b.channel_filter != Some(part) {
                    continue;
                }
                col = col.push(cc_binding_row(
                    idx,
                    false,
                    b.cc_number,
                    format!("{:?}", b.destination),
                    b.range_min,
                    b.range_max,
                    mode_color,
                ));
                shown += 1;
            }
        } else {
            // FX sub-tab.
            let fx_color = iced::Color {
                r: 0.80,
                g: 0.20,
                b: 0.53,
                a: 1.0,
            };
            for (idx, b) in matrix.fx_bindings.iter().enumerate() {
                col = col.push(cc_binding_row(
                    idx,
                    true,
                    b.cc_number,
                    b.destination.clone(),
                    b.range_min,
                    b.range_max,
                    fx_color,
                ));
                shown += 1;
            }
        }
        if shown == 0 {
            return container(
                text("No bindings yet — tap LEARN to add one.")
                    .size(10)
                    .color(LABEL_MUTED),
            )
            .padding([8, 4])
            .into();
        }
        col.into()
    }

    /// CLOCK section — embedded into the MIDI ACTIVITY panel below the
    /// per-part channel rows. The MIDI page is the authoritative home
    /// for clock controls (issue #10 phase B). State lives in
    /// `transport_ui`; mutations dispatch `Message::SetClockBpm` and
    /// `Message::SetClockSourceIdx` which both update local state and
    /// emit the corresponding `UiToEngine::SetTempo` / `SetClockMode`.
    fn clock_section(&self) -> Element<'_, Message> {
        let accent = Mode::Harmonic.color();
        let header = container(
            row![
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
                container(text("CLOCK").size(10).color(accent)).padding([0, 12]),
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
            ]
            .align_y(Alignment::Center),
        )
        .padding([0, 0])
        .width(Length::Fill);

        let clock_mode_idx = self.transport_ui.clock_mode_idx;
        let in_sync_mode = clock_mode_idx == 2;

        // BPM control — slider when Off/Master (user's internal target
        // tempo); non-interactive live readout of audio.bpm when Sync
        // (engine following external clock).
        let bpm_row: Element<'_, Message> = if in_sync_mode {
            row![
                text("BPM")
                    .width(Length::Fixed(110.0))
                    .size(11)
                    .color(TEXT_BRIGHT),
                container(
                    text(format!("{:.0}  (synced)", self.audio.bpm))
                        .size(11)
                        .color(TEXT_BRIGHT),
                )
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Left),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        } else {
            const BPM_MIN: f32 = 20.0;
            const BPM_MAX: f32 = 300.0;
            let value = self.transport_ui.bpm;
            let ratio = ((value - BPM_MIN) / (BPM_MAX - BPM_MIN)).clamp(0.0, 1.0);
            row![
                text("BPM")
                    .width(Length::Fixed(110.0))
                    .size(11)
                    .color(TEXT_BRIGHT),
                ratio_bar::ratio_bar(ratio, accent, move |r| {
                    Message::SetClockBpm(BPM_MIN + r * (BPM_MAX - BPM_MIN))
                })
                .width(Length::Fill)
                .height(Length::Fixed(20.0)),
                text(format!("{value:.0}"))
                    .width(Length::Fixed(80.0))
                    .size(11)
                    .color(TEXT_BRIGHT),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        };

        // CLOCK source — three-segment selector. Emits
        // SetClockSourceIdx(i) on tap; selected segment fills with the
        // accent color, others stay outlined.
        let segments =
            CLOCK_SOURCE_LABELS
                .iter()
                .enumerate()
                .fold(row![].spacing(2), |r, (i, name)| {
                    let active = i == clock_mode_idx;
                    let label_color = if active { TEXT_BRIGHT } else { TEXT_DIM };
                    let pill_color = accent;
                    r.push(
                        button(
                            container(text(*name).size(10).color(label_color))
                                .center_x(Length::Fill)
                                .padding([2, 0]),
                        )
                        .width(Length::Fill)
                        .padding([4, 8])
                        .on_press(Message::SetClockSourceIdx(i))
                        .style(move |_t, _s| menu_btn_style(active, pill_color)),
                    )
                });
        let clock_row: Element<'_, Message> = row![
            text("CLOCK")
                .width(Length::Fixed(110.0))
                .size(11)
                .color(TEXT_BRIGHT),
            segments,
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into();

        column![
            header,
            Space::with_height(Length::Fixed(10.0)),
            bpm_row,
            clock_row
        ]
        .spacing(8)
        .into()
    }

    fn midi_part_row(&self, mode: Mode) -> Element<'_, Message> {
        let part = mode.part();
        let assigned = self.midi_part_channel[part as usize];
        let notes = self.midi_part_notes[part as usize];
        let mode_color = mode.color();

        let selected_label: &'static str = match assigned {
            Some(ch) => CHANNEL_OPTIONS
                .get(ch as usize + 1)
                .copied()
                .unwrap_or(CHANNEL_OPTIONS[0]),
            None => CHANNEL_OPTIONS[0],
        };
        let ch_picker = pick_list(
            CHANNEL_OPTIONS.to_vec(),
            Some(selected_label),
            move |sel: &'static str| {
                let channel = parse_channel_option(sel);
                Message::SetChannelForPart { part, channel }
            },
        )
        .text_size(11)
        .padding([3, 10])
        .style(pick_list_style)
        .menu_style(pick_list_menu_style);

        let active = notes > 0;
        let status_color = if active { mode_color } else { LABEL_MUTED };
        let status_dot = container(Space::with_width(Length::Fill))
            .width(Length::Fixed(6.0))
            .height(Length::Fixed(6.0))
            .style(move |_t: &Theme| container::Style {
                background: Some(status_color.into()),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
            });
        let status_text = if active {
            format!("{notes} note{}", if notes == 1 { "" } else { "s" })
        } else {
            String::new()
        };

        row![
            text(mode.label())
                .size(11)
                .color(mode_color)
                .width(Length::Fixed(120.0)),
            container(ch_picker).width(Length::Fixed(110.0)),
            row![
                status_dot,
                Space::with_width(Length::Fixed(8.0)),
                text(status_text).size(10).color(TEXT_DIM),
            ]
            .align_y(Alignment::Center)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    /// Drain the shared `MidiActivity` and `ControlMatrix` into the
    /// local mirror used by the MIDI page. `try_lock` so a momentary
    /// contention with the midi-io writer doesn't stall the UI tick.
    pub(crate) fn poll_midi_activity(&mut self) {
        if let Some(act) = self.midi_activity.try_lock() {
            self.midi_device_name = act.device_name.clone();
            // Walk the assigned channel for each part (read from the
            // matrix below) and count active notes for that channel.
            // Doing this with the activity lock held keeps the read
            // self-consistent — the view sees one snapshot per tick.
            if let Ok(matrix) = self.control_matrix.read() {
                for p in 0..4u8 {
                    let mut assigned: Option<u8> = None;
                    for ch in 0..16u8 {
                        if matrix.channel_map.part_for_channel(ch) == Some(p) {
                            assigned = Some(ch);
                            break;
                        }
                    }
                    self.midi_part_channel[p as usize] = assigned;
                    self.midi_part_notes[p as usize] = match assigned {
                        Some(ch) => act
                            .channels
                            .get(ch as usize)
                            .map(|c| c.note_velocities.iter().filter(|v| **v > 0).count() as u32)
                            .unwrap_or(0),
                        None => 0,
                    };
                }
            }
        }
    }

    /// Apply a channel pick from the MIDI page's pick_list. Steps:
    ///
    ///   1. Unassign whatever channel currently routes to `part`.
    ///   2. If `channel` is `Some(ch)` and `ch` is already routed to
    ///      a different part, unassign that part too — each channel
    ///      can only route to one part at a time.
    ///   3. Set `ch` → `part`.
    ///
    /// Step 2 is what the cycle helper avoided by skipping occupied
    /// slots; with an explicit picker the user is allowed to steal,
    /// which is the more intuitive behavior in a popup selector.
    pub(crate) fn set_channel_for_part(&mut self, part: u8, channel: Option<u8>) {
        let Ok(mut matrix) = self.control_matrix.write() else {
            return;
        };
        for ch in 0..16u8 {
            if matrix.channel_map.part_for_channel(ch) == Some(part) {
                matrix.channel_map.set_channel(ch, None);
            }
        }
        if let Some(ch) = channel {
            if ch < 16 {
                matrix.channel_map.set_channel(ch, Some(part));
            }
        }
    }
}

/// One row in the CC MAPPING binding list. Layout: `CC N → DEST
/// min–max [×]`. The × button is a small red pill that emits
/// `Message::RemoveCcBinding`.
fn cc_binding_row(
    idx: usize,
    fx: bool,
    cc: u8,
    dest: String,
    range_min: f32,
    range_max: f32,
    accent: iced::Color,
) -> Element<'static, Message> {
    let cc_label = text(format!("CC {cc}"))
        .size(11)
        .color(accent)
        .font(iced::Font::MONOSPACE)
        .width(Length::Fixed(60.0));
    let arrow = text("→")
        .size(11)
        .color(LABEL_MUTED)
        .width(Length::Fixed(18.0));
    let dest_label = text(dest).size(11).color(LABEL_MUTED).width(Length::Fill);
    let range = text(format_range(range_min, range_max))
        .size(10)
        .color(LABEL_MUTED)
        .font(iced::Font::MONOSPACE)
        .width(Length::Fixed(120.0))
        .align_x(iced::alignment::Horizontal::Right);

    let remove_color = iced::Color::from_rgb(0.67, 0.20, 0.20);
    let remove_btn = button(
        container(text("×").size(13).color(remove_color))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(28.0))
    .height(Length::Fixed(20.0))
    .padding(0)
    .on_press(Message::RemoveCcBinding { fx, idx })
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

    row![cc_label, arrow, dest_label, range, remove_btn]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
}

/// Format a binding range as `min–max` with engine-unit precision —
/// integers when both ends are whole, two decimals otherwise.
fn format_range(min: f32, max: f32) -> String {
    let whole = min.fract() == 0.0 && max.fract() == 0.0;
    if whole {
        format!("{min:.1}–{max:.1}")
    } else {
        format!("{min:.2}–{max:.2}")
    }
}
