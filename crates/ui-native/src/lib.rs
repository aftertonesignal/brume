// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Native (iced + wgpu) UI for Brume.
//!
//! brume-main does engine/audio/midi setup and calls `run()` to
//! launch the UI on the main thread. The audio callback runs in its
//! own cpal thread and shares the `Arc<Mutex<BrumeEngine>>` with this
//! crate's UI loop only via the engine→UI crossbeam channel — no
//! direct state access.
//!
//! Per-(engine, sub-tab) parameter tables live in `tabs.rs`. Each tap
//! on a sub-tab updates `active_sub_tab` and republishes the shared
//! `KnobMapping` so a connected control surface's knobs follow what's
//! on screen.

mod assign_depth_bar;
pub mod controllers;
mod keyboard;
mod level_meter;
mod lfo_preview;
mod mix;
mod pages;
pub mod persist;
mod ratio_bar;
mod scope_bar;
mod signal_flow;
mod sys;
mod tabs;
mod widgets;

// Re-export the style + chrome helpers at crate root so pages can
// reach them via `use crate::{panel, tab_btn_style, ...}` without
// learning about the `widgets::styles` path.
#[allow(unused_imports)]
pub(crate) use widgets::styles::phosphor_glow;
pub(crate) use widgets::styles::{
    menu_btn_style, panel, panel_with_action, pick_list_menu_style, pick_list_style, tab_btn_style,
};

use std::collections::HashMap;
use std::sync::Arc;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use brume_app_protocol::{EngineToUi, OutputDeviceEntry, UiToEngine};
use brume_common::{KnobBinding, KnobMapping, ParameterId, Scale, try_send_or_log};
use brume_control_model::{ControlBinding, ControlMatrix, FxCcBinding};
use brume_midi_io::{CcCaptured, ControlSurfaceApi, MidiActivity, MidiEvent};
use brume_patch_store::PatchLibrary;
use brume_scripting::{ScriptEngine, ScriptMidiEvent};
use crossbeam_channel::{Receiver, Sender};

use iced::widget::{Space, button, column, container, horizontal_space, row, stack, text};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use persist::PersistHandle;

use tabs::{FM_TABS, GRANULAR_TABS, HARMONIC_TABS, ParamSpec, TIMBRAL_TABS};

// ── Visual tokens ─────────────────────────────────────────────────

const PANEL_BG: iced::Color = iced::Color {
    r: 0.082,
    g: 0.066,
    b: 0.047,
    a: 1.0,
}; // #15110c
const FX_TAB_TRAY_BG: iced::Color = iced::Color {
    r: 0.063,
    g: 0.051,
    b: 0.036,
    a: 1.0,
}; // ~#100d09
// Lifted, warmer brown for panel title bars — matches brume-web's
// `--ink-2` so the desktop UI shares chrome tonality with the
// website. Sits two steps above PANEL_BG so the title bar reads
// distinct from the panel body — `--night-3` was tonally too close
// and disappeared on the CM5 panel.
const TITLE_BAR_BG: iced::Color = iced::Color {
    r: 0.129,
    g: 0.110,
    b: 0.086,
    a: 1.0,
}; // #211C16 (brume-web --night-3)
const PANEL_BORDER: iced::Color = iced::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.078,
}; // #ffffff14
const RULE_LINE: iced::Color = iced::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.063,
}; // ~#ffffff10
const LABEL_MUTED: iced::Color = iced::Color {
    r: 0.549,
    g: 0.521,
    b: 0.482,
    a: 1.0,
}; // ~#8c857b
const TEXT_DIM: iced::Color = iced::Color {
    r: 0.55,
    g: 0.55,
    b: 0.55,
    a: 1.0,
};
const TEXT_BRIGHT: iced::Color = iced::Color {
    r: 0.85,
    g: 0.85,
    b: 0.85,
    a: 1.0,
};
const BRAND_AMBER: iced::Color = iced::Color {
    r: 0.80,
    g: 0.53,
    b: 0.0,
    a: 1.0,
}; // ~#cc8800
// Brighter, more orange amber for the LEARN flow's banner + DETECTED card.
const LEARN_AMBER: iced::Color = iced::Color {
    r: 1.0,
    g: 0.667,
    b: 0.0,
    a: 1.0,
}; // #ffaa00
const LEARN_GREEN: iced::Color = iced::Color {
    r: 0.0,
    g: 0.80,
    b: 0.40,
    a: 1.0,
};
const LEARN_RED: iced::Color = iced::Color {
    r: 0.85,
    g: 0.25,
    b: 0.25,
    a: 1.0,
};
const PERF_GOLD: iced::Color = iced::Color {
    r: 0.88,
    g: 0.75,
    b: 0.38,
    a: 1.0,
};

// ── Splash colors ─────────────────────────────────────────────────
// Lifted from the pre-iced WebKit splash (#0a0907 bg, BRAND_AMBER for
// the wordmark, then progressively dimmer monospace text for
// CM5 / Music Machine / copyright). The dark warm-black bg masked the
// VC4 framebuffer's uninitialized contents on cold boot — same role
// here, since iced + wgpu start the same way.
const SPLASH_BG: iced::Color = iced::Color {
    r: 0.039,
    g: 0.035,
    b: 0.027,
    a: 1.0,
}; // #0a0907
const SPLASH_SUBTITLE: iced::Color = iced::Color {
    r: 0.549,
    g: 0.522,
    b: 0.482,
    a: 1.0,
}; // #8c857b
const SPLASH_TAGLINE: iced::Color = iced::Color {
    r: 0.486,
    g: 0.459,
    b: 0.408,
    a: 1.0,
}; // #7c7568
const SPLASH_COPYRIGHT: iced::Color = iced::Color {
    r: 0.227,
    g: 0.208,
    b: 0.188,
    a: 1.0,
}; // #3a3530

const KEY_WHITE_BG: iced::Color = iced::Color {
    r: 0.10,
    g: 0.10,
    b: 0.10,
    a: 1.0,
};
const KEY_BLACK_BG: iced::Color = iced::Color {
    r: 0.04,
    g: 0.04,
    b: 0.04,
    a: 1.0,
};

/// Threshold above which an FX tap-param's selector flips from a
/// segmented button row to a `pick_list` popover. 5 keeps TYPE (4)
/// and CLOCK (3) as segments; collapses DELAY/SYNC (11) into a pick.
const SEGMENTED_MAX: usize = 5;

/// Number of CC MAPPING sub-tabs (4 engine parts + FX).
const CC_TAB_COUNT: usize = 5;

/// How long the RESET button stays armed after the first tap before
/// auto-disarming. Two-tap confirm to prevent muscle-memory accidents.
const RESET_DISARM: Duration = Duration::from_secs(3);

/// Names of the 24 transition shapes available on the engine's
/// `ShapeLfo`. Engine accepts these names verbatim via
/// `parse_shape` in engine-runtime/src/engine.rs.
const SHAPE_NAMES: [&str; 24] = [
    "Silk",
    "Caffeinated",
    "RampUp",
    "RampDown",
    "SlowBurn",
    "Freefall",
    "QuickDraw",
    "LongTail",
    "Molasses",
    "Catapult",
    "Unfurl",
    "ChunkWedge",
    "Trampoline",
    "Overshoot",
    "Jitters",
    "DiceRoll",
    "Reluctant",
    "LazyLanding",
    "EagerBeaver",
    "Coasting",
    "Switchback",
    "Foothill",
    "Whiplash",
    "Guillotine",
];

/// Modulation source options shown in the ROUTING table. Strings
/// match `engine-runtime`'s `parse_mod_source`.
const SOURCE_OPTIONS: [&str; 4] = ["Lfo1", "Lfo2", "Seq1", "Seq2"];

/// Modulation destinations on the ROUTING table. Curated list of
/// the parameters worth assigning a modulator to — engine-side
/// these all parse via `ParameterId::FromStr`.
const DEST_OPTIONS: [&str; 13] = [
    "FilterCutoff",
    "FilterResonance",
    "FmIndex",
    "FmFeedback",
    "Op1Level",
    "Op2Level",
    "Op3Level",
    "Op4Level",
    "Timbre",
    "Symmetry",
    "FilterEnvDepth",
    "HarmonicTilt",
    "MasterVolume",
];

/// Range bounds for the LFO RATE slider. Log-mapped so the slider
/// travel feels even from very-slow modulation to audio-rate.
const LFO_RATE_MIN_HZ: f32 = 0.05;
const LFO_RATE_MAX_HZ: f32 = 16.0;

/// Step `current` by `dir` through 0..len with wrap-around. `dir`
/// is typically -1 or +1; any non-zero integer works. Caller
/// guarantees `len > 0`.
fn cycle_index(current: usize, len: usize, dir: i32) -> usize {
    debug_assert!(len > 0, "cycle_index called with empty range");
    let len_i = len as i32;
    let cur_i = current as i32;
    (cur_i + dir).rem_euclid(len_i) as usize
}

/// Quantize a 0..1 ratio onto a 0..len bucket index. `floor(r * n)`
/// clamped at n-1 so the top of the slider lands exactly on the
/// last variant. Used wherever a continuous knob drives a discrete
/// option (LFO shape, FX tap params, clock mode, …). Caller
/// guarantees `len > 0`.
fn quantize_ratio_to_bucket(r: f32, len: usize) -> usize {
    debug_assert!(len > 0, "quantize_ratio_to_bucket called with empty range");
    ((r.clamp(0.0, 1.0) * len as f32) as usize).min(len - 1)
}

/// Convert a 0..1 slider ratio onto an LFO rate in Hz via a log
/// mapping. Inverse of `rate_to_ratio`.
fn ratio_to_rate(ratio: f32) -> f32 {
    let r = ratio.clamp(0.0, 1.0);
    let lo = LFO_RATE_MIN_HZ.ln();
    let hi = LFO_RATE_MAX_HZ.ln();
    (lo + r * (hi - lo)).exp()
}

/// Convert an LFO rate in Hz back to its 0..1 slider position.
fn rate_to_ratio(rate: f32) -> f32 {
    let r = rate.clamp(LFO_RATE_MIN_HZ, LFO_RATE_MAX_HZ);
    let lo = LFO_RATE_MIN_HZ.ln();
    let hi = LFO_RATE_MAX_HZ.ln();
    ((r.ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Top-level destination on the menu bar. Engine modes live to the
/// left; utility pages to the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Engine(Mode),
    Mod,
    Mix,
    Midi,
    Sys,
    Library,
    Script,
}

#[derive(Debug, Clone)]
struct Toast {
    message: String,
    color: iced::Color,
    expires_at: Instant,
}

const TOAST_LIFETIME: Duration = Duration::from_millis(1600);

/// Stable engine-side slot name for the user-loaded Lua FX. The
/// engine sees a single, consistent slot identifier across script
/// swaps, so `UiToEngine::SetFxParam` and `UiToEngine::RemoveFxSlot`
/// don't have to chase the script's display name. The UI's
/// constructed `LuaFxSlot` overrides its `name()` to this constant
/// via `LuaFxSlot::with_slot_name`; the script's own `fx.name`
/// declaration is preserved as the display name shown in the tab.
pub(crate) const LUA_FX_SLOT_NAME: &str = "LuaFx";

/// Index used in `MixState::active_fx_tab` to flag the LUA tab.
/// Equal to `mix::FX_TABS.len()` (one past the last built-in tab),
/// so `mix::FX_TABS.get(active_fx_tab)` returns `None` and the
/// usual rendering path falls through to the LUA-specific renderer.
pub(crate) const LUA_FX_TAB_INDEX: usize = 4;

/// View of a loaded Lua FX slot — the data the MIX page needs to
/// render the LUA tab's body. Static for the lifetime of the loaded
/// slot; replaced wholesale on script swap and cleared on UNLOAD.
#[derive(Debug, Clone)]
pub(crate) struct LuaFxLoaded {
    /// Source stem the user picked (no `.lua`). Used to re-load on
    /// hot-reload and as the SELECT-button highlight key.
    pub(crate) script_name: String,
    /// Display name from the script's `fx.name` field — what the
    /// user sees as the LUA tab heading.
    pub(crate) display_name: String,
    /// Param defs declared in the script's `fx.params` table at
    /// load time. Static for the lifetime of the loaded slot;
    /// re-declared if the script is reloaded.
    pub(crate) params: Vec<brume_fx_chain::FxParamDef>,
    /// Mirror of current values; updated optimistically on user
    /// drag and re-seeded to defaults on script load.
    pub(crate) values: Vec<f32>,
}

/// Per-(part, lfo) UI state for the MOD page. Engine doesn't echo
/// LFO config back, so the UI tracks its own snapshot. `SetLfo`
/// dispatches use the full triple (shape + rate + mode) every time
/// so the engine can swap any field without a partial-update path.
#[derive(Debug, Clone, Copy)]
struct LfoUiState {
    /// Index into `SHAPE_NAMES`.
    shape_idx: usize,
    /// Continuous rate in Hz. Slider is log-mapped over the range
    /// `[LFO_RATE_MIN_HZ, LFO_RATE_MAX_HZ]`.
    rate_hz: f32,
    /// `true` = "loop", `false` = "trig". Defaults to trig per the
    /// engine's ShapeLfo default.
    mode_loop: bool,
}

impl LfoUiState {
    const fn default_for(lfo: u8) -> Self {
        // Two distinct defaults so LFO1 vs LFO2 read different on
        // first paint.
        match lfo {
            0 => Self {
                shape_idx: 0,
                rate_hz: 0.25,
                mode_loop: false,
            }, // Silk @ 0.25 Hz, trig
            _ => Self {
                shape_idx: 6,
                rate_hz: 1.0,
                mode_loop: false,
            }, // QuickDraw @ 1.0 Hz, trig
        }
    }
}

/// One row of the MOD page's ROUTING table. Engine has no echo for
/// assignments, so the UI tracks its own list per part. Each
/// mutation here drives an engine dispatch (Add / Remove /
/// SetAssignmentDepth) so engine + UI stay in lockstep.
#[derive(Debug, Clone)]
struct ModAssignment {
    /// Index into `SOURCE_OPTIONS`.
    source: usize,
    /// Index into `DEST_OPTIONS`.
    dest: usize,
    /// Modulation depth in [-1, 1]. Bipolar so a routing can invert.
    depth: f32,
}

/// Which LFO param a MOD-knob slot drives — used by `apply_mod_knob`
/// to dispatch the right field of the local LFO snapshot. Three
/// discrete targets per LFO across six slots (LFO1 in 0..2, LFO2 in
/// 3..5).
#[derive(Debug, Clone, Copy)]
enum ModKnobTarget {
    Shape,
    Rate,
    Mode,
}

/// MIDI Learn flow state on the CC MAPPING panel. `Idle` is the
/// resting state; `Listening` is rendered as the amber banner card;
/// `Detected` morphs the same card into the channel/CC summary +
/// destination picker. midi-io self-clears the AtomicBool on the
/// first capture, so transition Listening → Detected on the first
/// drained `CcCaptured` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LearnPhase {
    Idle,
    Listening,
    Detected { channel: u8, cc: u8, value: u8 },
}

/// MOD page state — the active part tab, the per-part LFO/SEQ/
/// assignment snapshots the engine has no echo for, the live LFO/SEQ
/// values streamed from `EngineToUi::ModFrame`, and the free-running
/// preview phase the LFO PREVIEW canvas advances on Tick.
///
/// (Field name on `NativeUi` is `modulation` rather than `mod` since
/// `mod` is a reserved keyword.)
struct ModState {
    /// Active part on the MOD page. The MOD page has its own part
    /// sub-tabs (independent of the engine pages) so the user can
    /// inspect any part's modulation without leaving the page.
    active_part: u8,
    /// Per-(part, lfo) UI snapshot of LFO config. Engine has no
    /// echo, so this is best-effort: changes here drive `SetLfo`
    /// dispatch and stick on this side until next mutation.
    /// Indexed `[part][lfo]`.
    lfo_state: [[LfoUiState; 2]; 4],
    /// Per-(part, seq) sequencer step values, indexed
    /// `[part][seq][step]`. Engine has no echo for SEQ either —
    /// changes here drive `SetSeqStep` and stay locally cached.
    seq_state: [[[f32; 8]; 2]; 4],
    /// Per-part assignments list. Index in this Vec is the same
    /// index the engine uses for `RemoveAssignment` /
    /// `SetAssignmentDepth` (engine appends in order so first-add =
    /// index 0, etc.).
    assignments: [Vec<ModAssignment>; 4],
    /// Latest live modulation source values for the watched part —
    /// raw 0..1 per source, drained from `EngineToUi::ModFrame`.
    /// Reset to 0 on engine switch.
    lfo1: f32,
    lfo2: f32,
    seq1: f32,
    seq2: f32,
    /// Free-running phase per LFO on the active MOD page (0..1).
    /// Advanced on Tick so the LFO PREVIEW canvas animates at the
    /// configured rate — tweaking RATE produces immediate visible
    /// motion change instead of a static curve.
    lfo_phase: [f32; 2],
    /// Wall-clock timestamp of the last `Tick` that advanced
    /// `lfo_phase`. Lets the preview run at real time regardless
    /// of how often Tick fires.
    last_phase_tick: Instant,
}

impl ModState {
    fn boot() -> Self {
        // SEQ1 default: zigzag. SEQ2 default: smoother ramp.
        // Identical across all four parts so first paint reads
        // predictably; user sculpts per-part from there.
        let seq1: [f32; 8] = [0.0, 0.4, 0.7, 0.3, 0.5, 0.2, 0.8, 0.1];
        let seq2: [f32; 8] = [0.0, 0.25, 0.5, 0.75, 0.5, 0.25, 0.0, 0.5];
        let mut seq_state = [[[0.0f32; 8]; 2]; 4];
        for part in seq_state.iter_mut() {
            part[0] = seq1;
            part[1] = seq2;
        }
        let mut lfo_state = [[LfoUiState::default_for(0); 2]; 4];
        for part_states in lfo_state.iter_mut() {
            part_states[0] = LfoUiState::default_for(0);
            part_states[1] = LfoUiState::default_for(1);
        }
        Self {
            active_part: 0,
            lfo_state,
            seq_state,
            assignments: [vec![], vec![], vec![], vec![]],
            lfo1: 0.0,
            lfo2: 0.0,
            seq1: 0.0,
            seq2: 0.0,
            lfo_phase: [0.0; 2],
            last_phase_tick: Instant::now(),
        }
    }
}

/// Audio engine telemetry mirrored from `EngineToUi` echoes —
/// voice count, transport BPM, the scope peak/RMS for the
/// `WatchPart`-registered part, and the cached output device list.
/// One Tick drains every echo and writes into this cluster.
struct AudioState {
    /// Latest engine voice count (drained from EngineStatus echoes).
    voice_count: u8,
    /// Latest tempo (drained from TransportState echoes).
    bpm: f32,
    /// Latest fractional beat position (drained from TransportState).
    /// Used by the script dispatcher tick to call `on_tick(beat)` on
    /// the loaded script — Lua scripts that run a coroutine clock
    /// (`clock.run`/`clock.sync`) need this to schedule against the
    /// engine's authoritative beat counter rather than wall clock.
    beat: f64,
    /// Latest scope frame for the active engine's part. Engine only
    /// emits scope frames for the part registered via `WatchPart`,
    /// so when the user switches engines we re-register and reset
    /// these to 0 until the next echo arrives.
    scope_peak: f32,
    scope_rms: f32,
    /// OUT level-meter ballistics, advanced on Tick from `scope_peak`
    /// so the bar moves at the ~60 Hz UI rate rather than the ~40 Hz
    /// frame rate. `meter_level` is the displayed bar (instant attack,
    /// exponential release); `meter_hold` is the peak-hold tick;
    /// `meter_hold_remaining` and `meter_clip_remaining` are dwell
    /// timers in seconds for the hold tick and the clip latch.
    meter_level: f32,
    meter_hold: f32,
    meter_hold_remaining: f32,
    meter_clip_remaining: f32,
    /// Cached audio output device list — populated from
    /// `EngineToUi::OutputDeviceList`. Drained on Tick.
    devices: Vec<OutputDeviceEntry>,
    /// Currently-active audio output id (cpal `Device::name()`).
    active: String,
    /// Current audio latency preset (SYS selector), mirrored from
    /// `EngineToUi::AudioLatency`.
    latency: brume_common::LatencyPreset,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            voice_count: 0,
            bpm: 120.0,
            beat: 0.0,
            scope_peak: 0.0,
            scope_rms: 0.0,
            meter_level: 0.0,
            meter_hold: 0.0,
            meter_hold_remaining: 0.0,
            meter_clip_remaining: 0.0,
            devices: vec![],
            active: String::new(),
            latency: brume_common::LatencyPreset::default(),
        }
    }
}

/// User-authoritative transport settings. Distinct from `AudioState`
/// (which holds the engine's live telemetry, including the external-
/// clock-derived BPM in Sync mode); this struct is what the user
/// directly controls — the internal target tempo and the clock-source
/// selection. Persisted across binary restarts; mutations dispatch
/// `UiToEngine::SetTempo` / `UiToEngine::SetClockMode` so the engine
/// stays in sync with what the UI shows.
///
/// `clock_mode_idx` is 0 = Off, 1 = Master, 2 = Sync. Mapped onto the
/// engine's `SetClockMode` strings (`"off"` / `"master"` / `"external"`)
/// at dispatch time. The display labels are kept in
/// `CLOCK_SOURCE_LABELS` so the segmented control + persisted state
/// agree.
#[derive(Debug, Clone, Copy)]
struct TransportUi {
    /// User-set internal target tempo (the BPM slider's value).
    /// Effective when `clock_mode_idx` is 0 (Off) or 1 (Master); in
    /// Sync mode the engine follows external clock and this value is
    /// ignored — the live rate is `AudioState::bpm`.
    bpm: f32,
    /// 0 = Off, 1 = Master, 2 = Sync. See module-level constants.
    clock_mode_idx: usize,
}

/// Display labels for the CLOCK source segmented control. Keep in
/// sync with `CLOCK_SOURCE_WIRE` below — both are 3-element arrays.
pub(crate) const CLOCK_SOURCE_LABELS: [&str; 3] = ["Off", "Master", "Sync"];

/// Wire-format strings dispatched as `UiToEngine::SetClockMode`. The
/// engine maps `"off"` and `"master"` to `ClockMode::Internal`, and
/// `"external"` to `ClockMode::External`. "Master" is conceptually
/// "internal clock + send out" — the send-side isn't engine state, so
/// it lands at the same Internal mode at the audio thread.
pub(crate) const CLOCK_SOURCE_WIRE: [&str; 3] = ["off", "master", "external"];

impl Default for TransportUi {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            clock_mode_idx: 0,
        }
    }
}

/// MIDI Learn flow state — the LearnPhase state machine, the
/// destination the user picked on the DETECTED card, the RESET
/// button's two-tap timestamp, and the shared signals into midi-io
/// (the AtomicBool that arms capture; the Receiver the capture
/// thread pushes events through).
struct LearnState {
    /// Phase of the MIDI Learn flow. Idle by default; advanced by
    /// `Message::ToggleLearn` (Idle ↔ Listening) and by drained
    /// `CcCaptured` events (Listening → Detected).
    phase: LearnPhase,
    /// Destination selected on the DETECTED card. Stored as a String
    /// — engine destinations are `format!("{:?}", ParameterId)`,
    /// FX destinations are `"Slot:param"`. SAVE BINDING parses on
    /// dispatch.
    selected_dest: Option<String>,
    /// RESET button armed-since timestamp. `None` = idle (first tap
    /// arms); `Some(t)` = armed (second tap fires; expires after
    /// `RESET_DISARM`). Tick disarms on timeout.
    reset_armed_at: Option<Instant>,
    /// MIDI Learn arming flag — flipped by `ToggleLearn`; midi-io
    /// self-clears on the first captured CC.
    active: Arc<AtomicBool>,
    /// Receiver for `CcCaptured` events. Drained on Tick to advance
    /// the LearnPhase from Listening to Detected.
    capture_rx: Receiver<CcCaptured>,
}

impl LearnState {
    fn new(active: Arc<AtomicBool>, capture_rx: Receiver<CcCaptured>) -> Self {
        Self {
            phase: LearnPhase::Idle,
            selected_dest: None,
            reset_armed_at: None,
            active,
            capture_rx,
        }
    }
}

/// MIX page state — per-part levels + mute/solo state, the active FX
/// sub-tab, and the per-FX-param value cache. Held in one cluster so
/// the solo invariant (mutes track solo's "everyone-else-muted"
/// state) stays self-contained on the same value.
struct MixState {
    /// Per-part level (0..1). Boots at 0.8 per part.
    levels: [f32; 4],
    /// Per-part mute state.
    mutes: [bool; 4],
    /// Solo'd part index, or `None` if no part is soloed. Solo is
    /// exclusive — engaging solo on a part mutes every other part;
    /// disengaging un-mutes everyone.
    solo: Option<u8>,
    /// Active FX sub-tab index into `mix::FX_TABS`.
    active_fx_tab: usize,
    /// Per-FX-param engine-unit values, indexed `[tab_idx][param_idx]`.
    /// Hydrated from each spec's `default_value` at boot so sliders
    /// snap to the right position before any user interaction.
    fx_values: Vec<Vec<f32>>,
}

impl MixState {
    fn boot() -> Self {
        Self {
            levels: [0.8; 4],
            mutes: [false; 4],
            solo: None,
            active_fx_tab: 0,
            fx_values: mix::FX_TABS
                .iter()
                .map(|tab| tab.params.iter().map(|p| p.default_value).collect())
                .collect(),
        }
    }
}

/// Inline-keyboard state on the engine pages' CONTROLS panel. Lives
/// in one cluster on `NativeUi` rather than six flat fields so the
/// keyboard's invariants (HOLD-vs-RETRIG, fingers map, octave clamp)
/// stay co-located.
struct KeyboardState {
    /// Current octave. Display label is `"C{octave}"`. Each key on the
    /// 13-key strip plays MIDI note `(octave + 1) * 12 + key_offset`,
    /// so octave=4 → C4 starts at MIDI 60.
    octave: i8,
    /// Per-finger MIDI notes currently sounding. With CHORD off each
    /// entry is a single-note `Vec` (the root); with CHORD on it's a
    /// triad voicing. Touch fingers use real ids; the (single) mouse
    /// cursor uses sentinel `MOUSE_FINGER_ID`. Storing notes (not key
    /// indices) means octave shift mid-press can't strand the wrong
    /// NoteOff on release.
    fingers: HashMap<u64, Vec<u8>>,
    /// MIDI notes currently sustained because HOLD was on when their
    /// finger released (or moved off them via glissando). Released
    /// in one batch when HOLD toggles back off.
    held: Vec<u8>,
    /// Sustain held notes after finger lift (HOLD button on the
    /// transport row above the keyboard).
    hold: bool,
    /// Re-trigger envelope on every press, even of a held note
    /// (RETRIG button). Visual-only for now; engine integration is a
    /// follow-up.
    retrig: bool,
    /// Play a triad on each note (CHORD button). Visual-only for now.
    chord: bool,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            octave: 4,
            fingers: HashMap::new(),
            held: Vec::new(),
            hold: false,
            retrig: false,
            chord: false,
        }
    }
}

/// Synth engine assignment for the four parts. `Mode` doubles as the
/// Engine page id and the `(part, ...)` selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Fm,
    Harmonic,
    Timbral,
    Granular,
}

/// Canonical engine order, used by both `cycle_engine` (for relative
/// nav) and `select_engine` (for direct index → engine lookup).
pub(crate) const ENGINE_ORDER: [Mode; 4] =
    [Mode::Fm, Mode::Harmonic, Mode::Timbral, Mode::Granular];

impl Mode {
    /// Part index this engine occupies.
    pub(crate) fn part(self) -> u8 {
        match self {
            Mode::Fm => 0,
            Mode::Harmonic => 1,
            Mode::Timbral => 2,
            Mode::Granular => 3,
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Fm => "FM",
            Mode::Harmonic => "HARMONIC",
            Mode::Timbral => "TIMBRAL",
            Mode::Granular => "GRANULAR",
        }
    }
    /// Mode accent color, one per engine.
    fn color(self) -> iced::Color {
        match self {
            Mode::Fm => iced::Color::from_rgb(0.0, 0.80, 0.40),
            Mode::Harmonic => iced::Color::from_rgb(0.0, 0.67, 0.80),
            Mode::Timbral => iced::Color::from_rgb(0.80, 0.53, 0.0),
            Mode::Granular => iced::Color::from_rgb(0.67, 0.30, 0.80),
        }
    }
    /// Per-(engine, sub-tab) parameter tables (mirror of `SYNTH_TABS`).
    fn tabs(self) -> &'static [(&'static str, &'static [ParamSpec])] {
        match self {
            Mode::Fm => FM_TABS,
            Mode::Harmonic => HARMONIC_TABS,
            Mode::Timbral => TIMBRAL_TABS,
            Mode::Granular => GRANULAR_TABS,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    SwitchPage(Page),
    SwitchSubTab(Mode, usize),
    SliderChanged(KnobBinding, f32),
    /// Tap on a tap-style enum row (e.g. ALGO=STACK→DOUBLE→...).
    /// Cycles to the next variant in `names`, wrapping at the end.
    TapEnum(KnobBinding, &'static [&'static str]),
    /// Press on a keyboard key. `finger=None` means mouse; `Some(id)`
    /// is a per-touch finger id (multi-touch chords).
    KeyboardPress {
        finger: Option<u64>,
        key: u8,
    },
    /// Cursor or finger moved while a press is active. `key` is the
    /// new key index under the input. The handler computes whether
    /// the finger crossed onto a different key (glissando) and emits
    /// the right NoteOff / NoteOn pair.
    KeyboardMove {
        finger: Option<u64>,
        key: u8,
    },
    KeyboardRelease {
        finger: Option<u64>,
    },
    OctaveDown,
    OctaveUp,
    ToggleHold,
    ToggleRetrig,
    ToggleChord,
    /// PANIC button on the CONTROLS panel header. Resets every
    /// parameter on the given engine's part back to its
    /// `ParamSpec::default_value` and dispatches a `SetParameter` for
    /// each so the engine + UI snap to the same default state.
    Panic(Mode),
    /// Per-part level fader on the MIX page. Drag emits a stream of
    /// these; handler updates `mix_levels[part]` and dispatches
    /// `UiToEngine::SetPartLevel`.
    SetMixLevel {
        part: u8,
        ratio: f32,
    },
    /// M button on a MIX strip — toggles the part's mute state and
    /// dispatches `UiToEngine::SetPartMute`.
    ToggleMixMute {
        part: u8,
    },
    /// S button on a MIX strip — toggles exclusive solo. Turning
    /// solo on mutes every other part; turning it off (re-tap or
    /// solo elsewhere) clears all mutes.
    ToggleMixSolo {
        part: u8,
    },
    /// Switch the active FX sub-tab (SAT / DELAY / CHORUS / REVERB / OUT).
    SwitchFxTab(usize),
    /// FX param slider drag — `tab_idx` and `param_idx` look up the
    /// `mix::FxParamSpec` so the handler can scale `ratio` with the
    /// spec's `min`/`max`/`scale` and dispatch the right engine
    /// message (SetFxParam, or SetTempo for the OUT/BPM special case).
    SetFxRatio {
        tab_idx: usize,
        param_idx: usize,
        ratio: f32,
    },
    /// Direct selection of a tap-style FX param (TYPE / SYNC / CLOCK)
    /// from the segmented button row. `value_idx` is the index into the
    /// param spec's `Tap` variant list. CLOCK on the OUTPUT tab maps
    /// the index onto SetClockMode.
    SetFxEnum {
        tab_idx: usize,
        param_idx: usize,
        value_idx: usize,
    },
    /// User picked a channel for `part` from the MIDI page's channel
    /// pick_list. `channel` is the 0-indexed MIDI channel (0 = CH 1)
    /// or `None` for unassigned. The handler unassigns the chosen
    /// channel from any other part that may have owned it so the
    /// `MidiChannelMap` invariant (each channel routes to at most
    /// one part) is preserved.
    SetChannelForPart {
        part: u8,
        channel: Option<u8>,
    },
    /// Set the user's internal target tempo. Updates `transport_ui.bpm`
    /// AND dispatches `UiToEngine::SetTempo`. Effective when CLOCK
    /// source is Off or Master; ignored by the engine in Sync mode but
    /// the slider still tracks so the user has a target to fall back
    /// on when leaving Sync.
    SetClockBpm(f32),
    /// Set the clock source by index — 0 = Off, 1 = Master, 2 = Sync.
    /// Updates `transport_ui.clock_mode_idx` AND dispatches
    /// `UiToEngine::SetClockMode` with the corresponding wire string
    /// (`"off"` / `"master"` / `"external"`).
    SetClockSourceIdx(usize),
    /// Switch the active CC MAPPING sub-tab. Indices 0..=3 select the
    /// engine parts (FM / HARMONIC / TIMBRAL / GRANULAR); index 4
    /// selects the FX bindings list.
    SwitchCcTab(usize),
    /// Remove an existing binding. `fx=false` removes from
    /// `ControlMatrix::bindings`; `fx=true` removes from
    /// `ControlMatrix::fx_bindings`. `idx` is the raw `Vec` index in
    /// either case (computed at render time so it stays valid).
    RemoveCcBinding {
        fx: bool,
        idx: usize,
    },
    /// LEARN button toggle. Idle → Listening (arms the AtomicBool);
    /// Listening or Detected → Idle (cancels). The actual transition
    /// to Detected is driven by the capture channel drain, not this.
    ToggleLearn,
    /// CANCEL button on the DETECTED card — clears phase and the
    /// AtomicBool without writing a binding.
    CancelLearn,
    /// User picked a destination from the DETECTED card's grid.
    /// Stored on `learn_selected_dest`; SAVE BINDING uses it.
    SelectLearnDest(String),
    /// SAVE BINDING button. Pushes a `ControlBinding` (engine dest)
    /// or `FxCcBinding` (FX dest) into the matrix, triggers
    /// persistence, and returns to Idle.
    SaveLearnBinding,
    /// RESET button on the CC MAPPING toolbar. Two-tap confirm: the
    /// first tap arms the button and starts a 3 s auto-disarm timer;
    /// the second tap calls `ControlMatrix::reset_to_defaults` and
    /// triggers persistence. Tick handler disarms on timeout.
    ResetTap,
    /// Switch the active part on the MOD page (one of 0..=3).
    SwitchModPart(u8),
    /// User picked a SHAPE for `(part, lfo)`. `idx` is into
    /// `SHAPE_NAMES`.
    SetLfoShape {
        part: u8,
        lfo: u8,
        idx: usize,
    },
    /// LFO RATE slider drag (or controller knob update). `ratio` is
    /// 0..1 and gets log-mapped onto [`LFO_RATE_MIN_HZ`,
    /// `LFO_RATE_MAX_HZ`] via `ratio_to_rate`.
    SetLfoRate {
        part: u8,
        lfo: u8,
        ratio: f32,
    },
    /// MODE button — flips the mode between "trig" and "loop" for
    /// `(part, lfo)`.
    ToggleLfoMode {
        part: u8,
        lfo: u8,
    },
    /// SEQ step slider drag. `step` is 0..7; `ratio` is 0..1 and
    /// passes straight through as the engine's step value.
    SetSeqStep {
        part: u8,
        seq: u8,
        step: u8,
        ratio: f32,
    },
    /// "+ ADD ROUTING" tap on the MOD page. Appends a default
    /// (Lfo1 → FilterCutoff @ 0.5) assignment to the active part
    /// and dispatches AddAssignment.
    AddRouting,
    /// × tap on a routing row.
    RemoveRouting {
        part: u8,
        idx: u8,
    },
    /// Tap the SOURCE label on a routing row to advance it through
    /// `SOURCE_OPTIONS`. Engine has no "change source" so we
    /// remove + re-add at the same depth.
    CycleAssignmentSource {
        part: u8,
        idx: u8,
    },
    /// Tap the DEST label to advance through `DEST_OPTIONS`. Same
    /// remove + re-add round-trip as the source cycle.
    CycleAssignmentDest {
        part: u8,
        idx: u8,
    },
    /// Bipolar depth-bar drag. `ratio` 0..1 maps to depth -1..1.
    SetAssignmentDepth {
        part: u8,
        idx: u8,
        ratio: f32,
    },
    /// Tap on a non-active SYS audio device row — switches the
    /// engine's output to the chosen `device_id`. Engine pushes a
    /// fresh `OutputDeviceList` once the swap lands.
    SwitchAudioDevice(String),
    /// Tap on a SYS audio latency preset — rebuilds the audio stream
    /// at the new buffer size. Engine echoes back `AudioLatency`.
    SetLatencyPreset(brume_common::LatencyPreset),
    ScriptLoad(String),
    ScriptUnload,
    /// Slider drag inside the SCRIPT page's PARAMS section. `name`
    /// matches a `brume.add_param`-declared parameter on the loaded
    /// script; the handler pushes the new value into the engine's
    /// `BrumeApi::script_params` table so the script's next read
    /// (`brume.get_param_value(name)`) sees it. Local
    /// `script_params` is updated optimistically for slider-bar
    /// continuity.
    ScriptParamChange {
        name: String,
        value: f32,
    },
    /// User picked a script in the MIX page's LUA tab picker. UI
    /// constructs `LuaFxSlot::from_file(path).with_errors_tx().
    /// with_slot_name(LUA_FX_SLOT_NAME)`, sends it via `fx_slot_tx`,
    /// and refreshes `lua_fx_loaded` from the slot's params/defaults.
    LuaFxSelect(String),
    /// User pressed UNLOAD in the LUA tab. UI sends
    /// `UiToEngine::RemoveFxSlot(LUA_FX_SLOT_NAME)` and clears
    /// `lua_fx_loaded`.
    LuaFxUnload,
    /// User dragged a Lua FX param. UI sends
    /// `UiToEngine::SetFxParam { slot: LUA_FX_SLOT_NAME, param,
    /// value }` and updates the local `lua_fx_loaded.values`
    /// mirror so the bar follows the finger without waiting for an
    /// engine echo (FX params don't echo back today).
    LuaFxParamChange {
        param: String,
        value: f32,
    },
    /// Switch the active LIBRARY mode (fm / harmonic / timbral /
    /// granular) and refresh the patch list.
    SwitchLibMode(&'static str),
    /// SAVE button on LIBRARY — snapshots the current part's
    /// parameters into a Patch and writes it to disk.
    LibrarySavePatch,
    /// LOAD button on a patch row — reads the named patch and
    /// dispatches SetParameter for each entry.
    LibraryLoadPatch(String),
    /// DELETE button on a patch row — removes the named patch from
    /// disk. Triggers a list refresh so the row disappears.
    LibraryDeletePatch(String),
    Tick,
    /// Emitted by `iced::window::open_events` when a window appears.
    /// Used to dispatch `change_mode(Fullscreen)` on the CM5 once we
    /// know the live window Id.
    WindowOpened(iced::window::Id),
}

/// Splash-screen state machine. Lives on `NativeUi` while the boot
/// splash is being drawn; cleared to `None` on dismissal so the
/// normal UI is fully interactive.
///
/// Timing matches the pre-iced WebKit splash:
///
///   * 0–900 ms: bg only, wordmark text hidden (font-atlas warmup
///     in the WebKit version; kept here for cadence parity).
///   * 900–4000 ms: full splash visible.
///   * 4000–5500 ms: linear fade of the bg + text alpha from 1.0
///     to 0.0. The main UI (FM page) is already rendered in the
///     stack underneath, so as the splash goes transparent the
///     instrument fades in naturally.
///   * 5500 ms onward: dismissed, splash field flips to None,
///     normal UI takes over.
///
/// Tuned so the fade overlaps phrase 2 of the welcome chime in
/// `apps/brume-main::startup_chime`: the user sees the splash
/// dissolve into the FM page while the resolving Fmaj9 is still
/// ringing.
struct SplashState {
    started_at: Instant,
}

const SPLASH_WORDMARK_REVEAL_MS: u64 = 900;
const SPLASH_FADE_START_MS: u64 = 4_000;
const SPLASH_FADE_DURATION_MS: u64 = 1_500;

impl SplashState {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    fn wordmark_visible(&self) -> bool {
        self.started_at.elapsed() >= Duration::from_millis(SPLASH_WORDMARK_REVEAL_MS)
    }

    /// Linear opacity for the splash's bg + text. Held at 1.0 until
    /// `SPLASH_FADE_START_MS`, ramps to 0.0 over `SPLASH_FADE_DURATION_MS`,
    /// stays at 0.0 after. Multiplied into every color drawn on the
    /// splash so the whole composite fades out together.
    fn alpha(&self) -> f32 {
        let elapsed = self.started_at.elapsed();
        let fade_start = Duration::from_millis(SPLASH_FADE_START_MS);
        if elapsed < fade_start {
            return 1.0;
        }
        let into_fade = (elapsed - fade_start).as_secs_f32();
        let fade_secs = Duration::from_millis(SPLASH_FADE_DURATION_MS).as_secs_f32();
        (1.0 - into_fade / fade_secs).clamp(0.0, 1.0)
    }

    fn dismissed(&self) -> bool {
        self.started_at.elapsed()
            >= Duration::from_millis(SPLASH_FADE_START_MS + SPLASH_FADE_DURATION_MS)
    }
}

struct NativeUi {
    page: Page,
    /// Boot splash state. `Some` while the splash is being drawn;
    /// flipped to `None` on dismissal so the normal UI takes over.
    /// See `SplashState` for the timing model.
    splash: Option<SplashState>,
    /// Active sub-tab index per engine (indexed by `Mode::part()`).
    /// Persisted across page switches so coming back to an engine
    /// returns to the sub-tab the user left it on.
    active_sub_tab: [usize; 4],
    /// Latest engine-unit value per `(part, ParameterId)`. Hydrated
    /// from `ParamSpec::default_value` on boot and updated by every
    /// `EngineToUi::ParameterChanged` echo. Slider rendering converts
    /// engine value → 0..1 ratio via `binding.unapply`.
    param_values: HashMap<(u8, ParameterId), f32>,
    /// Audio engine telemetry — voice count, BPM, the scope peak +
    /// RMS for the watched part, and the cached output device list.
    audio: AudioState,
    /// User-authoritative transport settings — internal target BPM
    /// and the clock-source selection. Mutations dispatch SetTempo /
    /// SetClockMode to the engine; persisted across restarts so the
    /// user's choice survives a binary swap.
    transport_ui: TransportUi,
    /// Background-debounced persist worker for `transport_ui`. UI
    /// calls `transport_persist.save(snapshot)` from the SetClockBpm
    /// / SetClockSourceIdx handlers. Worker writes
    /// `~/.brume/transport.json` after a 1.5 s quiescence window so
    /// rapid slider drags coalesce into one disk write.
    transport_persist: persist::TransportPersistHandle,
    /// Inline-keyboard state on the engine pages' CONTROLS panel.
    /// Holds the octave, the active fingers, the HOLD/RETRIG/CHORD
    /// toggles, and the held-notes set used by the HOLD release.
    keyboard: KeyboardState,
    /// MIX page state — per-part levels + mute/solo, the active FX
    /// sub-tab, and the per-FX-param value cache.
    mix: MixState,
    /// Shared MIDI activity tracker — same handle midi-io writes into.
    /// Polled on `Tick` so the MIDI page can display live note counts
    /// + the connected device name.
    midi_activity: Arc<parking_lot::Mutex<MidiActivity>>,
    /// Shared control matrix (channel routing + Learn bindings). The
    /// MIDI page mutates `channel_map` via `set_channel`; midi-io
    /// reads through this RwLock when routing incoming notes/CCs.
    control_matrix: Arc<std::sync::RwLock<ControlMatrix>>,
    /// Snapshot of `midi_activity.device_name` at the last poll.
    midi_device_name: String,
    /// Per-part assigned MIDI channel (0-indexed, so 0 = CH 1) or
    /// `None` if no channel routes to this part. Refreshed every
    /// `Tick` by walking `control_matrix.channel_map`.
    midi_part_channel: [Option<u8>; 4],
    /// Per-part live note count drawn from the activity tracker for
    /// the assigned channel. 0 if the part has no assignment.
    midi_part_notes: [u32; 4],
    /// Active CC MAPPING sub-tab. Indices 0..=3 correspond to parts;
    /// 4 selects FX bindings. Persisted across page switches so coming
    /// back to MIDI returns to the same view.
    active_cc_tab: usize,
    /// MIDI Learn flow state — phase, selected destination, RESET
    /// arming timestamp, the shared `active` AtomicBool with midi-io,
    /// and the Receiver for captured CCs.
    learn: LearnState,
    /// MOD page state — active part tab, per-part LFO/SEQ/assignment
    /// snapshots, live mod source values, and the LFO preview phase.
    modulation: ModState,
    loaded_patch_name: [Option<String>; 4],
    /// Latest snapshot of system telemetry (temperature, memory,
    /// uptime, display, touch). Refreshed on Tick at ~1 Hz so the
    /// procfs / sysfs reads don't run 250 times a second.
    sys_telemetry: sys::Telemetry,
    /// Wall-clock timestamp of the last `sys_telemetry` refresh —
    /// `None` until the first poll lands.
    last_sys_poll: Option<Instant>,
    patch_library: Arc<PatchLibrary>,
    toast: Option<Toast>,
    lib_mode: &'static str,
    lib_patch_names: Vec<String>,
    script_engine: Option<Arc<std::sync::Mutex<ScriptEngine>>>,
    /// Sender for newly-constructed Lua FX slots. The audio thread's
    /// `process_block` drains the matching rx and pushes each slot
    /// into `FxChain` (replace-by-name semantics, so a single Lua
    /// FX slot sits at the end of the chain regardless of how many
    /// times the user swaps scripts in the MIX page's LUA tab).
    fx_slot_tx: Sender<Box<dyn brume_fx_chain::FxSlot>>,
    /// Sender side of the `script_fx_errors_rx` channel below.
    /// Cloned into each `LuaFxSlot::with_errors_tx` at construction
    /// time so the slot's audio-thread error path lands in the
    /// dispatcher's drain loop. `None` when scripting is
    /// unavailable. Held on the struct so its lifetime keeps the
    /// channel open even when no slot is currently loaded.
    lua_fx_errors_tx: Option<Sender<String>>,
    /// View of the currently-loaded Lua FX slot, if any. None when
    /// the LUA tab's picker is showing (no script loaded). The
    /// engine slot itself lives in `BrumeEngine::fx_chain` under
    /// the stable name `LuaFx`; this struct is the UI's mirror so
    /// it can render the param sliders without a round-trip.
    lua_fx_loaded: Option<LuaFxLoaded>,
    /// Cached list of `~/brume/scripts/fx/*.lua` stems, refreshed
    /// when the user enters the LUA tab so the picker reflects
    /// hot-added or hot-deleted files without restart.
    lua_fx_list: Vec<String>,
    /// Inbound MIDI events forwarded from `midi-io` for delivery to the
    /// loaded Lua script. Drained inside the dispatcher branch of the
    /// 50ms script tick on `Message::Tick`. `None` when scripting is
    /// unavailable (sandbox init failed) so we never even allocate the
    /// channel pair.
    script_midi_rx: Option<Receiver<MidiEvent>>,
    /// Wall-clock timestamp of the last script-engine drain pass.
    /// Gates the 50ms script-tick cadence inside the 250 Hz UI Tick;
    /// scripts that schedule against `on_tick(beat)` see updates at the
    /// engine's actual tick rate, not the UI's draw rate.
    last_script_tick: Option<Instant>,
    /// Errors raised by Lua FX scripts inside the audio-thread
    /// `process_stereo` path, surfaced via a wait-free `try_send` (see
    /// `LuaFxSlot::with_errors_tx`). Drained on the same 50ms cadence
    /// as the script tick and surfaced to the user as toasts. The
    /// matching `Sender` is cloned into each `LuaFxSlot` at FX-load
    /// time; that wiring lands when LuaFxSlot construction moves into
    /// the UI (next commit). For now the receiver exists so the
    /// dispatcher's drain loop is structurally complete.
    script_fx_errors_rx: Option<Receiver<String>>,
    script_list: Vec<String>,
    loaded_script: Option<String>,
    /// Script-defined parameters declared via Lua's
    /// `brume.add_param(name, label, min, max, default)`. Populated
    /// by `dispatch_script_tick` whenever `drain_params()` reports
    /// changes; cleared on `ScriptUnload`. Each tuple is
    /// `(name, label, min, max, value)` — same shape the engine's
    /// drainer returns, kept as a flat Vec rather than a HashMap
    /// because order of declaration is the natural display order
    /// and scripts rarely declare more than ~8 params.
    script_params: Vec<(String, String, f32, f32, f32)>,
    /// Background persistence worker for `ControlMatrix`. `mark_dirty`
    /// is called whenever the user adds, removes, or resets a binding;
    /// the worker debounces writes to `~/.brume/cc-bindings.json`.
    /// Held to keep the worker thread alive — drop on UI exit triggers
    /// a final flush.
    persist: PersistHandle,
    /// Logical-to-physical scale factor, resolved at startup by
    /// `autofit_scale` (auto-fit the canonical 1024×600 design onto
    /// the connected panel) unless `BRUME_UI_SCALE` overrides it. The
    /// UI is designed against a canonical 1024×600 logical resolution
    /// per `HARDWARE.md`.
    scale: f32,
    /// True when the iced window should be fullscreen-borderless on
    /// CM5 (controlled via `BRUME_FULLSCREEN=1`). When set, the
    /// `WindowOpened` handler dispatches `change_mode(Fullscreen)`.
    fullscreen: bool,
    /// Shared with `midi-io` so a control surface's audio-priority
    /// knobs read fresh bindings whenever the active engine + sub-tab
    /// change.
    knob_mapping: Arc<std::sync::RwLock<KnobMapping>>,
    ui_to_engine_tx: Sender<UiToEngine>,
    /// Separate channel for app-level messages (`SetOutputDevice`,
    /// `RequestOutputDeviceList`). brume-main's audio control thread
    /// owns the cpal stream and consumes from this — keeps the
    /// non-Send `AudioStream` off the iced main thread.
    app_tx: Sender<UiToEngine>,
    engine_ui_rx: Receiver<EngineToUi>,
    /// First-class control surface drivers. Shared with
    /// midi-io so the same registry both detects ports and dispatches
    /// the resulting `EngineToUi::ControllerCc` events.
    surfaces: Arc<brume_midi_io::ControlSurfaceRegistry>,
}

/// Reserved finger id for the mouse cursor in `keyboard_fingers`. The
/// real touch fingers are u64 ids assigned by winit; using
/// `u64::MAX` keeps mouse + multi-touch in a single map without the
/// risk of collision.
const MOUSE_FINGER_ID: u64 = u64::MAX;

impl NativeUi {
    /// Clamped index into the active engine's tab list.
    fn active_sub_tab_for(&self, mode: Mode) -> usize {
        let raw = self.active_sub_tab[mode.part() as usize];
        let len = mode.tabs().len();
        raw.min(len.saturating_sub(1))
    }

    /// Parameters visible on the active sub-tab for `mode`.
    fn current_params(&self, mode: Mode) -> &'static [ParamSpec] {
        let i = self.active_sub_tab_for(mode);
        mode.tabs()[i].1
    }

    /// Cycle the visible engine page by `dir` (-1 prev, +1 next).
    /// Reachable from a control surface driver via
    /// `ControlSurfaceApi::cycle_engine`.
    fn cycle_engine(&mut self, dir: i32) {
        let cur = match self.page {
            Page::Engine(m) => m,
            _ => Mode::Fm,
        };
        let cur_idx = ENGINE_ORDER.iter().position(|&m| m == cur).unwrap_or(0);
        let new_idx = cycle_index(cur_idx, ENGINE_ORDER.len(), dir);
        self.page = Page::Engine(ENGINE_ORDER[new_idx]);
        self.republish_knob_mapping();
        self.watch_active_part();
    }

    /// Switch directly to the engine at `index` (0=FM, 1=Harmonic,
    /// 2=Timbral, 3=Granular). Out-of-range no-ops. Reachable from
    /// a control surface driver via `ControlSurfaceApi::select_engine`.
    fn select_engine(&mut self, index: u8) {
        let Some(&mode) = ENGINE_ORDER.get(index as usize) else {
            return;
        };
        self.page = Page::Engine(mode);
        self.republish_knob_mapping();
        self.watch_active_part();
    }

    /// Cycle the active sub-tab of the visible engine by `dir` (-1
    /// prev, +1 next). On the MIX page, cycles the FX tab strip
    /// instead. No-op on pages with no sub-tabs.
    fn cycle_sub_tab(&mut self, dir: i32) {
        match self.page {
            Page::Engine(mode) => {
                let len = mode.tabs().len();
                if len == 0 {
                    return;
                }
                let p = mode.part() as usize;
                self.active_sub_tab[p] = cycle_index(self.active_sub_tab[p], len, dir);
                self.republish_knob_mapping();
            }
            // MIX page has no engine sub-tabs but does have FX sub-tabs;
            // Marker ◀/▶ should cycle them so the user can move between
            // SAT / DELAY / CHORUS / REVERB / OUT without touching the
            // screen.
            Page::Mix => self.cycle_fx_tab(dir),
            _ => {}
        }
    }

    /// Switch directly to sub-tab `index` of the visible engine. On
    /// the MIX page, switches the active FX tab. Out-of-range no-ops.
    /// Reachable from a control surface driver via
    /// `ControlSurfaceApi::select_sub_tab`.
    fn select_sub_tab(&mut self, index: u8) {
        let idx = index as usize;
        match self.page {
            Page::Engine(mode) => {
                if idx >= mode.tabs().len() {
                    return;
                }
                let p = mode.part() as usize;
                self.active_sub_tab[p] = idx;
                self.republish_knob_mapping();
            }
            Page::Mix => {
                if idx < mix::FX_TABS.len() {
                    self.mix.active_fx_tab = idx;
                }
            }
            _ => {}
        }
    }

    fn cycle_fx_tab(&mut self, dir: i32) {
        let len = mix::FX_TABS.len();
        if len == 0 {
            return;
        }
        self.mix.active_fx_tab = cycle_index(self.mix.active_fx_tab, len, dir);
    }

    /// Tell the engine which part to scope. Engine emits ScopeFrame
    /// (and ModFrame) only for the watched part — saves audio-thread
    /// work + crossbeam traffic. Reset our local scope cache so
    /// stale values for the prior part don't briefly linger.
    fn watch_active_part(&mut self) {
        let part = self.active_part();
        self.audio.scope_peak = 0.0;
        self.audio.scope_rms = 0.0;
        self.audio.meter_level = 0.0;
        self.audio.meter_hold = 0.0;
        self.audio.meter_hold_remaining = 0.0;
        self.audio.meter_clip_remaining = 0.0;
        self.modulation.lfo1 = 0.0;
        self.modulation.lfo2 = 0.0;
        self.modulation.seq1 = 0.0;
        self.modulation.seq2 = 0.0;
        try_send_or_log!(self.ui_to_engine_tx, UiToEngine::WatchPart { part });
    }

    /// Republish the shared KnobMapping with the active sub-tab's
    /// first 8 params so a connected control surface's audio-priority
    /// knobs follow what's on screen. Called on engine switch and
    /// sub-tab switch ("screen-follows").
    ///
    /// On the MIX and MOD pages the mapping is cleared to all-`None`
    /// so knob CCs fall through midi-io's short-circuit and arrive at
    /// the UI as `ControllerCc`. The driver there routes them into
    /// FX params (MIX) or LFO params (MOD) — same screen-follows
    /// shape, but for surfaces that don't share the
    /// `KnobBinding`/`ParameterId` path.
    fn republish_knob_mapping(&self) {
        let new_map: KnobMapping = match self.page {
            Page::Engine(mode) => {
                let params = self.current_params(mode);
                let mut m: KnobMapping = [None; 8];
                for (i, spec) in params.iter().take(8).enumerate() {
                    m[i] = Some(spec.binding);
                }
                m
            }
            Page::Mix | Page::Mod | Page::Script => [None; 8],
            // Other utility pages: leave whatever mapping was last
            // active so the controller stays useful while browsing.
            _ => return,
        };
        if let Ok(mut m) = self.knob_mapping.write() {
            *m = new_map;
        }
    }

    /// Look up the engine value for this binding, falling back to
    /// the spec's default if no echo has arrived yet.
    fn value_for(&self, spec: &ParamSpec) -> f32 {
        self.param_values
            .get(&(spec.binding.part, spec.binding.id))
            .copied()
            .unwrap_or(spec.default_value)
    }
}

/// Look up a `ParameterId`'s natural `(min, max, scale)` from the
/// sub-tab tables. Returns the first match across all engines —
/// shared parameters define the same range and scale on every
/// part's tab.
fn param_natural_range_and_scale(id: ParameterId) -> Option<(f32, f32, Scale)> {
    let modes = [Mode::Fm, Mode::Harmonic, Mode::Timbral, Mode::Granular];
    for mode in modes {
        for (_label, specs) in mode.tabs() {
            for spec in *specs {
                if spec.binding.id == id {
                    return Some((spec.binding.min, spec.binding.max, spec.binding.scale));
                }
            }
        }
    }
    None
}

impl NativeUi {
    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::SwitchPage(p) => {
                self.page = p;
                self.republish_knob_mapping();
                if matches!(p, Page::Engine(_)) {
                    self.watch_active_part();
                }
                // Refresh the audio device list whenever the user
                // lands on SYS — devices can hot-plug while the
                // app is open.
                if matches!(p, Page::Sys) {
                    try_send_or_log!(self.app_tx, UiToEngine::RequestOutputDeviceList);
                }
                if matches!(p, Page::Library) {
                    self.lib_patch_names = self.list_for_mode(self.lib_mode);
                }
                if matches!(p, Page::Script) {
                    self.refresh_scripts();
                }
                if matches!(p, Page::Mix) {
                    self.refresh_lua_fx_list();
                }
            }
            Message::SwitchSubTab(mode, idx) => {
                let len = mode.tabs().len();
                if len > 0 {
                    self.active_sub_tab[mode.part() as usize] = idx.min(len - 1);
                    self.republish_knob_mapping();
                }
            }
            Message::SliderChanged(binding, ratio) => {
                let value = binding.apply(ratio);
                self.param_values.insert((binding.part, binding.id), value);
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetParameter {
                        part: binding.part,
                        id: binding.id,
                        value,
                    }
                );
            }
            Message::TapEnum(binding, names) => {
                let cur = self
                    .param_values
                    .get(&(binding.part, binding.id))
                    .copied()
                    .unwrap_or(binding.min);
                let cur_idx = cur.round().clamp(0.0, names.len() as f32 - 1.0) as usize;
                let next = (cur_idx + 1) % names.len();
                let value = next as f32;
                self.param_values.insert((binding.part, binding.id), value);
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetParameter {
                        part: binding.part,
                        id: binding.id,
                        value,
                    }
                );
            }
            Message::Tick => {
                // Boot splash dismisses on the first Tick after its
                // total duration elapses. The Tick subscription fires
                // at ~250 Hz, so the actual dismissal lands within
                // ~4 ms of SPLASH_TOTAL_MS — imperceptible.
                if let Some(s) = &self.splash {
                    if s.dismissed() {
                        self.splash = None;
                    }
                }
                self.poll_midi_activity();
                // Advance free-running LFO preview phases so the
                // canvas animates. Use real wall-clock dt so the
                // motion stays at the configured Hz independent of
                // Tick cadence drift. Modulo 1 keeps phase bounded.
                let now = Instant::now();
                let dt = (now - self.modulation.last_phase_tick).as_secs_f32();
                self.modulation.last_phase_tick = now;
                if dt > 0.0 && dt < 0.5 {
                    let part = self.modulation.active_part as usize;
                    for lfo in 0..2usize {
                        let rate = self.modulation.lfo_state[part][lfo].rate_hz;
                        self.modulation.lfo_phase[lfo] =
                            (self.modulation.lfo_phase[lfo] + rate * dt).fract();
                    }
                    // Advance the OUT meter ballistics on the same dt so
                    // the bar animates at the UI rate, not the frame rate.
                    self.advance_meter(dt);
                }
                // Refresh procfs / sysfs telemetry at ~1 Hz. Reading
                // these files on every Tick (250 Hz) was wasteful;
                // the values move slowly so once a second is plenty.
                let need_poll = self
                    .last_sys_poll
                    .map_or(true, |t| now.duration_since(t) >= Duration::from_secs(1));
                if need_poll {
                    self.sys_telemetry = sys::Telemetry::poll();
                    self.last_sys_poll = Some(now);
                }
                // Script dispatcher — runs every ~50ms inside the
                // 250 Hz UI tick. Same gate-by-elapsed pattern as
                // sys telemetry above. Holds the engine mutex only
                // for the duration of the dispatch; queued MIDI
                // events stay buffered in the rx until the next
                // tick if the lock is contended (rare — only the
                // load_script / unload paths take it).
                let need_script_tick = self
                    .last_script_tick
                    .map_or(true, |t| now.duration_since(t) >= Duration::from_millis(50));
                if need_script_tick {
                    self.dispatch_script_tick();
                    self.last_script_tick = Some(now);
                }
                if let Some(t) = &self.toast {
                    if now >= t.expires_at {
                        self.toast = None;
                    }
                }
                // Auto-disarm RESET if the user didn't follow up
                // within RESET_DISARM. Cheap check — only runs the
                // Instant arithmetic when the button is armed.
                if let Some(t) = self.learn.reset_armed_at {
                    if t.elapsed() >= RESET_DISARM {
                        self.learn.reset_armed_at = None;
                    }
                }
                // Drain any CcCaptured events from midi-io. The
                // AtomicBool is self-cleared on the producer side
                // after a capture, so the UI only needs to advance
                // the LearnPhase. If the user already cancelled
                // (back to Idle) before midi-io's send arrived,
                // ignore the late event.
                while let Ok(cap) = self.learn.capture_rx.try_recv() {
                    if matches!(self.learn.phase, LearnPhase::Listening) {
                        self.learn.phase = LearnPhase::Detected {
                            channel: cap.channel,
                            cc: cap.cc,
                            value: cap.value,
                        };
                        self.learn.selected_dest = None;
                        // If the captured channel maps to one of our
                        // four parts, jump the CC tab there so the
                        // user's destination grid matches the part
                        // they actually played.
                        if cap.channel <= 3 {
                            self.active_cc_tab = cap.channel as usize;
                        }
                    }
                }
                while let Ok(msg) = self.engine_ui_rx.try_recv() {
                    match msg {
                        EngineToUi::ParameterChanged { part, id, value } => {
                            self.param_values.insert((part, id), value);
                        }
                        EngineToUi::ParameterSnapshot { params } => {
                            // Merge into param_values rather than replace —
                            // the engine only records params it has
                            // accepted a SetParameter for, so the UI's
                            // startup hydration of per-tab defaults stays
                            // intact for params nothing has touched yet.
                            // For everything else, the engine's recorded
                            // value is authoritative; this overwrites any
                            // stale UI value left over from a dropped
                            // SetParameter or dropped ParameterChanged.
                            for entry in params {
                                self.param_values
                                    .insert((entry.part, entry.id), entry.value);
                            }
                        }
                        EngineToUi::OutputDeviceList { current, devices } => {
                            self.audio.active = current;
                            self.audio.devices = devices;
                        }
                        EngineToUi::AudioLatency { preset } => {
                            self.audio.latency = preset;
                        }
                        EngineToUi::EngineStatus { voice_count, .. } => {
                            self.audio.voice_count = voice_count;
                        }
                        EngineToUi::TransportState { bpm, beat, .. } => {
                            self.audio.bpm = bpm;
                            self.audio.beat = beat;
                        }
                        EngineToUi::ScopeFrame { part, peak, rms } => {
                            // Engine only sends frames for the part
                            // we asked for via WatchPart, but guard
                            // anyway in case a stale frame slips
                            // through across an engine switch.
                            if part == self.active_part() {
                                self.audio.scope_peak = peak;
                                self.audio.scope_rms = rms;
                                // Arm the clip latch on any window that
                                // reached or crossed 0 dBFS; the Tick
                                // ballistics hold it for the dwell time.
                                if peak >= 1.0 {
                                    self.audio.meter_clip_remaining = METER_CLIP_HOLD_SECS;
                                }
                            }
                        }
                        EngineToUi::ModFrame {
                            part,
                            lfo1,
                            lfo2,
                            seq1,
                            seq2,
                            ..
                        } => {
                            if part == self.active_part() {
                                self.modulation.lfo1 = lfo1;
                                self.modulation.lfo2 = lfo2;
                                self.modulation.seq1 = seq1;
                                self.modulation.seq2 = seq2;
                            }
                        }
                        EngineToUi::ControllerCc { kind, cc, value } => {
                            // Dispatch to the matching surface driver.
                            // The driver knows the device's CC layout;
                            // ControlSurfaceApi is the only seam back
                            // into NativeUi state.
                            let surfaces = Arc::clone(&self.surfaces);
                            if let Some(surface) = surfaces.by_id(&kind) {
                                surface.handle_cc(self, cc, value);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::KeyboardPress { finger, key } => {
                let fid = finger.unwrap_or(MOUSE_FINGER_ID);
                let root = self.key_to_midi(key);
                let new_notes = self.chord_voicing(root);
                // Already pressing this finger? Stop the prior voicing
                // first so re-press doesn't pile voices up. (Most
                // fingers won't trigger this — winit doesn't normally
                // emit FingerPressed twice without a Lifted between.)
                if let Some(old_notes) = self.keyboard.fingers.remove(&fid) {
                    if old_notes.first() == new_notes.first() {
                        // Same root — leave the prior voicing intact
                        // and skip a redundant retrigger.
                        self.keyboard.fingers.insert(fid, old_notes);
                        return Task::none();
                    }
                    self.release_voicing(&old_notes);
                }
                for &n in &new_notes {
                    self.note_on(n);
                }
                self.keyboard.fingers.insert(fid, new_notes);
            }
            Message::KeyboardMove { finger, key } => {
                let fid = finger.unwrap_or(MOUSE_FINGER_ID);
                let Some(old_notes) = self.keyboard.fingers.get(&fid).cloned() else {
                    return Task::none();
                };
                let root = self.key_to_midi(key);
                let new_notes = self.chord_voicing(root);
                if old_notes.first() == new_notes.first() {
                    return Task::none();
                }
                self.release_voicing(&old_notes);
                for &n in &new_notes {
                    self.note_on(n);
                }
                self.keyboard.fingers.insert(fid, new_notes);
            }
            Message::KeyboardRelease { finger } => {
                let fid = finger.unwrap_or(MOUSE_FINGER_ID);
                if let Some(notes) = self.keyboard.fingers.remove(&fid) {
                    self.release_voicing(&notes);
                }
            }
            Message::OctaveDown => {
                // MIDI valid range is 0..=127; clamp octave so the top
                // C of the strip stays in range.
                self.keyboard.octave = (self.keyboard.octave - 1).clamp(-1, 9);
            }
            Message::OctaveUp => {
                self.keyboard.octave = (self.keyboard.octave + 1).clamp(-1, 9);
            }
            Message::ToggleHold => {
                self.keyboard.hold = !self.keyboard.hold;
                if !self.keyboard.hold {
                    // Drop every note that was sustained while HOLD
                    // was on. Currently-pressed fingers stay live —
                    // they'll release normally when the user lifts.
                    let to_release = std::mem::take(&mut self.keyboard.held);
                    for n in to_release {
                        self.note_off(n);
                    }
                }
            }
            Message::ToggleRetrig => {
                self.keyboard.retrig = !self.keyboard.retrig;
            }
            Message::ToggleChord => {
                self.keyboard.chord = !self.keyboard.chord;
            }
            Message::Panic(mode) => {
                let part = mode.part();
                for (_label, params) in mode.tabs() {
                    for spec in *params {
                        let value = spec.default_value;
                        self.param_values.insert((part, spec.binding.id), value);
                        try_send_or_log!(
                            self.ui_to_engine_tx,
                            UiToEngine::SetParameter {
                                part,
                                id: spec.binding.id,
                                value,
                            }
                        );
                    }
                }
                self.loaded_patch_name[part as usize] = None;
            }
            Message::SetMixLevel { part, ratio } => {
                if (part as usize) < self.mix.levels.len() {
                    let level = ratio.clamp(0.0, 1.0);
                    self.mix.levels[part as usize] = level;
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetPartLevel { part, level }
                    );
                }
            }
            Message::ToggleMixMute { part } => {
                self.toggle_mix_mute(part);
            }
            Message::SwitchFxTab(idx) => {
                // Accept indices into FX_TABS plus the appended LUA
                // tab at LUA_FX_TAB_INDEX. The guard rejects bogus
                // indices (anything past the LUA slot would be a
                // logic error in the strip renderer).
                if idx < mix::FX_TABS.len() || idx == LUA_FX_TAB_INDEX {
                    self.mix.active_fx_tab = idx;
                }
            }
            Message::SetFxRatio {
                tab_idx,
                param_idx,
                ratio,
            } => {
                let Some(tab) = mix::FX_TABS.get(tab_idx).copied() else {
                    return Task::none();
                };
                let Some(spec) = tab.params.get(param_idx) else {
                    return Task::none();
                };
                let mix::FxParamKind::Slider { min, max, scale } = spec.kind else {
                    // Tap params come through TapFxEnum; ignore drag.
                    return Task::none();
                };
                let value = apply_scale(scale, ratio, min, max);
                if let Some(row) = self.mix.fx_values.get_mut(tab_idx) {
                    if let Some(slot_value) = row.get_mut(param_idx) {
                        *slot_value = value;
                    }
                }
                if let Some(slot) = tab.slot {
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetFxParam {
                            slot: slot.to_string(),
                            param: spec.id.to_string(),
                            value,
                        }
                    );
                }
                // All current FX_TABS entries have a slot; the OUT
                // tab (BPM/CLOCK/LIMITER) was retired in #10 phase B.
                // BPM dispatch lives on Message::SetClockBpm now.
            }
            Message::SetChannelForPart { part, channel } => {
                self.set_channel_for_part(part, channel);
                self.persist.mark_dirty();
            }
            Message::SetClockBpm(value) => {
                let clamped = value.clamp(20.0, 300.0);
                self.transport_ui.bpm = clamped;
                try_send_or_log!(self.ui_to_engine_tx, UiToEngine::SetTempo(clamped));
                self.transport_persist.save(persist::TransportSnapshot {
                    bpm: self.transport_ui.bpm,
                    clock_mode_idx: self.transport_ui.clock_mode_idx,
                });
            }
            Message::SetClockSourceIdx(idx) => {
                if idx >= CLOCK_SOURCE_WIRE.len() {
                    return Task::none();
                }
                self.transport_ui.clock_mode_idx = idx;
                try_send_or_log!(
                    self.ui_to_engine_tx,
                    UiToEngine::SetClockMode(CLOCK_SOURCE_WIRE[idx].to_string())
                );
                self.transport_persist.save(persist::TransportSnapshot {
                    bpm: self.transport_ui.bpm,
                    clock_mode_idx: self.transport_ui.clock_mode_idx,
                });
            }
            Message::SwitchCcTab(idx) => {
                if idx < CC_TAB_COUNT {
                    self.active_cc_tab = idx;
                }
            }
            Message::RemoveCcBinding { fx, idx } => {
                if let Ok(mut matrix) = self.control_matrix.write() {
                    if fx {
                        if idx < matrix.fx_bindings.len() {
                            matrix.fx_bindings.swap_remove(idx);
                        }
                    } else if idx < matrix.bindings.len() {
                        matrix.bindings.swap_remove(idx);
                    }
                }
                self.persist.mark_dirty();
            }
            Message::ToggleLearn => match self.learn.phase {
                LearnPhase::Idle => {
                    self.learn.active.store(true, Ordering::Release);
                    self.learn.phase = LearnPhase::Listening;
                    self.learn.selected_dest = None;
                }
                LearnPhase::Listening | LearnPhase::Detected { .. } => {
                    self.learn.active.store(false, Ordering::Release);
                    self.learn.phase = LearnPhase::Idle;
                    self.learn.selected_dest = None;
                }
            },
            Message::CancelLearn => {
                self.learn.active.store(false, Ordering::Release);
                self.learn.phase = LearnPhase::Idle;
                self.learn.selected_dest = None;
            }
            Message::SelectLearnDest(dest) => {
                self.learn.selected_dest = Some(dest);
            }
            Message::SwitchModPart(part) => {
                if part < 4 {
                    self.modulation.active_part = part;
                }
            }
            Message::SetLfoShape { part, lfo, idx } => {
                if (part as usize) < 4 && (lfo as usize) < 2 && idx < SHAPE_NAMES.len() {
                    self.modulation.lfo_state[part as usize][lfo as usize].shape_idx = idx;
                    self.dispatch_set_lfo(part, lfo);
                }
            }
            Message::SetLfoRate { part, lfo, ratio } => {
                if (part as usize) < 4 && (lfo as usize) < 2 {
                    self.modulation.lfo_state[part as usize][lfo as usize].rate_hz =
                        ratio_to_rate(ratio);
                    self.dispatch_set_lfo(part, lfo);
                }
            }
            Message::ToggleLfoMode { part, lfo } => {
                if (part as usize) < 4 && (lfo as usize) < 2 {
                    let s = &mut self.modulation.lfo_state[part as usize][lfo as usize];
                    s.mode_loop = !s.mode_loop;
                    self.dispatch_set_lfo(part, lfo);
                }
            }
            Message::AddRouting => {
                let part = self.modulation.active_part;
                if (part as usize) < 4 {
                    // Default new routing: Lfo1 → FilterCutoff @ 0.5.
                    // User edits from there.
                    let new_a = ModAssignment {
                        source: 0,
                        dest: 0,
                        depth: 0.5,
                    };
                    self.modulation.assignments[part as usize].push(new_a.clone());
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::AddAssignment {
                            part,
                            source: SOURCE_OPTIONS[new_a.source].to_string(),
                            dest: DEST_OPTIONS[new_a.dest].to_string(),
                            depth: new_a.depth,
                        }
                    );
                }
            }
            Message::RemoveRouting { part, idx } => {
                let p = part as usize;
                let i = idx as usize;
                if p < 4 && i < self.modulation.assignments[p].len() {
                    self.modulation.assignments[p].remove(i);
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::RemoveAssignment { part, index: idx }
                    );
                }
            }
            Message::CycleAssignmentSource { part, idx } => {
                let p = part as usize;
                let i = idx as usize;
                if p < 4 && i < self.modulation.assignments[p].len() {
                    self.modulation.assignments[p][i].source =
                        (self.modulation.assignments[p][i].source + 1) % SOURCE_OPTIONS.len();
                    self.respin_assignment(part, idx);
                }
            }
            Message::CycleAssignmentDest { part, idx } => {
                let p = part as usize;
                let i = idx as usize;
                if p < 4 && i < self.modulation.assignments[p].len() {
                    self.modulation.assignments[p][i].dest =
                        (self.modulation.assignments[p][i].dest + 1) % DEST_OPTIONS.len();
                    self.respin_assignment(part, idx);
                }
            }
            Message::SwitchAudioDevice(device_id) => {
                // App channel — the audio control thread owns the
                // cpal stream and reopens it on this message.
                try_send_or_log!(self.app_tx, UiToEngine::SetOutputDevice { device_id });
            }
            Message::SetLatencyPreset(preset) => {
                // Optimistic local update for instant feedback; the audio
                // control thread echoes back AudioLatency to confirm.
                self.audio.latency = preset;
                try_send_or_log!(self.app_tx, UiToEngine::SetAudioLatency { preset });
            }
            Message::ScriptLoad(name) => {
                let outcome = if let Some(engine) = &self.script_engine {
                    engine.lock().ok().map(|mut e| {
                        if e.loaded_script().is_some() {
                            e.unload();
                        }
                        e.load_script(&name)
                    })
                } else {
                    None
                };
                match outcome {
                    Some(Ok(())) => {
                        self.loaded_script = Some(name.clone());
                        // Reset the local PARAMS view — the engine
                        // truncated its `script_params` inside
                        // `unload`, and a script that declares no
                        // params at all never triggers a
                        // `drain_params()` return so the local
                        // view would otherwise stay populated with
                        // the prior script's declarations.
                        self.script_params.clear();
                        self.set_toast(format!("LOADED: {name}.lua"), LEARN_GREEN);
                    }
                    Some(Err(err)) => {
                        eprintln!("brume script: {err}");
                        self.set_toast(format!("FAILED: {name}.lua"), LEARN_RED);
                    }
                    None => {}
                }
            }
            Message::ScriptUnload => {
                if let Some(engine) = &self.script_engine {
                    if let Ok(mut e) = engine.lock() {
                        e.unload();
                    }
                }
                self.loaded_script = None;
                // Wipe the local PARAMS view — the engine's
                // `script_params` table got truncated by `unload`,
                // and there's no way for the dispatcher to learn
                // that "nothing was added since last drain" means
                // "everything was removed." Clearing here is the
                // honest move.
                self.script_params.clear();
                self.set_toast("UNLOADED", LABEL_MUTED);
            }
            Message::ScriptParamChange { name, value } => {
                if let Some(engine) = &self.script_engine {
                    if let Ok(e) = engine.lock() {
                        e.set_script_param(&name, value);
                    }
                }
                // Optimistic local update so the slider bar moves
                // immediately under the user's finger instead of
                // waiting up to 50 ms for the next dispatcher tick
                // to drain the engine's view back. The dispatcher
                // would overwrite this anyway with the same value,
                // so the optimism is safe.
                if let Some(p) = self.script_params.iter_mut().find(|p| p.0 == name) {
                    p.4 = value;
                }
            }
            Message::SwitchLibMode(mode) => {
                self.lib_mode = mode;
                self.lib_patch_names = self.list_for_mode(mode);
            }
            Message::LibrarySavePatch => {
                if self.lib_mode == "perf" {
                    self.save_perf();
                } else {
                    self.save_active_patch();
                }
            }
            Message::LibraryLoadPatch(name) => {
                if self.lib_mode == "perf" {
                    self.load_perf(&name);
                } else {
                    self.load_patch(&name);
                }
            }
            Message::LibraryDeletePatch(name) => {
                if self.lib_mode == "perf" {
                    let _ = self.patch_library.delete_perf(&name);
                } else {
                    let _ = self.patch_library.delete(self.lib_mode, &name);
                }
                self.lib_patch_names = self.list_for_mode(self.lib_mode);
            }
            Message::SetAssignmentDepth { part, idx, ratio } => {
                let p = part as usize;
                let i = idx as usize;
                if p < 4 && i < self.modulation.assignments[p].len() {
                    let depth = (ratio.clamp(0.0, 1.0) * 2.0 - 1.0).clamp(-1.0, 1.0);
                    self.modulation.assignments[p][i].depth = depth;
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetAssignmentDepth {
                            part,
                            index: idx,
                            depth,
                        }
                    );
                }
            }
            Message::SetSeqStep {
                part,
                seq,
                step,
                ratio,
            } => {
                if (part as usize) < 4 && (seq as usize) < 2 && (step as usize) < 8 {
                    let value = ratio.clamp(0.0, 1.0);
                    self.modulation.seq_state[part as usize][seq as usize][step as usize] = value;
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetSeqStep {
                            part,
                            seq,
                            step,
                            value,
                        }
                    );
                }
            }
            Message::ResetTap => {
                // Two-tap pattern: first tap arms, second tap fires.
                // The Tick handler clears `reset_armed_at` after
                // `RESET_DISARM` so a stale arm can't fire by accident.
                match self.learn.reset_armed_at {
                    Some(_) => {
                        if let Ok(mut matrix) = self.control_matrix.write() {
                            matrix.reset_to_defaults();
                        }
                        self.learn.reset_armed_at = None;
                        self.persist.mark_dirty();
                    }
                    None => {
                        self.learn.reset_armed_at = Some(Instant::now());
                    }
                }
            }
            Message::SaveLearnBinding => {
                let LearnPhase::Detected { cc, .. } = self.learn.phase else {
                    return Task::none();
                };
                let Some(dest) = self.learn.selected_dest.take() else {
                    return Task::none();
                };
                if let Ok(mut matrix) = self.control_matrix.write() {
                    if let Some((slot, param)) = dest.split_once(':') {
                        // FX binding — Slot:param string. Range hard-coded
                        // to 0..1 (FX params normalize their own ranges).
                        matrix.fx_bindings.push(FxCcBinding {
                            cc_number: cc,
                            destination: format!("{slot}:{param}"),
                            range_min: 0.0,
                            range_max: 1.0,
                        });
                    } else if let Ok(id) = dest.parse::<ParameterId>() {
                        // Engine binding. The active CC tab maps to a
                        // part, stuffed into channel_filter since each
                        // part has a unique MIDI channel anyway.
                        let part = self.active_cc_tab as u8;
                        let (range_min, range_max, scale) =
                            param_natural_range_and_scale(id).unwrap_or((0.0, 1.0, Scale::Linear));
                        matrix.bindings.push(ControlBinding {
                            cc_number: cc,
                            channel_filter: Some(part),
                            destination: id,
                            range_min,
                            range_max,
                            scale,
                        });
                    }
                }
                self.learn.active.store(false, Ordering::Release);
                self.learn.phase = LearnPhase::Idle;
                self.persist.mark_dirty();
            }
            Message::SetFxEnum {
                tab_idx,
                param_idx,
                value_idx,
            } => {
                let Some(tab) = mix::FX_TABS.get(tab_idx).copied() else {
                    return Task::none();
                };
                let Some(spec) = tab.params.get(param_idx) else {
                    return Task::none();
                };
                let mix::FxParamKind::Tap(names) = spec.kind else {
                    return Task::none();
                };
                if value_idx >= names.len() {
                    return Task::none();
                }
                let value = value_idx as f32;
                if let Some(row) = self.mix.fx_values.get_mut(tab_idx) {
                    if let Some(slot_value) = row.get_mut(param_idx) {
                        *slot_value = value;
                    }
                }
                if let Some(slot) = tab.slot {
                    try_send_or_log!(
                        self.ui_to_engine_tx,
                        UiToEngine::SetFxParam {
                            slot: slot.to_string(),
                            param: spec.id.to_string(),
                            value,
                        }
                    );
                }
                // All current FX_TABS entries have a slot. CLOCK source
                // dispatch lives on Message::SetClockSourceIdx now.
            }
            Message::ToggleMixSolo { part } => {
                self.toggle_mix_solo(part);
            }
            Message::WindowOpened(id) => {
                if self.fullscreen {
                    return iced::window::change_mode(id, iced::window::Mode::Fullscreen);
                }
            }
            Message::LuaFxSelect(name) => {
                self.load_lua_fx(&name);
            }
            Message::LuaFxUnload => {
                self.unload_lua_fx();
            }
            Message::LuaFxParamChange { param, value } => {
                self.set_lua_fx_param(&param, value);
            }
        }
        Task::none()
    }

    /// Active engine's part — the keyboard plays into whichever engine
    /// the user is currently looking at (or FM by default for non-
    /// engine pages, so the keyboard still does something useful from
    /// MIDI / SYS / etc.).
    fn active_part(&self) -> u8 {
        match self.page {
            Page::Engine(m) => m.part(),
            _ => 0,
        }
    }

    fn key_to_midi(&self, key: u8) -> u8 {
        let n = (self.keyboard.octave as i32 + 1) * 12 + key as i32;
        n.clamp(0, 127) as u8
    }

    /// Notes a single key-press should produce. Just the root unless
    /// CHORD is on, then a major triad (root, +4, +7 semitones).
    /// CHORD state is captured at press time and stored in
    /// `keyboard_fingers` so toggling it mid-press doesn't leave
    /// stranded NoteOffs.
    fn chord_voicing(&self, root: u8) -> Vec<u8> {
        if self.keyboard.chord {
            let third = root.saturating_add(4).min(127);
            let fifth = root.saturating_add(7).min(127);
            vec![root, third, fifth]
        } else {
            vec![root]
        }
    }

    fn note_on(&self, note: u8) {
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::NoteOn {
                part: self.active_part(),
                note,
                velocity: 0.85,
            }
        );
    }

    fn note_off(&self, note: u8) {
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::NoteOff {
                part: self.active_part(),
                note,
            }
        );
    }

    /// Release a finger's voicing. With HOLD on, defer every NoteOff
    /// to `keyboard_held` (released when HOLD toggles back off).
    /// Without HOLD, send NoteOff immediately.
    fn release_voicing(&mut self, notes: &[u8]) {
        if self.keyboard.hold {
            self.keyboard.held.extend(notes);
        } else {
            for &n in notes {
                self.note_off(n);
            }
        }
    }

    /// Scale logical UI coordinates onto the physical panel. Designed
    /// against 1024×600 logical; on the MAGEX 1920×1200 panel this is
    /// 1.875×. Override via `BRUME_UI_SCALE`.
    fn scale_factor(&self) -> f64 {
        self.scale as f64
    }

    fn view(&self) -> Element<'_, Message> {
        let core: Element<'_, Message> =
            column![self.menu_bar(), self.body(), self.status_bar(),].into();
        let with_toast: Element<'_, Message> = match &self.toast {
            Some(t) => stack![core, self.toast_overlay(t)].into(),
            None => core,
        };
        // Splash sits above everything else — including any toast that
        // happened to fire during boot — so the cold-start visual is
        // never interrupted.
        match &self.splash {
            Some(s) => stack![with_toast, self.splash_overlay(s)].into(),
            None => with_toast,
        }
    }

    /// Renders the boot splash. Full-viewport dark warm-black bg
    /// painted from the first frame so the iced + wgpu compositor
    /// has something to draw before the rest of the UI's first paint
    /// resolves; the inner text content is hidden for the first
    /// `SPLASH_WORDMARK_REVEAL_MS` so the wordmark doesn't scan in
    /// half-rasterized on a cold boot.
    fn splash_overlay(&self, splash: &SplashState) -> Element<'_, Message> {
        // Multiply every color's alpha by the splash's current opacity
        // so the bg + all text fade together. While the splash is in
        // its visible window this is a no-op (alpha = 1.0); during
        // the 4–5.5 s fade-out the alpha ramps to 0 and the FM page
        // already in the stack underneath shows through.
        let alpha = splash.alpha();
        let scale = move |c: iced::Color| iced::Color {
            a: c.a * alpha,
            ..c
        };

        let inner: Element<'_, Message> = if splash.wordmark_visible() {
            column![
                text("Brume")
                    .size(96)
                    .color(scale(BRAND_AMBER))
                    .font(iced::Font {
                        family: iced::font::Family::Name("Instrument Serif"),
                        style: iced::font::Style::Italic,
                        ..iced::Font::DEFAULT
                    }),
                container(text("CM5").size(12).color(scale(SPLASH_SUBTITLE))).padding(
                    iced::Padding {
                        top: 18.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                    }
                ),
                container(text("MUSIC MACHINE").size(11).color(scale(SPLASH_TAGLINE))).padding(
                    iced::Padding {
                        top: 14.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                    }
                ),
                container(
                    text("© 2026 BRANDON HUEY")
                        .size(9)
                        .color(scale(SPLASH_COPYRIGHT))
                )
                .padding(iced::Padding {
                    top: 44.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                }),
            ]
            .align_x(Alignment::Center)
            .into()
        } else {
            // Wordmark hidden — render nothing, but the outer container
            // still paints the dark bg over the entire viewport so a
            // cold boot doesn't briefly flash the underlying UI.
            iced::widget::Space::new(Length::Shrink, Length::Shrink).into()
        };

        let bg = scale(SPLASH_BG);
        container(inner)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &Theme| container::Style {
                background: Some(bg.into()),
                ..container::Style::default()
            })
            .into()
    }

    fn toast_overlay(&self, toast: &Toast) -> Element<'_, Message> {
        let color = toast.color;
        let inner = container(text(toast.message.clone()).size(20).color(color))
            .padding(iced::Padding {
                top: 22.0,
                right: 40.0,
                bottom: 22.0,
                left: 40.0,
            })
            .style(move |_t: &Theme| container::Style {
                background: Some(iced::Color::from_rgba(0.024, 0.024, 0.039, 0.92).into()),
                border: iced::Border {
                    color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..Default::default()
            });
        container(inner)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    /// Inline 13-key keyboard for the CONTROLS panel of engine pages.
    /// Lives at the bottom of CONTROLS, not as a global window bar:
    /// non-engine pages don't have a keyboard. Single Canvas widget
    /// covers all keys so multi-touch + glissando are coherent.
    fn keyboard(&self) -> Element<'_, Message> {
        let kb = keyboard::Keyboard {
            white_bg: KEY_WHITE_BG,
            black_bg: KEY_BLACK_BG,
            border: PANEL_BORDER,
            label_color: TEXT_DIM,
        };
        iced::widget::canvas::Canvas::new(kb)
            .width(Length::Fill)
            .height(Length::Fixed(52.0))
            .into()
    }

    /// Transport row above the keyboard: HOLD / RETRIG / CHORD +
    /// octave shift. Each cell is a tappable button; toggle buttons
    /// (HOLD / RETRIG / CHORD) light up in mode color when active.
    fn transport_row(&self) -> Element<'_, Message> {
        let mode_color = match self.page {
            Page::Engine(m) => m.color(),
            _ => TEXT_DIM,
        };
        let toggle_cell =
            move |label: &'static str, active: bool, msg: Message| -> Element<'_, Message> {
                let label_color = if active { mode_color } else { TEXT_DIM };
                let pill_color = mode_color;
                button(container(text(label).size(11).color(label_color)).center_x(Length::Fill))
                    .width(Length::Fill)
                    .padding(8)
                    .on_press(msg)
                    .style(move |_t, _s| menu_btn_style(active, pill_color))
                    .into()
            };
        let action_cell = move |label: String, msg: Message| -> Element<'_, Message> {
            button(container(text(label).size(11).color(TEXT_DIM)).center_x(Length::Fill))
                .width(Length::Fill)
                .padding(8)
                .on_press(msg)
                .style(|_t, _s| menu_btn_style(false, iced::Color::TRANSPARENT))
                .into()
        };
        let octave_label: Element<'_, Message> = container(
            text(format!("C{}", self.keyboard.octave))
                .size(11)
                .color(TEXT_BRIGHT),
        )
        .center_x(Length::Fill)
        .width(Length::Fill)
        .padding(8)
        .into();

        row![
            toggle_cell("HOLD", self.keyboard.hold, Message::ToggleHold),
            toggle_cell("RETRIG", self.keyboard.retrig, Message::ToggleRetrig),
            toggle_cell("CHORD", self.keyboard.chord, Message::ToggleChord),
            action_cell("−".to_string(), Message::OctaveDown),
            octave_label,
            action_cell("+".to_string(), Message::OctaveUp),
        ]
        .spacing(2)
        .into()
    }

    /// Top tab bar — engine modes on the left, utility pages on the
    /// right. Active state is signaled by a soft colored pill backing
    /// behind the label; the label itself never changes font size,
    /// weight, or position, so tapping a tab can't shift the layout.
    fn menu_bar(&self) -> Element<'_, Message> {
        let mode_btn = |m: Mode| -> Element<'_, Message> {
            let active = matches!(self.page, Page::Engine(x) if x == m);
            // Active label goes white so it doesn't blend with the
            // mode-color pill behind it. Inactive stays in mode color
            // at reduced alpha — the four-color palette still reads
            // across the bar.
            let label_color = if active {
                TEXT_BRIGHT
            } else {
                iced::Color {
                    a: 0.6,
                    ..m.color()
                }
            };
            let pill_color = m.color();
            container(
                button(text(m.label()).size(13).color(label_color))
                    .padding(iced::Padding {
                        top: 3.0,
                        right: 6.0,
                        bottom: 3.0,
                        left: 6.0,
                    })
                    .on_press(Message::SwitchPage(Page::Engine(m)))
                    .style(move |_t, _s| menu_btn_style(active, pill_color)),
            )
            .padding(iced::Padding {
                top: 2.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            })
            .into()
        };
        let util_btn = |util: &'static str, page: Page| -> Element<'_, Message> {
            let active = self.page == page;
            let label_color = if active { TEXT_BRIGHT } else { TEXT_DIM };
            let pill_color = iced::Color::from_rgb(0.40, 0.40, 0.40);
            container(
                button(text(util).size(13).color(label_color))
                    .padding(iced::Padding {
                        top: 3.0,
                        right: 6.0,
                        bottom: 3.0,
                        left: 6.0,
                    })
                    .on_press(Message::SwitchPage(page))
                    .style(move |_t, _s| menu_btn_style(active, pill_color)),
            )
            .padding(iced::Padding {
                top: 2.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            })
            .into()
        };

        let wordmark: Element<'_, Message> = row![
            container(text("◆").size(10).color(BRAND_AMBER)).padding(iced::Padding {
                top: 0.0,
                right: 0.0,
                bottom: 6.0,
                left: 0.0
            }),
            text("Brume").size(18).color(BRAND_AMBER).font(iced::Font {
                family: iced::font::Family::Name("Instrument Serif"),
                style: iced::font::Style::Italic,
                ..iced::Font::DEFAULT
            }),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into();

        let modes = row![
            wordmark,
            mode_btn(Mode::Fm),
            mode_btn(Mode::Harmonic),
            mode_btn(Mode::Timbral),
            mode_btn(Mode::Granular),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let utils = row![
            util_btn("MOD", Page::Mod),
            util_btn("MIX", Page::Mix),
            util_btn("MIDI", Page::Midi),
            util_btn("SYS", Page::Sys),
            util_btn("LIBRARY", Page::Library),
            util_btn("SCRIPT", Page::Script),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        container(
            row![modes, horizontal_space(), utils]
                .align_y(Alignment::Center)
                .padding(iced::Padding {
                    top: 6.0,
                    right: 8.0,
                    bottom: 4.0,
                    left: 8.0,
                }),
        )
        .width(Length::Fill)
        .into()
    }

    fn body(&self) -> Element<'_, Message> {
        let inner: Element<'_, Message> = match self.page {
            Page::Engine(mode) => self.engine_page(mode),
            Page::Mod => self.mod_page(),
            Page::Mix => self.mix_page(),
            Page::Midi => self.midi_page(),
            Page::Sys => self.sys_page(),
            Page::Library => self.library_page(),
            Page::Script => self.script_page(),
        };
        container(inner)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8)
            .into()
    }

    /// Bottom status bar: ■ patch | DEVICE | 48kHz | voices | ♪=BPM
    /// (right) version | ENGINE_NAME.
    fn status_bar(&self) -> Element<'_, Message> {
        let mode_label = match self.page {
            Page::Engine(m) => m.label(),
            _ => "—",
        };
        let mode_color = match self.page {
            Page::Engine(m) => m.color(),
            _ => TEXT_DIM,
        };
        let active_part = self.active_part() as usize;
        let patch_label = self
            .loaded_patch_name
            .get(active_part)
            .and_then(|s| s.clone())
            .unwrap_or_else(|| "—".to_string());
        let device_label = audio_device_short(&self.audio.active);
        let version_label = concat!("v", env!("CARGO_PKG_VERSION"));
        let sep = || text("|").size(10).color(LABEL_MUTED);
        container(
            row![
                text("■").size(10).color(mode_color),
                Space::with_width(Length::Fixed(6.0)),
                text(patch_label).size(10).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(12.0)),
                sep(),
                Space::with_width(Length::Fixed(12.0)),
                text(device_label).size(10).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(12.0)),
                sep(),
                Space::with_width(Length::Fixed(12.0)),
                text("48kHz").size(10).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(12.0)),
                sep(),
                Space::with_width(Length::Fixed(12.0)),
                text(format!("{}v", self.audio.voice_count))
                    .size(10)
                    .color(LABEL_MUTED),
                Space::with_width(Length::Fixed(12.0)),
                sep(),
                Space::with_width(Length::Fixed(12.0)),
                text(format!("♪={:.0}", self.audio.bpm))
                    .size(10)
                    .color(LABEL_MUTED),
                horizontal_space().width(Length::Fill),
                text(version_label).size(10).color(LABEL_MUTED),
                Space::with_width(Length::Fixed(12.0)),
                sep(),
                Space::with_width(Length::Fixed(12.0)),
                text(mode_label).size(10).color(mode_color),
            ]
            .padding([6, 12])
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        // 16 ms ≈ 62.5 Hz — matches the system's visual refresh
        // target. The earlier 250 Hz tick burned the GUI thread
        // under heavy MIDI activity (full-resonance FM under a
        // 1/64 arp): every tick re-evaluates the iced view tree,
        // invalidates dirty canvas regions for the SCOPE peak
        // meter, and repaints — at 4 ms intervals that ran the
        // event-loop core at 99 % CPU on the CM5 even with the
        // audio thread sitting at 9 %. 16 ms is still well below
        // the script-tick gate cadence (50 ms) so the periodic
        // gates inside `update()` still fire on schedule.
        Subscription::batch([
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick),
            iced::window::open_events().map(Message::WindowOpened),
        ])
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

fn audio_device_short(id: &str) -> String {
    if id.is_empty() {
        "—".to_string()
    } else if id.contains("UAC2Gadget") {
        "MERIDIAN".to_string()
    } else if id.contains("vc4hdmi0") || id.contains("vc4-hdmi-0") {
        "HDMI 0".to_string()
    } else if id.contains("vc4hdmi1") || id.contains("vc4-hdmi-1") {
        "HDMI 1".to_string()
    } else {
        id.to_string()
    }
}

/// Format a numeric parameter value for the on-screen readout.
/// Compact value display: ms/Hz → "5", "200", "1.1k"; 0..1 → "0.30";
/// ratios → "1.5".
fn format_value(v: f32) -> String {
    let abs = v.abs();
    if abs >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else if abs >= 100.0 {
        format!("{v:.0}")
    } else if abs >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// Convert a 0..1 linear amplitude to a compact dB string. Floor at
/// −60 dB so the readout reads "−∞ DB" instead of an ever-decreasing
/// noise floor when there's no signal.
/// OUT level-meter ballistics. Attack is instant (the bar snaps up to
/// any higher peak); release is a one-pole fall toward the live peak;
/// the peak-hold tick dwells then falls; the clip latch dwells then
/// clears. Times in seconds.
const METER_RELEASE_TAU: f32 = 0.22;
const METER_HOLD_DWELL_SECS: f32 = 1.2;
const METER_HOLD_FALL_TAU: f32 = 0.5;
const METER_CLIP_HOLD_SECS: f32 = 1.5;

fn format_db(level: f32) -> String {
    if level <= 0.001 {
        "−∞ dB".to_string()
    } else {
        let db = 20.0 * level.log10();
        if db <= -60.0 {
            "−∞ dB".to_string()
        } else {
            format!("{db:.1} dB")
        }
    }
}

impl NativeUi {
    /// Advance the OUT meter ballistics by `dt` seconds, driven by the
    /// 16 ms Tick from the latest `scope_peak` frame so the bar moves at
    /// the UI rate: instant attack, exponential release, a dwelling
    /// peak-hold tick, and a self-clearing clip latch. The `level_meter`
    /// widget renders the resulting state.
    fn advance_meter(&mut self, dt: f32) {
        let target = self.audio.scope_peak;
        // Attack: snap straight up to a higher peak. Release: one-pole
        // fall toward the current window peak (which is ~0 once the part
        // goes quiet, so the bar fades out smoothly but quickly).
        if target >= self.audio.meter_level {
            self.audio.meter_level = target;
        } else {
            let coeff = (-dt / METER_RELEASE_TAU).exp();
            self.audio.meter_level = target + (self.audio.meter_level - target) * coeff;
        }
        // Peak-hold: jump to a new high and reset the dwell; after the
        // dwell expires, fall slowly toward the live level.
        if self.audio.meter_level >= self.audio.meter_hold {
            self.audio.meter_hold = self.audio.meter_level;
            self.audio.meter_hold_remaining = METER_HOLD_DWELL_SECS;
        } else if self.audio.meter_hold_remaining > 0.0 {
            self.audio.meter_hold_remaining -= dt;
        } else {
            let coeff = (-dt / METER_HOLD_FALL_TAU).exp();
            self.audio.meter_hold =
                self.audio.meter_level + (self.audio.meter_hold - self.audio.meter_level) * coeff;
        }
        // Clip latch: armed in the ScopeFrame handler when a window
        // reaches 0 dBFS; here it just counts down.
        if self.audio.meter_clip_remaining > 0.0 {
            self.audio.meter_clip_remaining -= dt;
        }
    }

    fn set_toast(&mut self, message: impl Into<String>, color: iced::Color) {
        self.toast = Some(Toast {
            message: message.into(),
            color,
            expires_at: Instant::now() + TOAST_LIFETIME,
        });
    }

    /// Drives the loaded Lua script's runtime hooks. Called from the
    /// `Message::Tick` handler at ~50 ms cadence (gated by
    /// `last_script_tick`). Sequence:
    ///
    ///   1. Drain the inbound MIDI receiver, queueing every event on
    ///      the engine's MIDI queue.
    ///   2. Process the queued MIDI events, firing on_note / on_cc /
    ///      on_start / on_stop / on_continue hooks.
    ///   3. Call `on_tick(beat)` so coroutine clocks (`clock.run`,
    ///      `clock.sync`, `clock.sleep`) advance against the engine's
    ///      authoritative beat counter.
    ///   4. Check the command file (`scripts/.cmd`) for any pending
    ///      brumectl-pushed load/unload requests.
    ///   5. Check whether the loaded script's source on disk has
    ///      changed; if so, hot-reload it.
    ///   6. Drain accumulated runtime errors and surface them as
    ///      toasts, plus drain the FX-side errors channel populated by
    ///      `LuaFxSlot::with_errors_tx`.
    ///
    /// `try_lock` on the engine mutex — a momentary contention with
    /// the SCRIPT page's `load_script`/`unload` path means we skip
    /// this tick rather than block. Inbound MIDI sits buffered until
    /// the next tick (50 ms later); good enough for a human-pace
    /// feature.
    pub(crate) fn dispatch_script_tick(&mut self) {
        let Some(engine) = self.script_engine.as_ref() else {
            return;
        };
        let Ok(mut e) = engine.try_lock() else {
            return;
        };

        // 1) Drain inbound MIDI from midi-io. Convert each MidiEvent
        // 1:1 into ScriptMidiEvent — the data shapes match because
        // both crates ship the same six variants.
        if let Some(rx) = self.script_midi_rx.as_ref() {
            while let Ok(ev) = rx.try_recv() {
                let queued = match ev {
                    MidiEvent::NoteOn {
                        part,
                        note,
                        velocity,
                    } => ScriptMidiEvent::NoteOn {
                        part,
                        note,
                        velocity,
                    },
                    MidiEvent::NoteOff { part, note } => ScriptMidiEvent::NoteOff { part, note },
                    MidiEvent::Cc { part, cc, value } => ScriptMidiEvent::Cc { part, cc, value },
                    MidiEvent::TransportStart => ScriptMidiEvent::TransportStart,
                    MidiEvent::TransportStop => ScriptMidiEvent::TransportStop,
                    MidiEvent::TransportContinue => ScriptMidiEvent::TransportContinue,
                };
                e.queue_midi(queued);
            }
        }

        // 2-5) Drive the engine. on_tick is parameterised on the
        // current beat so coroutine schedulers can wake at the
        // correct phrase boundary. The engine emits TransportState
        // every ~50ms via engine_ui_rx, so `self.audio.beat` is
        // always within one tick of the authoritative value.
        e.process_midi_queue();
        e.on_tick(self.audio.beat);
        e.check_command_file();
        e.check_hot_reload();

        // 6) Surface errors and freshly-declared script params.
        // `drain_errors` returns String messages accumulated by the
        // engine itself; `drain_params` returns the table of every
        // `brume.add_param`-declared parameter when one was added
        // since the last drain (None means "no change since last
        // call" — a script declaring nothing produces no churn).
        // Errors surface as toasts (latest one wins; sustained
        // errors would be noise). Params replace the local view
        // wholesale, since drain_params already returns the full
        // current state — additive declarations end up here too.
        let mut latest_error: Option<String> = None;
        for msg in e.drain_errors() {
            latest_error = Some(msg);
        }
        let new_params = e.drain_params();
        // Re-read the engine's loaded-script identity AFTER the
        // command-file + hot-reload + drain calls above. Either of
        // the first two can mutate the engine's `loaded_script`
        // (brumectl pushed `load <name>` over the .cmd channel; or
        // the file on disk changed and the loader re-init'd).
        // Without this resync the SCRIPT page would render its
        // stale local mirror until the user navigated away and back.
        let engine_loaded = e.loaded_script().map(str::to_string);
        // Release the engine lock before mutating self.script_params
        // / set_toast — both take &mut self, so we'd otherwise have
        // an aliasing problem.
        drop(e);

        if engine_loaded != self.loaded_script {
            self.loaded_script = engine_loaded;
            // A script swap behind our back resets script_params to
            // whatever drain_params reports next; clear here so the
            // SCRIPT page doesn't briefly show the prior script's
            // sliders if the new one declares nothing.
            self.script_params.clear();
        }

        if let Some(params) = new_params {
            self.script_params = params;
        }

        if let Some(rx) = self.script_fx_errors_rx.as_ref() {
            while let Ok(msg) = rx.try_recv() {
                latest_error = Some(msg);
            }
        }
        if let Some(msg) = latest_error {
            self.set_toast(msg, LEARN_RED);
        }
    }
}

/// Linear / log mapping from a 0..1 controller ratio to a parameter's
/// natural unit. Same math as `KnobBinding::apply` but takes
/// `min`/`max` separately so it works with `mix::FxParamSpec`'s
/// non-binding form.
fn apply_scale(scale: brume_common::Scale, ratio: f32, min: f32, max: f32) -> f32 {
    use brume_common::Scale;
    let r = ratio.clamp(0.0, 1.0);
    match scale {
        Scale::Linear => min + r * (max - min),
        Scale::Log if min > 0.0 && max > min => (min.ln() + r * (max.ln() - min.ln())).exp(),
        Scale::Log => min + r * (max - min),
    }
}

/// Inverse of `apply_scale` — natural unit → 0..1 ratio. Out-of-range
/// values clamp; degenerate ranges return 0 rather than NaN.
fn unapply_scale(scale: brume_common::Scale, value: f32, min: f32, max: f32) -> f32 {
    use brume_common::Scale;
    match scale {
        Scale::Linear if (max - min).abs() > f32::EPSILON => {
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        }
        Scale::Log if min > 0.0 && max > min && value > 0.0 => {
            ((value.ln() - min.ln()) / (max.ln() - min.ln())).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Auto-fit UI scale: detect the connected panel's native resolution
/// and compute the logical→physical factor that maps the canonical
/// 1024×600 design onto it, `min(w/1024, h/600)`. The `min` letter-
/// boxes on the looser axis so the design fits within *both*
/// dimensions — on the reference 1920×1200 panel that's
/// `min(1.875, 2.0) = 1.875`.
///
/// Reads the first connected DRM connector's preferred mode from
/// `/sys/class/drm/*/modes` (same source the SYS page uses for its
/// RESOLUTION readout). Returns `None` when nothing is readable
/// (macOS dev, headless, or an unparseable mode) so the caller falls
/// back to 1.0. An explicit `BRUME_UI_SCALE` always wins over this.
#[cfg(target_os = "linux")]
fn autofit_scale() -> Option<f32> {
    let leading_u32 = |s: &str| -> Option<f32> {
        let n: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
        n.parse::<f32>().ok().filter(|v| *v > 0.0)
    };
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip per-card roots (no '-') and the virtual writeback node,
        // mirroring sys::read_display's connector filter.
        if !name.contains('-') || name.contains("Writeback") {
            continue;
        }
        let connected = std::fs::read_to_string(path.join("status"))
            .map(|s| s.trim() == "connected")
            .unwrap_or(false);
        if !connected {
            continue;
        }
        let modes = std::fs::read_to_string(path.join("modes")).ok()?;
        let (w, h) = modes.lines().next()?.split_once('x')?;
        return Some((leading_u32(w)? / 1024.0).min(leading_u32(h)? / 600.0));
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn autofit_scale() -> Option<f32> {
    None
}

/// Run the native UI. Blocks the calling thread until the window
/// closes. brume-main does engine + audio + midi setup; this
/// function takes the channels and the shared `KnobMapping` it
/// already populated.
///
/// # Errors
/// Returns iced's runtime error if the application can't initialize
/// (e.g. wgpu can't find any rendering backend).
#[allow(clippy::too_many_arguments)]
pub fn run(
    ui_to_engine_tx: Sender<UiToEngine>,
    app_tx: Sender<UiToEngine>,
    engine_ui_rx: Receiver<EngineToUi>,
    knob_mapping: Arc<std::sync::RwLock<KnobMapping>>,
    midi_activity: Arc<parking_lot::Mutex<MidiActivity>>,
    control_matrix: Arc<std::sync::RwLock<ControlMatrix>>,
    midi_learn_active: Arc<AtomicBool>,
    midi_learn_capture_rx: Receiver<CcCaptured>,
    cc_bindings_path: PathBuf,
    transport_path: PathBuf,
    initial_transport: Option<persist::TransportSnapshot>,
    patch_library: Arc<PatchLibrary>,
    script_engine: Option<Arc<std::sync::Mutex<ScriptEngine>>>,
    script_midi_rx: Option<Receiver<MidiEvent>>,
    fx_slot_tx: Sender<Box<dyn brume_fx_chain::FxSlot>>,
    surfaces: Arc<brume_midi_io::ControlSurfaceRegistry>,
) -> iced::Result {
    // Heal CC bindings that pre-date the natural-range / scale
    // pairing. A 0..1 placeholder gets the parameter's full range
    // and scale; a natural-range binding with a stale scale gets
    // the scale corrected. Idempotent — runs every open, no-ops
    // once everything's already aligned.
    let mut matrix_changed = false;
    if let Ok(mut m) = control_matrix.write() {
        for binding in &mut m.bindings {
            let Some((min, max, scale)) = param_natural_range_and_scale(binding.destination) else {
                continue;
            };
            let placeholder = binding.range_min == 0.0 && binding.range_max == 1.0;
            if placeholder {
                binding.range_min = min;
                binding.range_max = max;
                binding.scale = scale;
                matrix_changed = true;
                continue;
            }
            // f32 tolerance so serde round-trips don't false-negative.
            let range_matches =
                (binding.range_min - min).abs() < 0.001 && (binding.range_max - max).abs() < 0.001;
            if range_matches && binding.scale != scale {
                binding.scale = scale;
                matrix_changed = true;
            }
        }
    }

    // Spawn the persist worker once we have the matrix Arc. It owns
    // its own clone; mark_dirty is the only call site users hit.
    let persist = PersistHandle::spawn(control_matrix.clone(), cc_bindings_path);
    if matrix_changed {
        persist.mark_dirty();
    }
    // Same pattern for the transport snapshot — its worker writes
    // ~/.brume/transport.json. Initial value (if any) was loaded by
    // the caller and is plumbed in below; the engine has already been
    // primed with the corresponding SetTempo / SetClockMode messages.
    let transport_persist = persist::TransportPersistHandle::spawn(transport_path);
    // UI scale precedence: an explicit BRUME_UI_SCALE override wins;
    // otherwise auto-fit the canonical 1024×600 design onto the
    // connected panel (1.875 on the MAGEX 1920×1200); failing both
    // (macOS dev, headless) fall back to 1.0.
    let scale: f32 = std::env::var("BRUME_UI_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(autofit_scale)
        .unwrap_or(1.0);

    // BRUME_FULLSCREEN=1 turns off labwc-side decorations and
    // dispatches change_mode(Fullscreen) once the window opens. On
    // dev (Mac) we leave it off so the iced window stays draggable.
    let fullscreen: bool = std::env::var("BRUME_FULLSCREEN")
        .ok()
        .map(|s| s == "1")
        .unwrap_or(false);

    // Hydrate param_values from each engine's per-tab default values
    // so sliders position correctly before the first ParameterChanged
    // echo arrives.
    let mut param_values: HashMap<(u8, ParameterId), f32> = HashMap::new();
    for tabs in [FM_TABS, HARMONIC_TABS, TIMBRAL_TABS, GRANULAR_TABS] {
        for (_label, specs) in tabs {
            for spec in *specs {
                param_values
                    .entry((spec.binding.part, spec.binding.id))
                    .or_insert(spec.default_value);
            }
        }
    }

    // FX-error channel pair. Created only when scripting is
    // available — same gate as `script_engine`. The sender lives on
    // NativeUi as the clone source for each `LuaFxSlot::with_errors_tx`
    // call; the receiver feeds the dispatcher's drain loop, which
    // surfaces messages as toasts. Capacity 32 — generous for an
    // exception path; flooding scripts get unloaded by the user
    // well before this overflows.
    let (lua_fx_errors_tx, script_fx_errors_rx) = if script_engine.is_some() {
        let (tx, rx) = crossbeam_channel::bounded::<String>(32);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let ui = NativeUi {
        page: Page::Engine(Mode::Fm),
        splash: Some(SplashState::new()),
        active_sub_tab: [0; 4],
        param_values,
        audio: AudioState::default(),
        transport_ui: initial_transport
            .map(|s| TransportUi {
                bpm: s.bpm,
                clock_mode_idx: s.clock_mode_idx,
            })
            .unwrap_or_default(),
        transport_persist,
        modulation: ModState::boot(),
        keyboard: KeyboardState::default(),
        mix: MixState::boot(),
        midi_activity,
        control_matrix,
        midi_device_name: String::new(),
        midi_part_channel: [None; 4],
        midi_part_notes: [0; 4],
        active_cc_tab: 0,
        learn: LearnState::new(midi_learn_active, midi_learn_capture_rx),
        loaded_patch_name: [None, None, None, None],
        sys_telemetry: sys::Telemetry::default(),
        last_sys_poll: None,
        // Snapshot the FM patch list at startup so the LIBRARY page
        // renders without a one-frame "empty" flicker. Switching tabs
        // refreshes per-mode.
        lib_patch_names: patch_library.list("fm"),
        patch_library,
        lib_mode: "fm",
        toast: None,
        script_list: Vec::new(),
        loaded_script: None,
        script_params: Vec::new(),
        script_engine,
        fx_slot_tx,
        lua_fx_errors_tx,
        lua_fx_loaded: None,
        lua_fx_list: Vec::new(),
        script_midi_rx,
        last_script_tick: None,
        script_fx_errors_rx,
        persist,
        scale,
        fullscreen,
        knob_mapping,
        ui_to_engine_tx,
        app_tx,
        engine_ui_rx,
        surfaces,
    };

    // Pre-publish the FM tab's first 8 bindings so a connected
    // control surface's knobs route directly to the engine from the
    // moment the UI is up. Every engine / sub-tab switch republishes
    // via republish_knob_mapping.
    ui.republish_knob_mapping();
    // Tell the engine to start scoping the default part so the SCOPE
    // panel has live data on first paint.
    try_send_or_log!(
        ui.ui_to_engine_tx,
        UiToEngine::WatchPart {
            part: ui.active_part(),
        }
    );

    iced::application("Brume", NativeUi::update, NativeUi::view)
        .subscription(NativeUi::subscription)
        .theme(NativeUi::theme)
        .scale_factor(NativeUi::scale_factor)
        .font(include_bytes!("../assets/InstrumentSerif-Italic.ttf").as_slice())
        .window_size((1024.0, 600.0))
        .decorations(!fullscreen)
        .run_with(|| (ui, Task::none()))
}

/// Action vocabulary control surface drivers use to drive `NativeUi`.
/// Methods delegate to the existing inherent methods one-for-one, so
/// the trait is a thin seam, not a new code path. Surfaces never see
/// `NativeUi` itself — only this trait.
impl ControlSurfaceApi for NativeUi {
    fn is_on_mix_page(&self) -> bool {
        matches!(self.page, Page::Mix)
    }

    fn is_on_mod_page(&self) -> bool {
        matches!(self.page, Page::Mod)
    }

    fn set_part_level(&mut self, part: u8, level: f32) {
        if (part as usize) >= self.mix.levels.len() {
            return;
        }
        let level = level.clamp(0.0, 1.0);
        self.mix.levels[part as usize] = level;
        try_send_or_log!(
            self.ui_to_engine_tx,
            UiToEngine::SetPartLevel { part, level }
        );
    }

    fn toggle_mix_mute(&mut self, part: u8) {
        NativeUi::toggle_mix_mute(self, part);
    }

    fn toggle_mix_solo(&mut self, part: u8) {
        NativeUi::toggle_mix_solo(self, part);
    }

    fn cycle_engine(&mut self, delta: i32) {
        NativeUi::cycle_engine(self, delta);
    }

    fn cycle_sub_tab(&mut self, delta: i32) {
        NativeUi::cycle_sub_tab(self, delta);
    }

    fn apply_fx_knob(&mut self, slot: usize, ratio: f32) {
        NativeUi::apply_fx_knob(self, slot, ratio);
    }

    fn apply_mod_knob(&mut self, slot: usize, ratio: f32) {
        NativeUi::apply_mod_knob(self, slot, ratio);
    }

    fn is_on_script_page(&self) -> bool {
        matches!(self.page, Page::Script)
    }

    fn apply_script_knob(&mut self, slot: usize, ratio: f32) {
        NativeUi::apply_script_knob(self, slot, ratio);
    }

    fn select_engine(&mut self, index: u8) {
        NativeUi::select_engine(self, index);
    }

    fn select_sub_tab(&mut self, index: u8) {
        NativeUi::select_sub_tab(self, index);
    }
}
