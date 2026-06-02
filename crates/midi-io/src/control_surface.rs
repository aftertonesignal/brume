// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! First-class control surface drivers — the bedrock layer below the
//! profile / Lua / MIDI Learn tiers.
//!
//! This module is intentionally *only* trait definitions and the
//! registry container. Concrete drivers (`NanoKontrol2`,
//! `LaunchControlXl3`) live in `crates/ui-native/src/controllers/`
//! because they need to call into `BrumeApp` action methods through
//! `ControlSurfaceApi`. The split keeps midi-io agnostic to UI shape
//! while still owning the seam every driver hooks into.

/// A first-class control surface driver. One implementation per
/// device family Brume ships support for. Drivers handle CC and note
/// events from a recognised hardware controller and dispatch them to
/// app actions through `ControlSurfaceApi`.
///
/// `Send + Sync` because the registry is shared between the UI thread
/// (which calls `handle_cc` / `handle_note`) and the MIDI input
/// thread (which calls `matches_port` / `rt_knob_slot`).
pub trait ControlSurface: Send + Sync {
    /// Stable identifier. midi-io tags
    /// `EngineToUi::ControllerCc { kind, .. }` and
    /// `EngineToUi::ControllerNote { kind, .. }` with this string;
    /// the UI dispatch matches on it. Stable across versions —
    /// community profiles and scripts may key on it.
    fn id(&self) -> &'static str;

    /// Does this MIDI input port belong to this surface? Called once
    /// per discovered port during connection. Case-insensitive
    /// substring match on the OS port name is the typical
    /// implementation; richer matchers (regex, USB VID:PID once the
    /// platform layer exposes it) are the surface's choice.
    fn matches_port(&self, port_name: &str) -> bool;

    /// Optional RT short-circuit: if this CC is one of the surface's
    /// "audio-priority knob" CCs, return the slot index into
    /// `KnobMapping` it maps to. midi-io uses this to route directly
    /// to the engine without a UI round-trip — the audio path stays
    /// Rust-only.
    ///
    /// Default: no short-circuit.
    fn rt_knob_slot(&self, _cc: u8) -> Option<usize> {
        None
    }

    /// Handle a CC event. Called on the UI thread (iced update
    /// loop). `value` is normalised 0.0..=1.0 from the raw 0..127
    /// byte.
    fn handle_cc(&self, app: &mut dyn ControlSurfaceApi, cc: u8, value: f32);

    /// Handle a Note event. Called on the UI thread. Default: ignore
    /// — most surfaces only need CCs.
    fn handle_note(&self, _app: &mut dyn ControlSurfaceApi, _note: u8, _velocity: f32, _on: bool) {}
}

/// What a control surface driver is allowed to do to the app. Small
/// on purpose — anything bigger is a sign the surface is doing
/// something user-customizable that should live in Lua, not Rust.
///
/// `BrumeApp` (in `ui-native`) implements this trait; driver modules
/// only see the trait, so they don't depend on `BrumeApp` internals.
pub trait ControlSurfaceApi {
    /// Is the user currently on the MIX page? Drivers consult this
    /// for page-aware behavior — e.g. nanoKONTROL2 knobs 1..8 fall
    /// through to the FX panel when MIX is visible.
    fn is_on_mix_page(&self) -> bool;

    /// Is the user currently on the MOD page? Same shape as
    /// `is_on_mix_page`.
    fn is_on_mod_page(&self) -> bool;

    /// Set a part's mixer level (0.0..=1.0).
    fn set_part_level(&mut self, part: u8, level: f32);

    /// Toggle mute on a part.
    fn toggle_mix_mute(&mut self, part: u8);

    /// Toggle solo on a part.
    fn toggle_mix_solo(&mut self, part: u8);

    /// Cycle the engine selection. `delta` is +1 / -1.
    fn cycle_engine(&mut self, delta: i32);

    /// Cycle the sub-tab on the current page. `delta` is +1 / -1.
    fn cycle_sub_tab(&mut self, delta: i32);

    /// Drive an FX knob slot from a 0..=1 ratio. Used by surfaces
    /// that fall through to the MIX page's FX panel.
    fn apply_fx_knob(&mut self, slot: usize, ratio: f32);

    /// Drive a MOD knob slot from a 0..=1 ratio. Used by surfaces
    /// that fall through to the MOD page's LFO panel.
    fn apply_mod_knob(&mut self, slot: usize, ratio: f32);

    /// Is the user currently on the SCRIPT page? Drivers consult
    /// this so knobs route to the loaded Lua script's
    /// `brume.add_param`-declared params instead of the engine
    /// page's `KnobBinding` table.
    fn is_on_script_page(&self) -> bool;

    /// Drive a SCRIPT-page param slot from a 0..=1 ratio. `slot`
    /// is the knob index (0..=7) and the implementation maps it
    /// onto the loaded script's parameter at the matching index.
    /// Out-of-range slots no-op. Used by surfaces that fall
    /// through to the SCRIPT page when a Lua script with
    /// `brume.add_param` declarations is loaded.
    fn apply_script_knob(&mut self, slot: usize, ratio: f32);

    /// Switch directly to the engine at `index` (0=FM, 1=Harmonic,
    /// 2=Timbral, 3=Granular). Out-of-range indices no-op. Surfaces
    /// with enough buttons to dedicate one per engine use this; ones
    /// with only relative nav stick to `cycle_engine`.
    fn select_engine(&mut self, index: u8);

    /// Switch directly to sub-tab `index` of the visible engine.
    /// On the MIX page, switches the active FX tab instead. Out-of-
    /// range indices no-op. Same trade-off as `select_engine`:
    /// surfaces with enough buttons jump directly, others cycle.
    fn select_sub_tab(&mut self, index: u8);
}

/// Registry of recognised control surface drivers. Constructed once
/// at startup with the shipped set; shared via `Arc` between the UI
/// thread and the MIDI input thread.
///
/// Linear scan in `by_id` / `by_port` is fine — the surface count is
/// tiny (currently 1, asymptotically <10) and these run only on
/// per-event dispatch, not in any hot loop.
pub struct ControlSurfaceRegistry {
    surfaces: Vec<Box<dyn ControlSurface>>,
}

impl ControlSurfaceRegistry {
    /// Build a registry from a vector of driver boxes. The
    /// `shipped_registry()` factory in `ui-native::controllers`
    /// constructs the canonical "everything Brume ships support
    /// for" instance.
    pub fn new(surfaces: Vec<Box<dyn ControlSurface>>) -> Self {
        Self { surfaces }
    }

    /// Find a surface by stable id. Used by the UI dispatch arm
    /// when an `EngineToUi::ControllerCc { kind, .. }` arrives.
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&dyn ControlSurface> {
        self.surfaces
            .iter()
            .map(AsRef::as_ref)
            .find(|s| s.id() == id)
    }

    /// Find the surface that owns this MIDI port name. Used by
    /// midi-io during connection setup to tag a port for direct
    /// surface dispatch.
    #[must_use]
    pub fn by_port(&self, port_name: &str) -> Option<&dyn ControlSurface> {
        self.surfaces
            .iter()
            .map(AsRef::as_ref)
            .find(|s| s.matches_port(port_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub;
    impl ControlSurface for Stub {
        fn id(&self) -> &'static str {
            "stub"
        }
        fn matches_port(&self, name: &str) -> bool {
            name.contains("Stub")
        }
        fn handle_cc(&self, _app: &mut dyn ControlSurfaceApi, _cc: u8, _value: f32) {}
    }

    #[test]
    fn registry_by_id_finds_registered_surface() {
        let reg = ControlSurfaceRegistry::new(vec![Box::new(Stub)]);
        assert!(reg.by_id("stub").is_some());
        assert!(reg.by_id("missing").is_none());
    }

    #[test]
    fn registry_by_port_uses_each_surface_matcher() {
        let reg = ControlSurfaceRegistry::new(vec![Box::new(Stub)]);
        assert!(reg.by_port("My Stub Device").is_some());
        assert!(reg.by_port("Other Hardware").is_none());
    }

    #[test]
    fn rt_knob_slot_defaults_to_none() {
        let s = Stub;
        assert_eq!(s.rt_knob_slot(16), None);
    }
}
