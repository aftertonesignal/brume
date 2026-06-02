// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Korg nanoKONTROL2 driver. Factory CC mode.
//!
//! Layout reference (factory CC mode, channel 1):
//!
//! | element                | CC          |
//! |------------------------|-------------|
//! | Sliders 1..4           | 0..3        |
//! | Sliders 5..8           | 4..7        |
//! | Knobs 1..8             | 16..23      |
//! | S buttons 1..8         | 32..39      |
//! | M buttons 1..8         | 48..55      |
//! | R buttons 1..8         | 64..71      |
//! | Track ◀ / Track ▶      | 58 / 59     |
//! | Cycle                  | 46          |
//! | Marker Set / ◀ / ▶     | 60 / 61 / 62|
//! | Rew / FF / Stop / ▶ /● | 43 / 44 / 42 / 41 / 45 |
//!
//! Knobs 1..8 are the audio-priority path: midi-io short-circuits
//! them via `rt_knob_slot` directly to the engine through the shared
//! `KnobMapping`, so a CC turn moves the corresponding parameter on
//! the audio thread without waiting for a UI tick. UI dispatch only
//! sees knob CCs when the active page leaves them unbound (MIX / MOD
//! fall-through paths below).

use brume_midi_io::{ControlSurface, ControlSurfaceApi};

/// Factory-CC-mode nanoKONTROL2 driver. Stateless.
pub struct NanoKontrol2;

impl ControlSurface for NanoKontrol2 {
    fn id(&self) -> &'static str {
        "nanoKONTROL2"
    }

    fn matches_port(&self, port_name: &str) -> bool {
        // ALSA reports something like "nanoKONTROL2:nanoKONTROL2 _ CTRL 20:0";
        // CoreMIDI uses different casing. Substring match handles both.
        port_name.to_lowercase().contains("nanokontrol2")
    }

    fn rt_knob_slot(&self, cc: u8) -> Option<usize> {
        // Top-row knobs 1..8 → KnobMapping slots 0..7. Anything else
        // returns None and falls through to `handle_cc`.
        if (16..=23).contains(&cc) {
            Some((cc - 16) as usize)
        } else {
            None
        }
    }

    fn handle_cc(&self, app: &mut dyn ControlSurfaceApi, cc: u8, value: f32) {
        // Sliders 1..4 → part levels. Page-agnostic so the user can
        // ride mix from any tab. Continuous values, no press/release
        // gating.
        if cc <= 3 {
            app.set_part_level(cc, value.clamp(0.0, 1.0));
            return;
        }

        // Knobs 1..8 only reach handle_cc when midi-io's RT path
        // didn't short-circuit them (i.e. the active sub-tab leaves
        // the slot unbound). On the MIX, MOD, and SCRIPT pages
        // those unbound knobs hijack to FX / LFO / script-param
        // controls instead.
        if (16..=23).contains(&cc) {
            let slot = (cc - 16) as usize;
            if app.is_on_mix_page() {
                app.apply_fx_knob(slot, value);
            } else if app.is_on_mod_page() {
                app.apply_mod_knob(slot, value);
            } else if app.is_on_script_page() {
                app.apply_script_knob(slot, value);
            }
            return;
        }

        // Buttons send 127 on press, 0 on release. Act on press only —
        // anything below ignores the release edge.
        if value <= 0.0 {
            return;
        }

        match cc {
            // S buttons 1..4 → solo on parts 0..3. CC 36..39 (S 5..8)
            // intentionally drop on the floor — only four parts exist.
            32..=35 => app.toggle_mix_solo(cc - 32),
            // M buttons 1..4 → mute on parts 0..3. Same reasoning.
            48..=51 => app.toggle_mix_mute(cc - 48),
            // Track ◀ / Track ▶ → cycle the visible engine.
            58 => app.cycle_engine(-1),
            59 => app.cycle_engine(1),
            // Marker ◀ / Marker ▶ → cycle the active sub-tab. On MIX
            // this cycles the FX tab strip; on engine pages it cycles
            // the engine's sub-tabs. ControlSurfaceApi::cycle_sub_tab
            // owns the page-aware branching.
            61 => app.cycle_sub_tab(-1),
            62 => app.cycle_sub_tab(1),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock app that records every action call so the dispatch table
    /// can be exercised without a live `NativeUi`.
    #[derive(Default)]
    struct MockApi {
        on_mix: bool,
        on_mod: bool,
        on_script: bool,
        levels: Vec<(u8, f32)>,
        mutes: Vec<u8>,
        solos: Vec<u8>,
        engine_cycles: Vec<i32>,
        sub_tab_cycles: Vec<i32>,
        fx_knobs: Vec<(usize, f32)>,
        mod_knobs: Vec<(usize, f32)>,
        script_knobs: Vec<(usize, f32)>,
    }

    impl ControlSurfaceApi for MockApi {
        fn is_on_mix_page(&self) -> bool {
            self.on_mix
        }
        fn is_on_mod_page(&self) -> bool {
            self.on_mod
        }
        fn is_on_script_page(&self) -> bool {
            self.on_script
        }
        fn set_part_level(&mut self, p: u8, l: f32) {
            self.levels.push((p, l));
        }
        fn toggle_mix_mute(&mut self, p: u8) {
            self.mutes.push(p);
        }
        fn toggle_mix_solo(&mut self, p: u8) {
            self.solos.push(p);
        }
        fn cycle_engine(&mut self, d: i32) {
            self.engine_cycles.push(d);
        }
        fn cycle_sub_tab(&mut self, d: i32) {
            self.sub_tab_cycles.push(d);
        }
        fn apply_fx_knob(&mut self, s: usize, r: f32) {
            self.fx_knobs.push((s, r));
        }
        fn apply_mod_knob(&mut self, s: usize, r: f32) {
            self.mod_knobs.push((s, r));
        }
        fn apply_script_knob(&mut self, s: usize, r: f32) {
            self.script_knobs.push((s, r));
        }
        fn select_engine(&mut self, _i: u8) {}
        fn select_sub_tab(&mut self, _i: u8) {}
    }

    #[test]
    fn id_is_stable() {
        assert_eq!(NanoKontrol2.id(), "nanoKONTROL2");
    }

    #[test]
    fn matches_port_is_case_insensitive_substring() {
        let nk = NanoKontrol2;
        assert!(nk.matches_port("nanoKONTROL2:nanoKONTROL2 _ CTRL 20:0"));
        assert!(nk.matches_port("NanoKontrol2 SLIDER/KNOB"));
        assert!(!nk.matches_port("Launch Control XL"));
    }

    #[test]
    fn rt_knob_slot_covers_top_row_only() {
        let nk = NanoKontrol2;
        assert_eq!(nk.rt_knob_slot(15), None);
        assert_eq!(nk.rt_knob_slot(16), Some(0));
        assert_eq!(nk.rt_knob_slot(23), Some(7));
        assert_eq!(nk.rt_knob_slot(24), None);
        assert_eq!(nk.rt_knob_slot(0), None);
    }

    #[test]
    fn sliders_drive_part_levels() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 0, 0.5);
        nk.handle_cc(&mut api, 3, 1.0);
        assert_eq!(api.levels, vec![(0, 0.5), (3, 1.0)]);
    }

    #[test]
    fn knobs_fall_through_to_fx_on_mix_page() {
        let nk = NanoKontrol2;
        let mut api = MockApi {
            on_mix: true,
            ..Default::default()
        };
        nk.handle_cc(&mut api, 16, 0.25);
        nk.handle_cc(&mut api, 23, 0.75);
        assert_eq!(api.fx_knobs, vec![(0, 0.25), (7, 0.75)]);
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn knobs_fall_through_to_mod_on_mod_page() {
        let nk = NanoKontrol2;
        let mut api = MockApi {
            on_mod: true,
            ..Default::default()
        };
        nk.handle_cc(&mut api, 18, 0.5);
        assert_eq!(api.mod_knobs, vec![(2, 0.5)]);
        assert!(api.fx_knobs.is_empty());
    }

    #[test]
    fn knobs_fall_through_to_script_on_script_page() {
        let nk = NanoKontrol2;
        let mut api = MockApi {
            on_script: true,
            ..Default::default()
        };
        nk.handle_cc(&mut api, 16, 0.0); // slot 0 → script param 0
        nk.handle_cc(&mut api, 23, 1.0); // slot 7 → script param 7
        assert_eq!(api.script_knobs, vec![(0, 0.0), (7, 1.0)]);
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn knobs_drop_on_other_pages() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 16, 0.5);
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn s_buttons_press_only() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 32, 1.0);
        nk.handle_cc(&mut api, 32, 0.0); // release
        nk.handle_cc(&mut api, 35, 1.0);
        assert_eq!(api.solos, vec![0, 3]);
    }

    #[test]
    fn s_buttons_5_through_8_drop() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 36, 1.0);
        nk.handle_cc(&mut api, 39, 1.0);
        assert!(api.solos.is_empty());
    }

    #[test]
    fn m_buttons_press_only() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 48, 1.0);
        nk.handle_cc(&mut api, 51, 1.0);
        assert_eq!(api.mutes, vec![0, 3]);
    }

    #[test]
    fn track_buttons_cycle_engine() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 58, 1.0);
        nk.handle_cc(&mut api, 59, 1.0);
        assert_eq!(api.engine_cycles, vec![-1, 1]);
    }

    #[test]
    fn marker_buttons_cycle_sub_tab() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 61, 1.0);
        nk.handle_cc(&mut api, 62, 1.0);
        assert_eq!(api.sub_tab_cycles, vec![-1, 1]);
    }

    #[test]
    fn release_edges_dont_fire_buttons() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        nk.handle_cc(&mut api, 58, 0.0);
        nk.handle_cc(&mut api, 59, 0.0);
        nk.handle_cc(&mut api, 32, 0.0);
        nk.handle_cc(&mut api, 48, 0.0);
        assert!(api.engine_cycles.is_empty());
        assert!(api.solos.is_empty());
        assert!(api.mutes.is_empty());
    }

    #[test]
    fn unmapped_ccs_silently_drop() {
        let nk = NanoKontrol2;
        let mut api = MockApi::default();
        // CC 100 isn't in our table — should be a no-op.
        nk.handle_cc(&mut api, 100, 1.0);
        assert!(api.levels.is_empty());
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
        assert!(api.solos.is_empty());
        assert!(api.mutes.is_empty());
        assert!(api.engine_cycles.is_empty());
        assert!(api.sub_tab_cycles.is_empty());
    }
}
