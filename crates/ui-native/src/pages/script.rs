// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! SCRIPT page — Lua script browser + load/unload controls.
//!
//! Reads `~/brume/scripts/` via `ScriptEngine::list_scripts()` and
//! shows one row per `.lua` file. The active script gets a green
//! RUNNING pill; inactive scripts have a LOAD button. The page
//! also surfaces an UNLOAD button in the toolbar; the actual
//! load/unload roundtrip happens in update() via the
//! `Message::ScriptLoad` / `Message::ScriptUnload` handlers.

use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, row, scrollable, text,
};
use iced::{Alignment, Element, Length};

use crate::ratio_bar;
use crate::{LABEL_MUTED, LEARN_GREEN, LEARN_RED, RULE_LINE, TEXT_BRIGHT, TEXT_DIM, panel};
use crate::{Message, NativeUi};

/// Format a script-declared parameter value for the right-side
/// readout. Unlike engine parameters, script params don't carry
/// a unit hint or scale — pick precision from magnitude so a
/// 0..1 mix knob shows three decimals while a 0..127 MIDI value
/// stays integer.
fn format_param_value(v: f32) -> String {
    let abs = v.abs();
    if abs < 1.0 {
        format!("{v:.3}")
    } else if abs < 10.0 {
        format!("{v:.2}")
    } else if abs < 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    }
}

impl NativeUi {
    /// Re-read the script list and currently-loaded name from
    /// `ScriptEngine`. Called when SCRIPT becomes the active page so
    /// hot-added or hot-deleted files appear without a restart.
    pub(crate) fn refresh_scripts(&mut self) {
        let Some(engine) = &self.script_engine else {
            self.script_list.clear();
            self.loaded_script = None;
            return;
        };
        let Ok(eng) = engine.lock() else { return };
        self.script_list = eng.list_scripts();
        self.loaded_script = eng.loaded_script().map(str::to_string);
    }

    /// Drive the loaded script's parameter at index `slot` from a
    /// 0..=1 ratio, mapped into the user-declared (min, max) range.
    /// Called by the nanoKONTROL2 driver when the SCRIPT page is
    /// active and a knob CC arrives. Out-of-range slots and the
    /// no-script-loaded case both no-op.
    ///
    /// Updates both sides: pushes the new value into the engine's
    /// `script_params` table (so the script's next
    /// `brume.get_param_value(name)` sees it) AND mirrors it into
    /// the local `script_params` view so the on-screen ratio_bar
    /// follows the knob without waiting for the next dispatcher
    /// drain.
    pub(crate) fn apply_script_knob(&mut self, slot: usize, ratio: f32) {
        let Some((name, _label, min, max, _value)) = self.script_params.get(slot).cloned() else {
            return;
        };
        let value = min + ratio.clamp(0.0, 1.0) * (max - min);
        if let Some(engine) = &self.script_engine {
            if let Ok(e) = engine.lock() {
                e.set_script_param(&name, value);
            }
        }
        if let Some(p) = self.script_params.get_mut(slot) {
            p.4 = value;
        }
    }

    pub(crate) fn script_page(&self) -> Element<'_, Message> {
        let status_color = if self.loaded_script.is_some() {
            LEARN_GREEN
        } else {
            LABEL_MUTED
        };
        let status_text = match &self.loaded_script {
            Some(name) => format!("Running: {name}.lua"),
            None => "No script loaded".to_string(),
        };
        let unload_btn = button(text("UNLOAD").size(11).color(LEARN_RED))
            .padding([4, 14])
            .on_press(Message::ScriptUnload)
            .style(|_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: LEARN_RED,
                border: iced::Border {
                    color: LEARN_RED,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });
        let toolbar = row![
            text(status_text).size(11).color(status_color),
            horizontal_space().width(Length::Fill),
            unload_btn,
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let mut list_col = column![].spacing(0);
        if self.script_engine.is_none() {
            list_col = list_col.push(
                container(
                    text("Lua scripting unavailable on this build.")
                        .size(11)
                        .color(LABEL_MUTED),
                )
                .padding([16, 4]),
            );
        } else if self.script_list.is_empty() {
            list_col = list_col.push(
                container(
                    text("No scripts in ~/brume/scripts/")
                        .size(11)
                        .color(LABEL_MUTED)
                        .font(iced::Font::MONOSPACE),
                )
                .padding([16, 4]),
            );
        } else {
            for name in &self.script_list {
                list_col = list_col.push(self.script_row(name));
            }
        }

        let body = column![
            container(toolbar).padding([8, 12]).width(Length::Fill),
            horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }),
            container(scrollable(list_col).height(Length::FillPortion(2)))
                .padding([8, 12])
                .height(Length::FillPortion(2)),
            self.script_params_section(),
        ]
        .spacing(0);
        panel("SCRIPTS", body.into())
    }

    /// Renders the PARAMS section below the script list. Only visible
    /// when a script is loaded AND it has declared at least one
    /// parameter via Lua's `brume.add_param(name, label, min, max,
    /// default)`. Each parameter is one `ratio_bar` widget — same
    /// custom Canvas Program the engine pages use, which handles
    /// both Mouse and Touch events explicitly (the stock iced
    /// `slider` widget falls through finger drags on the CM5
    /// labwc + winit setup). Drag emits `Message::ScriptParamChange`
    /// with the value already mapped from 0..1 ratio space into the
    /// script's user-declared range.
    fn script_params_section(&self) -> Element<'_, Message> {
        if self.loaded_script.is_none() || self.script_params.is_empty() {
            return iced::widget::Space::new(Length::Shrink, Length::Shrink).into();
        }

        let header = container(text("PARAMS").size(11).color(LEARN_GREEN))
            .padding([10, 12])
            .width(Length::Fill);

        let mut col = column![].spacing(8);
        for (name, label, min, max, value) in &self.script_params {
            let display_label = if label.is_empty() { name } else { label };
            // Linear ratio mapping. Degenerate range (max == min)
            // collapses to 0 rather than dividing by zero — the
            // bar renders empty and on_change emits min on any
            // press, which round-trips as a no-op.
            let span = max - min;
            let ratio = if span.abs() > f32::EPSILON {
                ((value - min) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let n = name.clone();
            let lo = *min;
            let hi = *max;
            let label_widget = text(display_label.to_string())
                .size(11)
                .color(TEXT_DIM)
                .width(Length::Fixed(110.0))
                .font(iced::Font::MONOSPACE);
            let bar =
                ratio_bar::ratio_bar(ratio, LEARN_GREEN, move |r| Message::ScriptParamChange {
                    name: n.clone(),
                    value: lo + r * (hi - lo),
                })
                .width(Length::Fill)
                .height(Length::Fixed(20.0));
            let value_widget = text(format_param_value(*value))
                .size(11)
                .color(TEXT_BRIGHT)
                .width(Length::Fixed(80.0))
                .font(iced::Font::MONOSPACE)
                .align_x(iced::alignment::Horizontal::Right);
            col = col.push(
                row![label_widget, bar, value_widget]
                    .spacing(12)
                    .align_y(Alignment::Center),
            );
        }

        column![
            horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }),
            header,
            // Bottom padding (24) puts a comfortable gap between
            // the last slider and the system status row that lives
            // in the parent body's footer.
            container(col)
                .padding(iced::Padding {
                    top: 4.0,
                    right: 12.0,
                    bottom: 24.0,
                    left: 12.0,
                })
                .width(Length::Fill),
        ]
        .spacing(0)
        .into()
    }

    fn script_row(&self, name: &str) -> Element<'_, Message> {
        let is_loaded = self.loaded_script.as_deref() == Some(name);
        let label_color = if is_loaded { LEARN_GREEN } else { TEXT_DIM };
        let label = text(format!("{name}.lua"))
            .size(11)
            .color(label_color)
            .font(iced::Font::MONOSPACE)
            .width(Length::Fill);

        let btn_label = if is_loaded { "RUNNING" } else { "LOAD" };
        let btn_color = if is_loaded { LEARN_GREEN } else { TEXT_DIM };
        let btn_border = if is_loaded {
            LEARN_GREEN
        } else {
            iced::Color {
                a: 0.45,
                ..LABEL_MUTED
            }
        };
        let mut btn = button(text(btn_label).size(10).color(btn_color))
            .padding([3, 12])
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: btn_color,
                border: iced::Border {
                    color: btn_border,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });
        if !is_loaded {
            btn = btn.on_press(Message::ScriptLoad(name.to_string()));
        }

        container(row![label, btn].spacing(10).align_y(Alignment::Center))
            .padding([6, 4])
            .into()
    }
}
