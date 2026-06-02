// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MIX page — per-part faders, mute/solo, master strip, FX panel.
//!
//! Top half of the page is the MIXER panel (4 part strips + MASTER);
//! bottom half is the EFFECTS panel hosting the FX sub-tab strip and
//! the active tab's parameter rows. The FX tabs themselves are
//! `crate::mix::FX_TABS` data, distinct from this view module.
//!
//! The state-mutating helpers (toggle_mix_mute / toggle_mix_solo /
//! apply_fx_knob) live here too. They're called from the chassis
//! `update()` dispatcher and from the `ControlSurfaceApi` impl in
//! `lib.rs`; both call sites land on the same logic.

use brume_app_protocol::UiToEngine;
use brume_common::try_send_or_log;

use iced::widget::{
    Space, button, column, container, horizontal_rule, horizontal_space, pick_list, row, text,
};
use iced::{Alignment, Element, Length, Theme};

use crate::{
    BRAND_AMBER, FX_TAB_TRAY_BG, LABEL_MUTED, RULE_LINE, SEGMENTED_MAX, TEXT_BRIGHT, TEXT_DIM,
    apply_scale, format_db, format_value, menu_btn_style, mix, panel, pick_list_menu_style,
    pick_list_style, quantize_ratio_to_bucket, ratio_bar, tab_btn_style, unapply_scale,
};
use crate::{Message, Mode, NativeUi};

impl NativeUi {
    /// MIX page body. Top half is the MIXER panel — four part strips
    /// (FM/HARMONIC/TIMBRAL/GRANULAR) plus a static MASTER row. The
    /// FX panel below is a Wave 6b placeholder.
    pub(crate) fn mix_page(&self) -> Element<'_, Message> {
        let strips = column![
            self.mix_strip(Mode::Fm),
            self.mix_strip(Mode::Harmonic),
            self.mix_strip(Mode::Timbral),
            self.mix_strip(Mode::Granular),
            // Hairline rule between the four parts and MASTER — same
            // style as the controls panel's rule above the transport.
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
                        .into()
                    ),
                    ..Default::default()
                }),
            self.mix_master_strip(),
        ]
        .spacing(8);
        let mixer_panel = container(panel("MIXER", container(strips).padding([12, 14]).into()))
            .height(Length::FillPortion(1));
        let fx_panel =
            container(panel("EFFECTS", self.fx_panel_body())).height(Length::FillPortion(1));
        column![mixer_panel, fx_panel].spacing(8).into()
    }

    /// Single MIX strip for one part. M/S buttons on the left, the
    /// part name in mode color, a level fader filling the row's
    /// stretch, and a fixed-width dB readout on the right.
    fn mix_strip(&self, mode: Mode) -> Element<'_, Message> {
        let part = mode.part();
        let mode_color = mode.color();
        let muted = self.mix.mutes[part as usize];
        let solo = self.mix.solo == Some(part);
        let level = self.mix.levels[part as usize];

        // Name color dims when the strip is muted so the row visually
        // recedes from the active strips. Solo'd strip stays bright.
        let name_color = if muted && !solo {
            iced::Color {
                a: 0.4,
                ..mode_color
            }
        } else {
            mode_color
        };

        let fader = ratio_bar::ratio_bar(level, mode_color, move |r| Message::SetMixLevel {
            part,
            ratio: r,
        })
        .width(Length::Fill)
        .height(Length::Fixed(20.0));

        row![
            mix_letter_button("M", muted, BRAND_AMBER, Message::ToggleMixMute { part }),
            mix_letter_button("S", solo, mode_color, Message::ToggleMixSolo { part }),
            text(mode.label())
                .size(11)
                .color(name_color)
                .width(Length::Fixed(96.0)),
            fader,
            text(format_db(level))
                .size(10)
                .color(TEXT_BRIGHT)
                .font(iced::Font::MONOSPACE)
                .width(Length::Fixed(80.0))
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    /// MASTER strip — non-interactive for now. A bus level meter
    /// + master fader come in a later wave; this just labels the
    /// summing point so the MIXER panel reads as complete.
    fn mix_master_strip(&self) -> Element<'_, Message> {
        row![
            // Spacer matching the M+S column on part strips so MASTER
            // aligns with the part names. (28+28+10*2=74; columns
            // including spacing are 28+28 plus the row spacing of 10
            // around each cell, so reserving 76 lines them up.)
            Space::with_width(Length::Fixed(76.0)),
            text("MASTER")
                .size(11)
                .color(TEXT_BRIGHT)
                .width(Length::Fixed(96.0)),
            horizontal_space().width(Length::Fill),
            text("0.0 dB")
                .size(10)
                .color(TEXT_BRIGHT)
                .font(iced::Font::MONOSPACE)
                .width(Length::Fixed(80.0))
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    /// Body of the EFFECTS panel: FX sub-tab strip on top, then the
    /// active tab's parameter rows. Title is implied by which tab is
    /// active (the strip itself reads as the heading).
    fn fx_panel_body(&self) -> Element<'_, Message> {
        column![
            self.fx_tabs_strip(),
            container(self.fx_params())
                .height(Length::Fill)
                .padding([8, 12]),
        ]
        .spacing(0)
        .into()
    }

    /// Sub-tab strip across the FX panel header. One button per
    /// `mix::FX_TABS` entry plus a final LUA button for the
    /// user-loaded Lua FX slot. Active tab gets its accent color as
    /// a low-alpha pill backing. Wrapped in a slightly darker
    /// container background than `PANEL_BG` so the strip reads as a
    /// header tray distinct from the param body.
    fn fx_tabs_strip(&self) -> Element<'_, Message> {
        let active = self.mix.active_fx_tab;
        let mut strip = mix::FX_TABS
            .iter()
            .enumerate()
            .fold(row![].spacing(0), |r, (i, tab)| {
                let is_active = i == active;
                let pill_color = tab.color;
                let label_color = if is_active { TEXT_BRIGHT } else { TEXT_DIM };
                r.push(
                    button(
                        container(text(tab.title).size(10).color(label_color))
                            .center_x(Length::Fill)
                            .padding([2, 0]),
                    )
                    .width(Length::Fill)
                    .padding([4, 14])
                    .on_press(Message::SwitchFxTab(i))
                    .style(move |_t, _s| tab_btn_style(is_active, pill_color)),
                )
            });
        // LUA tab — separate from FX_TABS because its params are
        // dynamic (declared by whatever script the user loaded into
        // the slot), which doesn't fit FX_TABS' static `&'static
        // [FxParamSpec]` shape.
        let lua_active = active == crate::LUA_FX_TAB_INDEX;
        let lua_color = crate::LEARN_GREEN;
        let lua_label_color = if lua_active { TEXT_BRIGHT } else { TEXT_DIM };
        strip = strip.push(
            button(
                container(text("LUA").size(10).color(lua_label_color))
                    .center_x(Length::Fill)
                    .padding([2, 0]),
            )
            .width(Length::Fill)
            .padding([4, 14])
            .on_press(Message::SwitchFxTab(crate::LUA_FX_TAB_INDEX))
            .style(move |_t, _s| tab_btn_style(lua_active, lua_color)),
        );
        // Same hairline framing as the CC MAPPING strip — Border on
        // all four sides keeps the active pill anchored against the
        // panel chrome instead of floating to the panel edges.
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

    /// Param rows for the active FX tab. Dispatches to the LUA tab
    /// renderer when `active_fx_tab == LUA_FX_TAB_INDEX`; otherwise
    /// renders the matching `FX_TABS` entry's static params.
    fn fx_params(&self) -> Element<'_, Message> {
        let tab_idx = self.mix.active_fx_tab;
        if tab_idx == crate::LUA_FX_TAB_INDEX {
            return self.lua_fx_body();
        }
        let Some(tab) = mix::FX_TABS.get(tab_idx).copied() else {
            return Space::with_width(Length::Fill).into();
        };

        let rows =
            tab.params
                .iter()
                .enumerate()
                .fold(column![].spacing(8), |col, (param_idx, spec)| {
                    col.push(self.fx_param_row(tab_idx, param_idx, spec, tab.color, LABEL_MUTED))
                });

        column![
            // Title bar mirroring engine_params' section header — small
            // caps muted, with hairline rules either side.
            container(
                row![
                    horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                        color: RULE_LINE,
                        width: 1,
                        radius: 0.0.into(),
                        fill_mode: iced::widget::rule::FillMode::Full,
                    }),
                    container(text(tab.title).size(9).color(LABEL_MUTED)).padding([0, 12]),
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

    /// One row in the FX param list. Tap params render `[label][name]`;
    /// slider params render `[label][bar][value+unit]`. Same column
    /// widths as engine param rows so the two pages read consistently.
    /// Visibility is `pub(crate)` because the MIDI page reuses this
    /// renderer to embed the OUTPUT tab's BPM and CLOCK params in its
    /// own CLOCK section — same engine-dispatch path either way.
    /// `label_color` lets each call site pick the param-name foreground
    /// (MIX uses `LABEL_MUTED`; MIDI uses `TEXT_BRIGHT` so BPM and
    /// CLOCK read as white labels next to the channel routing).
    pub(crate) fn fx_param_row(
        &self,
        tab_idx: usize,
        param_idx: usize,
        spec: &mix::FxParamSpec,
        tab_color: iced::Color,
        label_color: iced::Color,
    ) -> Element<'_, Message> {
        let value = self
            .mix
            .fx_values
            .get(tab_idx)
            .and_then(|row| row.get(param_idx))
            .copied()
            .unwrap_or(spec.default_value);
        let label_widget = text(spec.label)
            .width(Length::Fixed(110.0))
            .size(11)
            .color(label_color);

        match spec.kind {
            mix::FxParamKind::Tap(names) => {
                let cur_idx = (value.round() as i64).clamp(0, names.len() as i64 - 1) as usize;
                // Short option lists (TYPE, CLOCK) read best as a
                // segmented row — every option is visible and tappable
                // in one step. Long lists (DELAY/SYNC has 11) blow out
                // the row, so they collapse into a pick_list popover
                // — same data, but compact in the resting state.
                if names.len() <= SEGMENTED_MAX {
                    let segments =
                        names
                            .iter()
                            .enumerate()
                            .fold(row![].spacing(2), |r, (i, name)| {
                                let active = i == cur_idx;
                                let label_color = if active { TEXT_BRIGHT } else { TEXT_DIM };
                                let pill_color = tab_color;
                                r.push(
                                    button(
                                        container(text(*name).size(10).color(label_color))
                                            .center_x(Length::Fill)
                                            .padding([2, 0]),
                                    )
                                    .width(Length::Fill)
                                    .padding([4, 8])
                                    .on_press(Message::SetFxEnum {
                                        tab_idx,
                                        param_idx,
                                        value_idx: i,
                                    })
                                    .style(move |_t, _s| menu_btn_style(active, pill_color)),
                                )
                            });
                    row![label_widget, segments]
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .into()
                } else {
                    let options: Vec<&'static str> = names.to_vec();
                    let selected: Option<&'static str> = names.get(cur_idx).copied();
                    let names_for_lookup = names;
                    let picker = pick_list(options, selected, move |sel: &'static str| {
                        let idx = names_for_lookup.iter().position(|n| *n == sel).unwrap_or(0);
                        Message::SetFxEnum {
                            tab_idx,
                            param_idx,
                            value_idx: idx,
                        }
                    })
                    .text_size(11)
                    .padding([3, 10])
                    .style(pick_list_style)
                    .menu_style(pick_list_menu_style);
                    row![
                        label_widget,
                        container(picker).width(Length::Fill),
                        horizontal_space().width(Length::Fixed(80.0)),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into()
                }
            }
            mix::FxParamKind::Slider { min, max, scale } => {
                let ratio = unapply_scale(scale, value, min, max);
                let readout = if spec.unit.is_empty() {
                    format_value(value)
                } else {
                    format!("{} {}", format_value(value), spec.unit)
                };
                row![
                    label_widget,
                    ratio_bar::ratio_bar(ratio, tab_color, move |r| {
                        Message::SetFxRatio {
                            tab_idx,
                            param_idx,
                            ratio: r,
                        }
                    })
                    .width(Length::Fill)
                    .height(Length::Fixed(20.0)),
                    text(readout)
                        .width(Length::Fixed(80.0))
                        .size(11)
                        .color(TEXT_BRIGHT),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .into()
            }
        }
    }

    /// Clear the active solo and unmute every part the solo was
    /// suppressing. Dispatches `SetPartMute { muted: false }` only
    /// for parts whose state actually flips, so the engine and UI
    /// don't drift over a stream of redundant unmute echoes.
    fn clear_solo_and_unmute_all(&mut self) {
        self.mix.solo = None;
        for p in 0..4u8 {
            if self.mix.mutes[p as usize] {
                self.mix.mutes[p as usize] = false;
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetPartMute {
                        part: p,
                        muted: false
                    }
                );
            }
        }
    }

    /// Flip mute on `part` and dispatch `SetPartMute`. Muting the
    /// currently-soloed part first clears solo (and the mutes solo
    /// imposed) so the user ends up with only `part` muted and no
    /// active solo, rather than a confused combined state.
    pub(crate) fn toggle_mix_mute(&mut self, part: u8) {
        let p = part as usize;
        if p >= self.mix.mutes.len() {
            return;
        }
        if self.mix.solo == Some(part) {
            self.clear_solo_and_unmute_all();
        }
        self.mix.mutes[p] = !self.mix.mutes[p];
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::SetPartMute {
                part,
                muted: self.mix.mutes[p]
            }
        );
    }

    /// Toggle exclusive solo on `part`. Re-engaging the active solo
    /// clears it and unmutes everyone; engaging a fresh solo mutes
    /// every other part.
    pub(crate) fn toggle_mix_solo(&mut self, part: u8) {
        if (part as usize) >= self.mix.mutes.len() {
            return;
        }
        if self.mix.solo == Some(part) {
            self.clear_solo_and_unmute_all();
        } else {
            self.mix.solo = Some(part);
            for p in 0..4u8 {
                let want = p != part;
                if self.mix.mutes[p as usize] != want {
                    self.mix.mutes[p as usize] = want;
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetPartMute {
                            part: p,
                            muted: want
                        }
                    );
                }
            }
        }
    }

    /// Drive an FX param from a 0..=1 ratio. `slot` is the knob
    /// index (0..7); maps to the active FX tab's params list. Slider
    /// params scale via the spec's `min`/`max`/`scale`; Tap params
    /// quantize the ratio onto the variant index. OUT-tab special
    /// cases (BPM → SetTempo, CLOCK → SetClockMode) match the touch
    /// path so the engine sees identical messages either way.
    pub(crate) fn apply_fx_knob(&mut self, slot: usize, ratio: f32) {
        let tab_idx = self.mix.active_fx_tab;
        // LUA tab — different param model (dynamic, declared in
        // the loaded script's `fx.params` table). Route to the
        // matching helper so it can map ratio → script-declared
        // (min, max) and dispatch SetFxParam against the stable
        // Lua slot name.
        if tab_idx == crate::LUA_FX_TAB_INDEX {
            self.apply_lua_fx_knob(slot, ratio);
            return;
        }
        let Some(tab) = mix::FX_TABS.get(tab_idx).copied() else {
            return;
        };
        let Some(spec) = tab.params.get(slot) else {
            return;
        };
        let r = ratio.clamp(0.0, 1.0);
        match spec.kind {
            mix::FxParamKind::Slider { min, max, scale } => {
                let value = apply_scale(scale, r, min, max);
                if let Some(row) = self.mix.fx_values.get_mut(tab_idx) {
                    if let Some(stored) = row.get_mut(slot) {
                        *stored = value;
                    }
                }
                if let Some(slot_name) = tab.slot {
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetFxParam {
                            slot: slot_name.to_string(),
                            param: spec.id.to_string(),
                            value,
                        }
                    );
                }
                // Slot-less tabs are gone (issue #10 phase B retired
                // the OUTPUT tab); BPM lives on the MIDI page now.
            }
            mix::FxParamKind::Tap(names) => {
                if names.is_empty() {
                    return;
                }
                let value_idx = quantize_ratio_to_bucket(r, names.len());
                let value = value_idx as f32;
                let prev = self
                    .mix
                    .fx_values
                    .get(tab_idx)
                    .and_then(|row| row.get(slot))
                    .copied();
                // No-op when the knob hasn't crossed into a new bucket
                // — avoids spamming engine messages every frame.
                if prev == Some(value) {
                    return;
                }
                if let Some(row) = self.mix.fx_values.get_mut(tab_idx) {
                    if let Some(stored) = row.get_mut(slot) {
                        *stored = value;
                    }
                }
                if let Some(slot_name) = tab.slot {
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetFxParam {
                            slot: slot_name.to_string(),
                            param: spec.id.to_string(),
                            value,
                        }
                    );
                }
                // Slot-less Tap params are gone (CLOCK retired with
                // OUTPUT tab in issue #10 phase B); CLOCK source
                // dispatch lives on the MIDI page now.
            }
        }
    }

    /// Construct a LuaFxSlot from `~/brume/scripts/fx/<name>.lua`,
    /// stamp the stable engine slot name, wire the audio-thread
    /// errors channel, and ship the boxed slot through `fx_slot_tx`.
    /// Engine-side `process_block` drains and replaces-by-name. The
    /// local `lua_fx_loaded` mirror is populated with param defs +
    /// defaults so the LUA tab can render its sliders without an
    /// engine round-trip.
    pub(crate) fn load_lua_fx(&mut self, script_name: &str) {
        // Resolve the path inside a tight scope so the engine lock
        // is released before any subsequent set_toast (which takes
        // &mut self and would otherwise overlap with the lock
        // guard's borrow of self.script_engine).
        let path_result: Option<Result<std::path::PathBuf, brume_scripting::ScriptError>> = self
            .script_engine
            .as_ref()
            .and_then(|e| e.lock().ok().map(|eng| eng.fx_script_path(script_name)));

        let path = match path_result {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                self.set_toast(format!("FX: {e}"), crate::LEARN_RED);
                return;
            }
            None => {
                self.set_toast("Lua scripting unavailable on this build", crate::LEARN_RED);
                return;
            }
        };

        // sample_rate matches the audio thread's open. Hardcoded
        // 48 kHz today; if cpal's actual rate ever drifts from
        // SAMPLE_RATE, the LuaFxSlot's DSP primitives (delay,
        // lowpass, etc.) would tune off. Plumb a real value
        // through when that becomes a real concern.
        let sample_rate = 48_000.0;

        let mut slot = match brume_scripting::LuaFxSlot::from_file(&path, sample_rate) {
            Ok(s) => s,
            Err(e) => {
                self.set_toast(format!("FX load failed: {e}"), crate::LEARN_RED);
                return;
            }
        };

        if let Some(tx) = self.lua_fx_errors_tx.clone() {
            slot = slot.with_errors_tx(tx);
        }
        slot = slot.with_slot_name(crate::LUA_FX_SLOT_NAME);

        let display_name = slot.display_name().to_string();
        let param_defs: Vec<brume_fx_chain::FxParamDef> = brume_fx_chain::FxSlot::params(&slot)
            .iter()
            .cloned()
            .collect();
        let values: Vec<f32> = param_defs.iter().map(|p| p.default).collect();

        // Push the boxed slot. The engine's process_block drains
        // this channel and replaces-by-name on the audio thread,
        // so a prior Lua FX slot (if any) gets cleanly replaced
        // without a UI-side RemoveFxSlot dance.
        let boxed: Box<dyn brume_fx_chain::FxSlot> = Box::new(slot);
        if let Err(e) = self.fx_slot_tx.try_send(boxed) {
            self.set_toast(format!("FX channel full: {e}"), crate::LEARN_RED);
            return;
        }

        self.lua_fx_loaded = Some(crate::LuaFxLoaded {
            script_name: script_name.to_string(),
            display_name,
            params: param_defs,
            values,
        });
        self.set_toast(format!("FX: {script_name}.lua"), crate::LEARN_GREEN);
    }

    /// Send `RemoveFxSlot` for the stable engine slot name and clear
    /// the local mirror. Tolerant of the no-script-loaded case;
    /// `RemoveFxSlot` against a non-existent slot is a no-op.
    pub(crate) fn unload_lua_fx(&mut self) {
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::RemoveFxSlot(crate::LUA_FX_SLOT_NAME.to_string())
        );
        let toast = self
            .lua_fx_loaded
            .as_ref()
            .map(|l| format!("FX UNLOADED: {}.lua", l.script_name))
            .unwrap_or_else(|| "FX UNLOADED".to_string());
        self.lua_fx_loaded = None;
        self.set_toast(toast, crate::LABEL_MUTED);
    }

    /// Drag handler for a Lua FX param slider. Sends `SetFxParam`
    /// (slot = stable Lua slot name; param = the script's declared
    /// param name) and updates the local values mirror so the bar
    /// follows the finger. FX params don't echo back from the
    /// engine today, so the optimistic local update is the only
    /// source of truth for what the UI shows.
    pub(crate) fn set_lua_fx_param(&mut self, param: &str, value: f32) {
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::SetFxParam {
                slot: crate::LUA_FX_SLOT_NAME.to_string(),
                param: param.to_string(),
                value,
            }
        );
        if let Some(loaded) = self.lua_fx_loaded.as_mut() {
            if let Some(idx) = loaded.params.iter().position(|p| p.name == param) {
                if let Some(slot) = loaded.values.get_mut(idx) {
                    *slot = value;
                }
            }
        }
    }

    /// nanoKONTROL2 / on-screen knob → Lua FX param. Mirrors
    /// `apply_fx_knob` for the LUA tab: `slot` is the knob index
    /// (0..=7), `ratio` is the 0..=1 input from the surface
    /// driver. Linear-maps onto the script-declared (min, max)
    /// range, dispatches `SetFxParam` against the stable slot
    /// name, updates the local `lua_fx_loaded.values` mirror so
    /// the on-screen `ratio_bar` follows the knob without waiting
    /// for an engine echo. No-ops gracefully when no script is
    /// loaded or the slot index exceeds the script's declared
    /// param count.
    pub(crate) fn apply_lua_fx_knob(&mut self, slot: usize, ratio: f32) {
        let r = ratio.clamp(0.0, 1.0);
        let Some(loaded) = self.lua_fx_loaded.as_ref() else {
            return;
        };
        let Some(def) = loaded.params.get(slot) else {
            return;
        };
        let value = def.min + r * (def.max - def.min);
        let param_name = def.name.clone();
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::SetFxParam {
                slot: crate::LUA_FX_SLOT_NAME.to_string(),
                param: param_name.clone(),
                value,
            }
        );
        if let Some(loaded) = self.lua_fx_loaded.as_mut() {
            if let Some(stored) = loaded.values.get_mut(slot) {
                *stored = value;
            }
        }
    }

    /// Refresh `lua_fx_list` from the filesystem. Called when the
    /// user enters the LUA tab so the picker reflects hot-added or
    /// hot-deleted files. Cheap (one `read_dir` + sort).
    pub(crate) fn refresh_lua_fx_list(&mut self) {
        if let Some(engine) = self.script_engine.as_ref() {
            if let Ok(eng) = engine.lock() {
                self.lua_fx_list = eng.list_fx_scripts();
                return;
            }
        }
        self.lua_fx_list.clear();
    }

    /// LUA tab body. Two render states:
    ///
    /// - **Loaded** (`lua_fx_loaded` is `Some`): shows the script's
    ///   display name + UNLOAD button + one `ratio_bar` per param.
    /// - **Empty**: shows a header and a list of `fx/*.lua` files,
    ///   each row tappable to load the script. Empty fx/ directory
    ///   gets a friendly "no scripts" placeholder.
    pub(crate) fn lua_fx_body(&self) -> Element<'_, Message> {
        if let Some(loaded) = self.lua_fx_loaded.as_ref() {
            return self.lua_fx_loaded_body(loaded);
        }
        self.lua_fx_picker_body()
    }

    fn lua_fx_loaded_body(&self, loaded: &crate::LuaFxLoaded) -> Element<'_, Message> {
        let header = row![
            container(
                text(loaded.display_name.clone())
                    .size(11)
                    .color(TEXT_BRIGHT)
            )
            .padding([0, 12]),
            horizontal_space().width(Length::Fill),
            button(text("UNLOAD").size(10).color(crate::LEARN_RED))
                .padding([3, 12])
                .on_press(Message::LuaFxUnload)
                .style(|_t, _s| button::Style {
                    background: Some(iced::Color::TRANSPARENT.into()),
                    text_color: crate::LEARN_RED,
                    border: iced::Border {
                        color: crate::LEARN_RED,
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    shadow: iced::Shadow::default(),
                }),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        // One ratio_bar per declared param. Linear ratio mapping
        // (FxParamDef has no scale today; if a future schema adds
        // log scaling we'd respect it here). Edge case: zero-span
        // params clamp ratio to 0 rather than dividing by zero.
        let mut rows = column![].spacing(8);
        for (idx, def) in loaded.params.iter().enumerate() {
            let value = loaded.values.get(idx).copied().unwrap_or(def.default);
            let span = def.max - def.min;
            let ratio = if span.abs() > f32::EPSILON {
                ((value - def.min) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let lo = def.min;
            let hi = def.max;
            let param_name = def.name.clone();
            let label_widget = text(def.label.to_string())
                .size(11)
                .color(TEXT_DIM)
                .width(Length::Fixed(110.0))
                .font(iced::Font::MONOSPACE);
            let bar = ratio_bar::ratio_bar(ratio, crate::LEARN_GREEN, move |r| {
                Message::LuaFxParamChange {
                    param: param_name.clone(),
                    value: lo + r * (hi - lo),
                }
            })
            .width(Length::Fill)
            .height(Length::Fixed(20.0));
            let unit_suffix = def
                .unit
                .as_deref()
                .map(|u| format!(" {u}"))
                .unwrap_or_default();
            let value_widget = text(format!("{value:.3}{unit_suffix}"))
                .size(11)
                .color(TEXT_BRIGHT)
                .width(Length::Fixed(96.0))
                .font(iced::Font::MONOSPACE)
                .align_x(iced::alignment::Horizontal::Right);
            rows = rows.push(
                row![label_widget, bar, value_widget]
                    .spacing(12)
                    .align_y(Alignment::Center),
            );
        }

        column![
            container(header).padding([4, 0]).width(Length::Fill),
            horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }),
            container(rows)
                .padding(iced::Padding {
                    top: 8.0,
                    right: 12.0,
                    bottom: 12.0,
                    left: 12.0,
                })
                .width(Length::Fill),
        ]
        .spacing(0)
        .into()
    }

    fn lua_fx_picker_body(&self) -> Element<'_, Message> {
        if self.script_engine.is_none() {
            return container(
                text("Lua scripting unavailable on this build.")
                    .size(11)
                    .color(LABEL_MUTED),
            )
            .padding([16, 12])
            .into();
        }

        if self.lua_fx_list.is_empty() {
            return container(
                text("No scripts in ~/brume/scripts/fx/")
                    .size(11)
                    .color(LABEL_MUTED)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([16, 12])
            .into();
        }

        let mut list = column![].spacing(2);
        for name in &self.lua_fx_list {
            let label = text(format!("{name}.lua"))
                .size(11)
                .color(TEXT_DIM)
                .width(Length::Fill)
                .font(iced::Font::MONOSPACE);
            let load_btn = button(text("LOAD").size(10).color(TEXT_DIM))
                .padding([3, 12])
                .on_press(Message::LuaFxSelect(name.clone()))
                .style(|_t, _s| button::Style {
                    background: Some(iced::Color::TRANSPARENT.into()),
                    text_color: TEXT_DIM,
                    border: iced::Border {
                        color: iced::Color {
                            a: 0.45,
                            ..LABEL_MUTED
                        },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    shadow: iced::Shadow::default(),
                });
            list = list.push(
                container(row![label, load_btn].spacing(10).align_y(Alignment::Center))
                    .padding([6, 12]),
            );
        }

        list.into()
    }
}

/// Compact letter-pill button used for M / S on each MIX strip.
/// Active state: solid background + dark glyph; inactive: 1 px outline
/// + dim glyph. Same dimensions in both states so toggling can't shift
/// the row layout.
fn mix_letter_button(
    glyph: &'static str,
    active: bool,
    accent: iced::Color,
    on_press: Message,
) -> Element<'static, Message> {
    let glyph_color = if active {
        iced::Color::from_rgb(0.05, 0.04, 0.03)
    } else {
        TEXT_DIM
    };
    button(
        container(text(glyph).size(10).color(glyph_color))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(28.0))
    .height(Length::Fixed(20.0))
    .padding(0)
    .on_press(on_press)
    .style(move |_t, _s| {
        let background = if active {
            Some(accent.into())
        } else {
            Some(iced::Color::TRANSPARENT.into())
        };
        let border_color = if active {
            accent
        } else {
            iced::Color { a: 0.45, ..accent }
        };
        button::Style {
            background,
            text_color: glyph_color,
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: 3.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    })
    .into()
}
