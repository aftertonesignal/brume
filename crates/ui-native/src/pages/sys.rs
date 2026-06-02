// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! SYS page — single SETTINGS panel with stacked sections.
//!
//! Sections: AUDIO OUTPUT (live device picker + Meridian channel
//! map), MIDI (connected device + ports), SYSTEM (telemetry +
//! tempo + voices), DISPLAY (resolution + touch + scale +
//! fullscreen). The audio device picker is the only interactive
//! element on the page; everything else mirrors live state from
//! the engine and the procfs/sysfs telemetry poll on every Tick.

use brume_app_protocol::OutputDeviceEntry;

use iced::widget::{
    Space, button, column, container, horizontal_rule, horizontal_space, row, scrollable, text,
};
use iced::{Alignment, Element, Length, Theme};

use crate::{LABEL_MUTED, RULE_LINE, TEXT_BRIGHT, TEXT_DIM, panel};
use crate::{Message, Mode, NativeUi};

impl NativeUi {
    /// SYS page — single SETTINGS panel with stacked sections
    /// (AUDIO OUTPUT / MIDI / SYSTEM / DISPLAY). Scrolling stays
    /// inside the panel since the whole page is one panel.
    pub(crate) fn sys_page(&self) -> Element<'_, Message> {
        let body = container(
            scrollable(
                column![
                    self.sys_audio_section(),
                    self.sys_midi_section(),
                    self.sys_system_section(),
                    self.sys_display_section(),
                ]
                .spacing(20),
            )
            .height(Length::Fill),
        )
        .padding([16, 18])
        .height(Length::Fill);
        panel("SETTINGS", body.into())
    }

    /// Section heading bar — small caps in the section accent, with
    /// hairline rules either side. Matches the engine sub-tab section
    /// labels so SYS reads like the same family.
    fn sys_section_header(&self, label: &'static str, accent: iced::Color) -> Element<'_, Message> {
        container(
            row![
                horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                    color: RULE_LINE,
                    width: 1,
                    radius: 0.0.into(),
                    fill_mode: iced::widget::rule::FillMode::Full,
                }),
                container(text(label).size(10).color(accent)).padding([0, 12]),
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
        .width(Length::Fill)
        .into()
    }

    /// Static "label · value" SYS row — left-aligned label in muted
    /// caps, right-aligned value in bright text.
    fn sys_static_row(&self, label: &'static str, value: String) -> Element<'_, Message> {
        row![
            text(label)
                .size(11)
                .color(LABEL_MUTED)
                .width(Length::Fixed(120.0)),
            text(value).size(11).color(TEXT_BRIGHT),
            horizontal_space().width(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .into()
    }

    fn sys_audio_section(&self) -> Element<'_, Message> {
        let accent = Mode::Fm.color();
        let mut col = column![self.sys_section_header("AUDIO OUTPUT", accent)].spacing(8);

        // Devices list — active first, then options. Tapping a non-
        // active row dispatches SetOutputDevice; engine echoes back
        // a fresh OutputDeviceList once the swap lands.
        if self.audio.devices.is_empty() {
            col = col.push(self.sys_static_row("DEVICE", "…scanning".to_string()));
        } else {
            // Stable sort: active first.
            let mut sorted: Vec<&OutputDeviceEntry> = self.audio.devices.iter().collect();
            sorted.sort_by_key(|d| if d.id == self.audio.active { 0u8 } else { 1u8 });
            for d in sorted {
                let is_active = d.id == self.audio.active;
                col = col.push(self.sys_audio_device_row(d, is_active, accent));
            }
        }

        // Lower block: channels + rate + format on the left; the latency
        // selector fills the otherwise-empty right half, so it adds no
        // height and no extra scroll to the section.
        let left = column![
            self.sys_channels_block(),
            self.sys_static_row("SAMPLE RATE", "48000 Hz".to_string()),
            self.sys_static_row("FORMAT", "f32 stereo".to_string()),
        ]
        .spacing(8)
        .width(Length::Fill);
        // Trailing Space insets the latency column off the right margin
        // so its right-aligned values clear the scrollable's overlay
        // scrollbar (which covers the last ~14 px of the content width).
        col = col.push(
            row![
                left,
                Space::with_width(Length::Fixed(24.0)),
                self.sys_latency_block(accent),
                Space::with_width(Length::Fixed(26.0)),
            ]
            .align_y(Alignment::Start),
        );
        col.into()
    }

    /// Latency-preset selector. A LOW / BALANCED / SAFE / RELAXED column
    /// (buffer size → output latency tradeoff); the active row is
    /// highlighted, the others dispatch `SetLatencyPreset`. Mirrors the
    /// audio-device row styling so the section reads as one family.
    fn sys_latency_block(&self, accent: iced::Color) -> Element<'_, Message> {
        let header = row![
            text("LATENCY").size(11).color(LABEL_MUTED),
            horizontal_space().width(Length::Fixed(8.0)),
            text("output buffer").size(10).color(TEXT_DIM),
        ]
        .align_y(Alignment::Center);
        let mut col = column![header].spacing(4).width(Length::Fixed(360.0));
        for preset in brume_common::LatencyPreset::all() {
            let is_active = preset == self.audio.latency;
            col = col.push(self.sys_latency_option(preset, is_active, accent));
        }
        col.into()
    }

    fn sys_latency_option(
        &self,
        preset: brume_common::LatencyPreset,
        is_active: bool,
        accent: iced::Color,
    ) -> Element<'_, Message> {
        let dot = if is_active { "●" } else { "○" };
        let frames = preset.frames();
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let ms = (frames as f32 / 48.0).round() as u32;
        let dot_color = if is_active { accent } else { TEXT_DIM };
        let name_color = if is_active {
            accent
        } else {
            iced::Color {
                a: 0.85,
                ..TEXT_DIM
            }
        };

        let inner: Element<'_, Message> = row![
            text(dot)
                .size(12)
                .color(dot_color)
                .width(Length::Fixed(20.0)),
            text(preset.label())
                .size(11)
                .color(name_color)
                .width(Length::Fixed(90.0)),
            horizontal_space().width(Length::Fill),
            text(format!("{frames} smp · {ms} ms"))
                .size(10)
                .color(LABEL_MUTED),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into();

        if is_active {
            container(inner)
                .padding([6, 12])
                .width(Length::Fill)
                .style(move |_t: &Theme| container::Style {
                    background: Some(iced::Color { a: 0.08, ..accent }.into()),
                    ..Default::default()
                })
                .into()
        } else {
            button(container(inner).padding([6, 12]).width(Length::Fill))
                .padding(0)
                .on_press(Message::SetLatencyPreset(preset))
                .style(|_t, _s| button::Style {
                    background: Some(iced::Color::TRANSPARENT.into()),
                    text_color: TEXT_DIM,
                    border: iced::Border::default(),
                    shadow: iced::Shadow::default(),
                })
                .into()
        }
    }

    /// Meridian channel-mapping table: which 8-ch stem pair carries
    /// which Brume part, plus the spatial-position label Apple's
    /// UAC2 driver shows in DAWs. Read-only — the mapping is fixed
    /// at the firmware level. The iChannelNames workaround that
    /// would have shown per-part labels regressed on macOS 26, so
    /// the "documented convention" path wins for now.
    fn sys_channels_block(&self) -> Element<'_, Message> {
        let header = row![
            text("CHANNELS")
                .size(11)
                .color(LABEL_MUTED)
                .width(Length::Fixed(120.0)),
            text("Meridian stems · pair → part · DAW label")
                .size(10)
                .color(TEXT_DIM),
            horizontal_space().width(Length::Fill),
        ]
        .align_y(Alignment::Center);

        let rows: [(&'static str, Mode, &'static str); 4] = [
            ("1–2", Mode::Fm, "Front L / R"),
            ("3–4", Mode::Harmonic, "Front Center / LFE"),
            ("5–6", Mode::Timbral, "Back L / R"),
            ("7–8", Mode::Granular, "Front L / R of Center"),
        ];

        let mut col = column![header].spacing(4);
        for (pair, part_mode, daw) in rows {
            let part_color = part_mode.color();
            col = col.push(
                row![
                    // Indent to align under the CHANNELS hint above.
                    Space::with_width(Length::Fixed(120.0)),
                    text(pair)
                        .size(11)
                        .color(LABEL_MUTED)
                        .font(iced::Font::MONOSPACE)
                        .width(Length::Fixed(48.0)),
                    text("●")
                        .size(11)
                        .color(part_color)
                        .width(Length::Fixed(16.0)),
                    text(part_mode.label())
                        .size(11)
                        .color(part_color)
                        .width(Length::Fixed(96.0)),
                    text(daw).size(10).color(TEXT_DIM),
                ]
                .align_y(Alignment::Center),
            );
        }
        col.into()
    }

    fn sys_audio_device_row(
        &self,
        d: &OutputDeviceEntry,
        is_active: bool,
        accent: iced::Color,
    ) -> Element<'_, Message> {
        let label_text = if is_active { "ACTIVE" } else { "OPTION" };
        let label_color = if is_active { accent } else { LABEL_MUTED };
        let dot = if is_active { "●" } else { "○" };
        let dot_color = if is_active { accent } else { TEXT_DIM };
        let name_color = if is_active {
            accent
        } else {
            iced::Color {
                a: 0.85,
                ..TEXT_DIM
            }
        };

        let inner: Element<'_, Message> = row![
            text(label_text)
                .size(10)
                .color(label_color)
                .width(Length::Fixed(70.0)),
            text(dot)
                .size(12)
                .color(dot_color)
                .width(Length::Fixed(20.0)),
            text(d.label.clone()).size(11).color(name_color),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into();

        if is_active {
            container(inner)
                .padding([6, 12])
                .width(Length::Fill)
                .style(move |_t: &Theme| container::Style {
                    background: Some(iced::Color { a: 0.08, ..accent }.into()),
                    ..Default::default()
                })
                .into()
        } else {
            // Non-active row is tappable. Wrap in a button with
            // transparent style so the visual is unchanged but the
            // press dispatches SwitchAudioDevice. mouse_area would
            // also work; button gives keyboard focus too.
            let id = d.id.clone();
            button(container(inner).padding([6, 12]).width(Length::Fill))
                .padding(0)
                .on_press(Message::SwitchAudioDevice(id))
                .style(|_t, _s| button::Style {
                    background: Some(iced::Color::TRANSPARENT.into()),
                    text_color: TEXT_DIM,
                    border: iced::Border::default(),
                    shadow: iced::Shadow::default(),
                })
                .into()
        }
    }

    fn sys_midi_section(&self) -> Element<'_, Message> {
        let accent = Mode::Harmonic.color();
        let device = if self.midi_device_name.is_empty() {
            "(scanning…)".to_string()
        } else {
            self.midi_device_name.clone()
        };
        column![
            self.sys_section_header("MIDI", accent),
            self.sys_static_row("DEVICE", device.clone()),
            self.sys_static_row("PORTS", device),
        ]
        .spacing(8)
        .into()
    }

    fn sys_system_section(&self) -> Element<'_, Message> {
        let accent = Mode::Timbral.color();
        column![
            self.sys_section_header("SYSTEM", accent),
            self.sys_static_row(
                "VERSION",
                concat!("v", env!("CARGO_PKG_VERSION")).to_string(),
            ),
            self.sys_static_row("ENGINE", "4 parts × 6 voices".to_string()),
            self.sys_static_row("VOICES", format!("{} active", self.audio.voice_count)),
            self.sys_static_row("TEMPO", format!("{:.0} BPM", self.audio.bpm)),
            self.sys_static_row("CLOCK", self.clock_source_label()),
            self.sys_static_row("TEMPERATURE", or_dash(&self.sys_telemetry.temperature),),
            self.sys_static_row("MEMORY", or_dash(&self.sys_telemetry.memory)),
            self.sys_static_row("UPTIME", or_dash(&self.sys_telemetry.uptime)),
        ]
        .spacing(8)
        .into()
    }

    /// Read-only display string for the current clock source. Reads
    /// the authoritative `transport_ui.clock_mode_idx` (same value
    /// the MIDI page's CLOCK source segmented control writes). SYS
    /// shows status; it doesn't dispatch.
    fn clock_source_label(&self) -> String {
        let idx = self.transport_ui.clock_mode_idx;
        crate::CLOCK_SOURCE_LABELS
            .get(idx)
            .copied()
            .unwrap_or("Off")
            .to_string()
    }

    fn sys_display_section(&self) -> Element<'_, Message> {
        let accent = Mode::Granular.color();
        column![
            self.sys_section_header("DISPLAY", accent),
            self.sys_static_row("MONITOR", or_dash(&self.sys_telemetry.display)),
            self.sys_static_row("TOUCH", or_dash(&self.sys_telemetry.touch)),
            self.sys_static_row("SCALE", format!("{:.3}×", self.scale)),
            self.sys_static_row(
                "FULLSCREEN",
                if self.fullscreen {
                    "on".to_string()
                } else {
                    "off".to_string()
                },
            ),
        ]
        .spacing(8)
        .into()
    }
}

/// Render an empty SYS telemetry value as `--` so blank rows still
/// have a visual anchor while a poll lands.
fn or_dash(s: &str) -> String {
    if s.is_empty() {
        "--".to_string()
    } else {
        s.to_string()
    }
}
