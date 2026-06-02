// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Novation Launch Control XL 3 driver. Default factory custom mode.
//!
//! Layout reference (channel 1 CCs, captured empirically from a
//! fresh-out-of-box Mk3 unit):
//!
//! | element                         | CC      |
//! |---------------------------------|---------|
//! | Faders 1..8                     | 5..12   |
//! | Encoder row 1 (top, 1..8)       | 13..20  |
//! | Encoder row 2 (mid, 1..8)       | 21..28  |
//! | Encoder row 3 (bot, 1..8)       | 29..36  |
//! | Pads top "DAW Control" 1..8     | 37..44  |
//! | Pads bot "DAW Mixer" 1..8       | 45..52  |
//!
//! Pads emit non-zero on press, 0 on release. The PAGE / TRACK /
//! Record / Play / Shift / Mode / Solo+Arm / Mute+Select buttons
//! emit nothing on the surface port — they're internal navigation
//! only and never reach this dispatch.
//!
//! Mk3 exposes four ALSA ports per device. We bind only the surface
//! port (`LCXL3 N MIDI In/Out`); the DAW port (`LCXL3 N DAW In/Out`,
//! Mackie/HUI protocol) and the two `To DIN Out` passthroughs are
//! deliberately ignored.
//!
//! Encoder row 1 mirrors the nanoKONTROL2 top-row knob path: midi-io's
//! RT short-circuit routes CC 13..20 directly to the engine through
//! the shared `KnobMapping`, so a turn moves the active sub-tab's
//! parameter on the audio thread without waiting for a UI tick. UI
//! dispatch only sees these CCs when the active page leaves them
//! unbound — at which point the MIX / MOD / SCRIPT fall-through
//! paths take over.
//!
//! Pad bindings:
//!   - Top row pads 1..4 → direct engine select (FM, Harmonic,
//!     Timbral, Granular). Mk3 firmware locks PAGE/TRACK to internal
//!     navigation, so we use the unbound top-pad row instead.
//!   - Bottom row pads 1..6 → direct sub-tab select (index 0..5).
//!     Engines have 5–6 sub-tabs; pads 7..8 stay unbound for engines
//!     with fewer.
//!
//! Top pads 5..8 and faders 5..8 and encoder rows 2 & 3 are
//! intentionally unbound. They remain available to MIDI Learn — a
//! real use, not a no-op.

use brume_midi_io::{ControlSurface, ControlSurfaceApi};

/// Default-custom-mode LCXL3 driver. Stateless.
pub struct LaunchControlXl3;

impl ControlSurface for LaunchControlXl3 {
    fn id(&self) -> &'static str {
        "LCXL3"
    }

    fn matches_port(&self, port_name: &str) -> bool {
        // Linux ALSA presents the device as "LCXL3 N" (abbreviated);
        // CoreMIDI / Windows present "Launch Control XL 3" (full).
        // Accept either, then explicitly exclude the DAW (HUI/Mackie)
        // and DIN passthrough siblings.
        let lower = port_name.to_lowercase();
        let is_device = lower.contains("lcxl3") || lower.contains("launch control xl");
        is_device && !lower.contains("daw") && !lower.contains("din")
    }

    fn rt_knob_slot(&self, cc: u8) -> Option<usize> {
        // Encoder row 1 (top) → KnobMapping slots 0..7. Same audio-
        // priority semantics as nanoKONTROL2's top-row knobs.
        if (13..=20).contains(&cc) {
            Some((cc - 13) as usize)
        } else {
            None
        }
    }

    fn handle_cc(&self, app: &mut dyn ControlSurfaceApi, cc: u8, value: f32) {
        // Faders 1..4 → part levels. Page-agnostic so the user can
        // ride the mixer from any tab. Faders 5..8 (CC 9..12) fall
        // through unbound — only four parts exist.
        if (5..=8).contains(&cc) {
            app.set_part_level(cc - 5, value.clamp(0.0, 1.0));
            return;
        }

        // Encoder row 1 only reaches handle_cc when midi-io's RT
        // path didn't short-circuit (active sub-tab leaves the slot
        // unbound). On MIX/MOD/SCRIPT pages those unbound knobs
        // hijack to FX/LFO/script-param controls — same shape as
        // nanoKONTROL2.
        if (13..=20).contains(&cc) {
            let slot = (cc - 13) as usize;
            if app.is_on_mix_page() {
                app.apply_fx_knob(slot, value);
            } else if app.is_on_mod_page() {
                app.apply_mod_knob(slot, value);
            } else if app.is_on_script_page() {
                app.apply_script_knob(slot, value);
            }
            return;
        }

        // Pads in the factory custom mode are toggle-style: each
        // physical press flips state, sending one event with
        // alternating value=127 / value=0. Fire on every event so
        // each press triggers the action exactly once. A momentary
        // "ignore release edge" filter (as nano uses) would skip
        // every other press here.
        match cc {
            // DAW Control top row pads 1..4 → direct engine select.
            // Pads 5..8 (CC 41..44) remain unbound for MIDI Learn.
            37 => app.select_engine(0), // FM
            38 => app.select_engine(1), // Harmonic
            39 => app.select_engine(2), // Timbral
            40 => app.select_engine(3), // Granular
            // DAW Mixer bottom row pads 1..6 → direct sub-tab select.
            // Pads 7..8 (CC 51..52) remain unbound — engines have at
            // most 6 sub-tabs.
            45..=50 => app.select_sub_tab(cc - 45),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        engine_selects: Vec<u8>,
        sub_tab_selects: Vec<u8>,
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
        fn select_engine(&mut self, i: u8) {
            self.engine_selects.push(i);
        }
        fn select_sub_tab(&mut self, i: u8) {
            self.sub_tab_selects.push(i);
        }
    }

    #[test]
    fn id_is_stable() {
        assert_eq!(LaunchControlXl3.id(), "LCXL3");
    }

    #[test]
    fn matches_port_picks_surface_excludes_daw_and_din() {
        let lc = LaunchControlXl3;
        // Linux ALSA shape.
        assert!(lc.matches_port("LCXL3 1:LCXL3 1 MIDI In 16:0"));
        assert!(lc.matches_port("LCXL3 1 MIDI Out"));
        // CoreMIDI / Windows shape (full device name).
        assert!(lc.matches_port("Launch Control XL 3 MIDI"));
        assert!(lc.matches_port("Launch Control XL"));
        // Sibling ports we explicitly exclude.
        assert!(!lc.matches_port("LCXL3 1:LCXL3 1 DAW In 16:1"));
        assert!(!lc.matches_port("LCXL3 1 To DIN Out"));
        assert!(!lc.matches_port("LCXL3 1 To DIN Out 2"));
        // Other surfaces don't false-match.
        assert!(!lc.matches_port("nanoKONTROL2:nanoKONTROL2 _ CTRL 20:0"));
    }

    #[test]
    fn rt_knob_slot_covers_top_row_only() {
        let lc = LaunchControlXl3;
        assert_eq!(lc.rt_knob_slot(12), None);
        assert_eq!(lc.rt_knob_slot(13), Some(0));
        assert_eq!(lc.rt_knob_slot(20), Some(7));
        assert_eq!(lc.rt_knob_slot(21), None);
        assert_eq!(lc.rt_knob_slot(0), None);
    }

    #[test]
    fn faders_1_to_4_drive_part_levels() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 5, 0.5);
        lc.handle_cc(&mut api, 8, 1.0);
        assert_eq!(api.levels, vec![(0, 0.5), (3, 1.0)]);
    }

    #[test]
    fn faders_5_to_8_drop() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 9, 0.5);
        lc.handle_cc(&mut api, 12, 1.0);
        assert!(api.levels.is_empty());
    }

    #[test]
    fn encoder_row_1_falls_through_to_fx_on_mix() {
        let lc = LaunchControlXl3;
        let mut api = MockApi {
            on_mix: true,
            ..Default::default()
        };
        lc.handle_cc(&mut api, 13, 0.25);
        lc.handle_cc(&mut api, 20, 0.75);
        assert_eq!(api.fx_knobs, vec![(0, 0.25), (7, 0.75)]);
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn encoder_row_1_falls_through_to_mod_on_mod() {
        let lc = LaunchControlXl3;
        let mut api = MockApi {
            on_mod: true,
            ..Default::default()
        };
        lc.handle_cc(&mut api, 15, 0.5);
        assert_eq!(api.mod_knobs, vec![(2, 0.5)]);
        assert!(api.fx_knobs.is_empty());
    }

    #[test]
    fn encoder_row_1_falls_through_to_script_on_script() {
        let lc = LaunchControlXl3;
        let mut api = MockApi {
            on_script: true,
            ..Default::default()
        };
        lc.handle_cc(&mut api, 13, 0.0);
        lc.handle_cc(&mut api, 20, 1.0);
        assert_eq!(api.script_knobs, vec![(0, 0.0), (7, 1.0)]);
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn encoder_row_1_drops_on_engine_pages() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 13, 0.5);
        lc.handle_cc(&mut api, 20, 0.5);
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
        assert!(api.script_knobs.is_empty());
    }

    #[test]
    fn encoder_rows_2_and_3_silently_drop() {
        let lc = LaunchControlXl3;
        let mut api = MockApi {
            on_mix: true,
            ..Default::default()
        };
        // Row 2 (CC 21..28) and row 3 (CC 29..36) — none should
        // route anywhere even on a fall-through page.
        for cc in 21..=36 {
            lc.handle_cc(&mut api, cc, 0.5);
        }
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
    }

    #[test]
    fn pads_top_1_to_4_select_engine() {
        // Toggle-mode pads alternate value 127 / 0; both edges
        // represent the same physical "press" intent and must each
        // fire the action exactly once.
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 37, 1.0); // FM
        lc.handle_cc(&mut api, 37, 0.0); // re-press FM
        lc.handle_cc(&mut api, 38, 1.0); // Harmonic
        lc.handle_cc(&mut api, 39, 1.0); // Timbral
        lc.handle_cc(&mut api, 40, 1.0); // Granular
        assert_eq!(api.engine_selects, vec![0, 0, 1, 2, 3]);
        assert!(api.engine_cycles.is_empty());
    }

    #[test]
    fn pads_top_5_to_8_silently_drop() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        for cc in 41..=44 {
            lc.handle_cc(&mut api, cc, 1.0);
            lc.handle_cc(&mut api, cc, 0.0);
        }
        assert!(api.engine_selects.is_empty());
        assert!(api.sub_tab_selects.is_empty());
    }

    #[test]
    fn pads_bot_1_to_6_select_sub_tab() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 45, 1.0); // sub-tab 0
        lc.handle_cc(&mut api, 45, 0.0); // re-press sub-tab 0
        lc.handle_cc(&mut api, 46, 1.0); // sub-tab 1
        lc.handle_cc(&mut api, 47, 1.0); // sub-tab 2
        lc.handle_cc(&mut api, 48, 1.0); // sub-tab 3
        lc.handle_cc(&mut api, 49, 1.0); // sub-tab 4
        lc.handle_cc(&mut api, 50, 1.0); // sub-tab 5
        assert_eq!(api.sub_tab_selects, vec![0, 0, 1, 2, 3, 4, 5]);
        assert!(api.sub_tab_cycles.is_empty());
    }

    #[test]
    fn pads_bot_7_and_8_silently_drop() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 51, 1.0);
        lc.handle_cc(&mut api, 52, 1.0);
        assert!(api.sub_tab_selects.is_empty());
    }

    #[test]
    fn unmapped_ccs_silently_drop() {
        let lc = LaunchControlXl3;
        let mut api = MockApi::default();
        lc.handle_cc(&mut api, 1, 1.0);
        lc.handle_cc(&mut api, 100, 1.0);
        lc.handle_cc(&mut api, 127, 1.0);
        assert!(api.levels.is_empty());
        assert!(api.fx_knobs.is_empty());
        assert!(api.mod_knobs.is_empty());
        assert!(api.mutes.is_empty());
        assert!(api.solos.is_empty());
        assert!(api.engine_cycles.is_empty());
        assert!(api.sub_tab_cycles.is_empty());
    }
}
