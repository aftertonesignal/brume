// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! LIBRARY page — patch + perf save/load/delete.
//!
//! Toolbar across the top selects which mode the listing belongs to
//! (FM / HARMONIC / TIMBRAL / GRANULAR / PERF). The PERF mode wraps
//! the whole instrument — all four parts plus FX state plus mixer
//! state — into one named snapshot; the per-engine modes save just
//! that engine's params for the active part.
//!
//! Save/load/delete dispatch through `PatchLibrary` and `Perf::new`
//! from `brume-patch-store`, with Engine-side `SetParameter` /
//! `SetFxParam` / `SetPartLevel` / `SetPartMute` follow-up messages
//! so the engine state matches the on-disk snapshot.

use brume_app_protocol::UiToEngine;
use brume_common::{ParameterId, try_send_or_log};
use brume_patch_store::{FxSnapshot, MixerSnapshot, PartSnapshot, Patch, PatchSource, Perf};

use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, row, scrollable, text,
};
use iced::{Alignment, Element, Length, Theme};

use crate::{
    LABEL_MUTED, LEARN_GREEN, PERF_GOLD, RULE_LINE, TEXT_BRIGHT, TEXT_DIM, mix, panel,
    tab_btn_style,
};
use crate::{Message, Mode, NativeUi, Page};

impl NativeUi {
    /// List the patch (or perf) names for `mode`. PERF resolves to
    /// the dedicated perf store; everything else is a per-engine
    /// patch directory.
    pub(crate) fn list_for_mode(&self, mode: &str) -> Vec<String> {
        if mode == "perf" {
            self.patch_library.list_perfs()
        } else {
            self.patch_library.list(mode)
        }
    }

    /// Snapshot every part's params + FX state + mixer state into a
    /// `Perf` and write it to disk. Auto-named `PERF-NNNN` from the
    /// last 4 digits of the unix timestamp.
    pub(crate) fn save_perf(&mut self) {
        let modes: [(&'static str, Mode); 4] = [
            ("fm", Mode::Fm),
            ("harmonic", Mode::Harmonic),
            ("timbral", Mode::Timbral),
            ("granular", Mode::Granular),
        ];
        let mut parts: [Option<PartSnapshot>; 4] = [None, None, None, None];
        for (idx, (mode_str, mode)) in modes.iter().enumerate() {
            let part = mode.part();
            let mut params: std::collections::BTreeMap<String, f64> =
                std::collections::BTreeMap::new();
            for (_label, specs) in mode.tabs() {
                for spec in *specs {
                    if spec.binding.part != part {
                        continue;
                    }
                    let value = self
                        .param_values
                        .get(&(part, spec.binding.id))
                        .copied()
                        .unwrap_or(spec.default_value);
                    params.insert(format!("{:?}", spec.binding.id), value as f64);
                }
            }
            parts[idx] = Some(PartSnapshot {
                mode: (*mode_str).to_string(),
                params,
                source_patch: None,
            });
        }

        let mut fx_params: std::collections::BTreeMap<String, f64> =
            std::collections::BTreeMap::new();
        for (tab_idx, tab) in mix::FX_TABS.iter().enumerate() {
            let Some(slot) = tab.slot else {
                continue;
            };
            for (param_idx, spec) in tab.params.iter().enumerate() {
                let value = self
                    .mix
                    .fx_values
                    .get(tab_idx)
                    .and_then(|row| row.get(param_idx))
                    .copied()
                    .unwrap_or(spec.default_value);
                fx_params.insert(format!("{slot}:{}", spec.id), value as f64);
            }
        }
        let fx = Some(FxSnapshot {
            params: fx_params,
            source_patch: None,
        });

        let mixer = MixerSnapshot {
            levels: self.mix.levels,
            mutes: self.mix.mutes,
            delay_sends: [0.0; 4],
            reverb_sends: [0.0; 4],
            master_volume: 0.8,
        };

        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() % 10_000)
            .unwrap_or(0);
        let name = format!("PERF-{suffix:04}");
        let perf = Perf::new(name.clone(), parts, fx, mixer);
        if let Err(e) = self.patch_library.save_perf(&perf) {
            eprintln!("brume library: save_perf failed: {e}");
            return;
        }
        self.lib_patch_names = self.patch_library.list_perfs();
        self.set_toast(format!("SAVED PERF: {name}"), PERF_GOLD);
    }

    /// Load a `Perf` by name and dispatch the matching engine
    /// messages so the live state catches up. Restores params, FX
    /// params, mixer levels + mutes, and jumps the active page to
    /// the first non-empty part's engine.
    pub(crate) fn load_perf(&mut self, name: &str) {
        let perf = match self.patch_library.load_perf(name) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("brume library: load_perf failed: {e}");
                return;
            }
        };

        for (idx, slot) in perf.parts.iter().enumerate() {
            let Some(snap) = slot else {
                continue;
            };
            let part = idx as u8;
            for (key, value) in &snap.params {
                let Ok(id) = key.parse::<ParameterId>() else {
                    continue;
                };
                let v = *value as f32;
                self.param_values.insert((part, id), v);
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetParameter { part, id, value: v }
                );
            }
        }

        if let Some(fx) = &perf.fx {
            for (key, value) in &fx.params {
                let Some((slot, param)) = key.split_once(':') else {
                    continue;
                };
                let v = *value as f32;
                if let Some((tab_idx, tab)) = mix::FX_TABS
                    .iter()
                    .enumerate()
                    .find(|(_, t)| t.slot == Some(slot))
                {
                    if let Some(param_idx) = tab.params.iter().position(|p| p.id == param) {
                        if let Some(row) = self.mix.fx_values.get_mut(tab_idx) {
                            if let Some(stored) = row.get_mut(param_idx) {
                                *stored = v;
                            }
                        }
                    }
                }
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetFxParam {
                        slot: slot.to_string(),
                        param: param.to_string(),
                        value: v,
                    }
                );
            }
        }

        // Mixer.
        for p in 0..4u8 {
            let level = perf.mixer.levels[p as usize];
            self.mix.levels[p as usize] = level;
            try_send_or_log!(
                self.ui_to_engine_tx,
                UiToEngine::SetPartLevel { part: p, level }
            );
            let muted = perf.mixer.mutes[p as usize];
            if self.mix.mutes[p as usize] != muted {
                self.mix.mutes[p as usize] = muted;
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetPartMute { part: p, muted }
                );
            }
            let dly = perf.mixer.delay_sends[p as usize];
            try_send_or_log!(
                self.ui_to_engine_tx,
                UiToEngine::SetPartDelaySend {
                    part: p,
                    level: dly
                }
            );
            let rvb = perf.mixer.reverb_sends[p as usize];
            try_send_or_log!(
                self.ui_to_engine_tx,
                UiToEngine::SetPartReverbSend {
                    part: p,
                    level: rvb
                }
            );
        }
        self.mix.solo = None;

        for (idx, slot) in perf.parts.iter().enumerate() {
            self.loaded_patch_name[idx] = slot.as_ref().map(|_| perf.name.clone());
        }

        let target_mode =
            perf.parts
                .iter()
                .find_map(|slot| slot.as_ref())
                .map_or(Mode::Fm, |snap| match snap.mode.as_str() {
                    "harmonic" => Mode::Harmonic,
                    "timbral" => Mode::Timbral,
                    "granular" => Mode::Granular,
                    _ => Mode::Fm,
                });
        self.page = Page::Engine(target_mode);
        self.republish_knob_mapping();
        self.watch_active_part();
        self.set_toast(format!("LOADED PERF: {}", perf.name), PERF_GOLD);
    }

    /// Snapshot the active engine page's parameters into a Patch
    /// and write it to disk. Auto-naming: `MODE-NNNN` with the last
    /// 4 digits of the unix timestamp.
    pub(crate) fn save_active_patch(&mut self) {
        let mode_str = self.lib_mode;
        // Resolve the mode to a Mode enum so we can iterate the
        // right ParamSpec list and pull the right part's values.
        let mode = match mode_str {
            "fm" => Mode::Fm,
            "harmonic" => Mode::Harmonic,
            "timbral" => Mode::Timbral,
            "granular" => Mode::Granular,
            _ => return,
        };
        let part = mode.part();
        let mut params: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
        for (_label, specs) in mode.tabs() {
            for spec in *specs {
                if spec.binding.part != part {
                    continue;
                }
                let value = self
                    .param_values
                    .get(&(part, spec.binding.id))
                    .copied()
                    .unwrap_or(spec.default_value);
                let key = format!("{:?}", spec.binding.id);
                params.insert(key, value as f64);
            }
        }
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() % 10_000)
            .unwrap_or(0);
        let name = format!("{}-{:04}", mode_str.to_uppercase(), suffix);
        let patch_name = name.clone();
        let patch = Patch::new(name, mode_str.to_string(), params);
        if let Err(e) = self.patch_library.save(&patch) {
            eprintln!("brume library: save failed: {e}");
            return;
        }
        self.lib_patch_names = self.patch_library.list(mode_str);
        self.set_toast(format!("SAVED: {patch_name}"), mode.color());
    }

    /// Load a patch by name and dispatch SetParameter for each
    /// (param, value) entry. Param keys are ParameterId variant
    /// names (`format!("{:?}", id)`); decode via the matching
    /// `ParameterId::FromStr` impl. Out-of-range values clamp on
    /// the engine side, so this side is intentionally lenient —
    /// unknown keys are skipped silently.
    pub(crate) fn load_patch(&mut self, name: &str) {
        let mode_str = self.lib_mode;
        let mode = match mode_str {
            "fm" => Mode::Fm,
            "harmonic" => Mode::Harmonic,
            "timbral" => Mode::Timbral,
            "granular" => Mode::Granular,
            _ => return,
        };
        let part = mode.part();
        let patch = match self.patch_library.load(mode_str, name) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("brume library: load failed: {e}");
                return;
            }
        };
        for (key, value) in patch.params {
            let Ok(id) = key.parse::<ParameterId>() else {
                continue;
            };
            let v = value as f32;
            self.param_values.insert((part, id), v);
            try_send_or_log!(
                self.ui_to_engine_tx,
                UiToEngine::SetParameter { part, id, value: v }
            );
        }
        self.loaded_patch_name[mode.part() as usize] = Some(patch.name.clone());
        self.page = Page::Engine(mode);
        self.republish_knob_mapping();
        self.watch_active_part();
        self.set_toast(format!("LOADED: {}", patch.name), mode.color());
    }

    /// LIBRARY page body — toolbar with mode tabs + SAVE button,
    /// scrollable patch list below.
    pub(crate) fn library_page(&self) -> Element<'_, Message> {
        let perf_color = PERF_GOLD;
        let modes: [(&'static str, &'static str, iced::Color); 5] = [
            ("fm", "FM", Mode::Fm.color()),
            ("harmonic", "HARMONIC", Mode::Harmonic.color()),
            ("timbral", "TIMBRAL", Mode::Timbral.color()),
            ("granular", "GRANULAR", Mode::Granular.color()),
            ("perf", "PERF", perf_color),
        ];
        let mut toolbar = row![].spacing(6).align_y(Alignment::Center);
        for (key, label, accent) in modes {
            let is_active = self.lib_mode == key;
            let label_color = if is_active { TEXT_BRIGHT } else { TEXT_DIM };
            toolbar = toolbar.push(
                button(text(label).size(11).color(label_color))
                    .padding([4, 12])
                    .on_press(Message::SwitchLibMode(key))
                    .style(move |_t, _s| tab_btn_style(is_active, accent)),
            );
        }
        let save_btn = button(
            container(text("SAVE").size(11).color(LEARN_GREEN))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fixed(96.0))
        .height(Length::Fixed(28.0))
        .padding(0)
        .on_press(Message::LibrarySavePatch)
        .style(|_t, _s| button::Style {
            background: Some(iced::Color::TRANSPARENT.into()),
            text_color: LEARN_GREEN,
            border: iced::Border {
                color: LEARN_GREEN,
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow::default(),
        });
        let toolbar_row = row![toolbar, horizontal_space().width(Length::Fill), save_btn]
            .spacing(8)
            .align_y(Alignment::Center);

        let (accent, label_for_empty) = match self.lib_mode {
            "fm" => (Mode::Fm.color(), "FM"),
            "harmonic" => (Mode::Harmonic.color(), "HARMONIC"),
            "timbral" => (Mode::Timbral.color(), "TIMBRAL"),
            "granular" => (Mode::Granular.color(), "GRANULAR"),
            "perf" => (perf_color, "PERF"),
            _ => (Mode::Fm.color(), "FM"),
        };

        let mut list_col = column![].spacing(6);
        if self.lib_patch_names.is_empty() {
            let empty_text = if self.lib_mode == "perf" {
                "No perfs saved yet — tap SAVE to capture the whole instrument.".to_string()
            } else {
                format!("No patches saved for {label_for_empty}")
            };
            list_col = list_col
                .push(container(text(empty_text).size(11).color(LABEL_MUTED)).padding([16, 4]));
        } else {
            // Perfs only live under the user library; factory shipping
            // is a per-mode patch concern. For mode tabs, ask
            // patch_library to classify each entry so factory rows can
            // surface a badge and the DELETE button can be disabled.
            let is_perf = self.lib_mode == "perf";
            for name in &self.lib_patch_names {
                let factory = if is_perf {
                    false
                } else {
                    matches!(
                        self.patch_library.patch_source(self.lib_mode, name),
                        Some(PatchSource::Factory)
                    )
                };
                list_col = list_col.push(self.library_patch_row(name, accent, factory));
            }
        }

        let body = column![
            container(toolbar_row).padding([8, 12]).width(Length::Fill),
            horizontal_rule(1).style(|_t| iced::widget::rule::Style {
                color: RULE_LINE,
                width: 1,
                radius: 0.0.into(),
                fill_mode: iced::widget::rule::FillMode::Full,
            }),
            // Wrap `list_col` in an inner container with right-side
            // padding so each row's DELETE button doesn't run into
            // the scrollable's scrollbar rail when the list is long
            // enough to overflow. The outer container's [12, 16]
            // pads the scrollable as a whole; the inner padding is
            // strictly to clear the scrollbar gutter inside.
            container(
                scrollable(container(list_col).padding(iced::Padding::ZERO.right(16)))
                    .height(Length::Fill),
            )
            .padding([12, 16])
            .height(Length::Fill),
        ]
        .spacing(0);
        panel("LIBRARY", body.into())
    }

    fn library_patch_row(
        &self,
        name: &str,
        accent: iced::Color,
        factory: bool,
    ) -> Element<'_, Message> {
        let label = text(name.to_string())
            .size(13)
            .color(accent)
            .width(Length::Fill);

        // "F" badge sits between the name and the LOAD button on
        // factory entries — small dim chip in the row's accent
        // colour, just enough to signal "this came with the
        // instrument" without dominating the row.
        let badge: Element<'_, Message> = if factory {
            container(text("F").size(10).color(accent))
                .padding([2, 6])
                .style(move |_t: &Theme| container::Style {
                    background: Some(iced::Color { a: 0.08, ..accent }.into()),
                    border: iced::Border {
                        color: iced::Color { a: 0.4, ..accent },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..Default::default()
                })
                .into()
        } else {
            // Reserve no space when there's no badge — the label's
            // Length::Fill flexes to absorb the extra room.
            iced::widget::Space::with_width(Length::Shrink).into()
        };

        let load_btn = button(text("LOAD").size(11).color(accent))
            .padding([4, 12])
            .on_press(Message::LibraryLoadPatch(name.to_string()))
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: accent,
                border: iced::Border {
                    color: accent,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });

        // Factory entries are read-only: dim DELETE and drop the
        // on_press handler so the button can't fire. The runtime's
        // delete() also refuses, but disabling the click here makes
        // the constraint visible in the UI rather than failing
        // silently after a click.
        let delete_color = if factory {
            iced::Color {
                a: 0.30,
                ..iced::Color::from_rgb(0.67, 0.20, 0.20)
            }
        } else {
            iced::Color::from_rgb(0.67, 0.20, 0.20)
        };
        let mut delete_btn = button(text("DELETE").size(11).color(delete_color))
            .padding([4, 12])
            .style(move |_t, _s| button::Style {
                background: Some(iced::Color::TRANSPARENT.into()),
                text_color: delete_color,
                border: iced::Border {
                    color: delete_color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                shadow: iced::Shadow::default(),
            });
        if !factory {
            delete_btn = delete_btn.on_press(Message::LibraryDeletePatch(name.to_string()));
        }

        container(
            row![label, badge, load_btn, delete_btn]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .padding([6, 4])
        .style(|_t: &Theme| container::Style {
            border: iced::Border {
                color: iced::Color {
                    a: 0.10,
                    ..iced::Color::WHITE
                },
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
    }
}
