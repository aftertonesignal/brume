// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Message protocol between Brume's UI / control layer and audio engine.

use std::borrow::Cow;

use brume_common::ParameterId;
use brume_modulation::ModSource;
use serde::{Deserialize, Serialize};

/// Transport clock source — mirrors the engine's internal `ClockMode`
/// but lives in the protocol so the engine doesn't have to allocate a
/// `String` on the audio thread to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMode {
    Internal,
    External,
}

/// Max modulation assignments reported per part in a `ModFrame`.
/// Anything beyond this is silently truncated — the UI only displays
/// the first few anyway. Lets `ModFrame::assignments` be a stack array
/// so the audio thread never allocates to report mod state.
pub const MAX_MOD_ASSIGNMENTS: usize = 8;

/// Messages sent from the UI or control layer to the audio engine.
///
/// Received by the engine via a bounded channel. All variants must be
/// safe to construct on any thread and cheap to send.
///
/// `part` identifies the target part (0 = FM, 1 = Harmonic, 2 = Timbral).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiToEngine {
    /// Set a parameter on a specific part.
    SetParameter {
        part: u8,
        id: ParameterId,
        value: f32,
    },

    /// Trigger a note on a specific part.
    NoteOn { part: u8, note: u8, velocity: f32 },

    /// Release a note on a specific part.
    NoteOff { part: u8, note: u8 },

    /// Pitch-bend wheel for a part, normalized to [-1, 1] (0 = center).
    /// The engine scales this by its bend range and applies it to every
    /// sounding voice on the part.
    PitchBend { part: u8, bend: f32 },

    /// Mod wheel (CC1) for a part, normalized to [0, 1]. Drives vibrato
    /// depth by default.
    ModWheel { part: u8, value: f32 },

    /// Set the global master volume (0.0-1.0).
    SetMasterVolume(f32),

    /// Release all notes on a specific part (MIDI CC 123).
    AllNotesOff { part: u8 },

    /// Release all notes on all parts.
    AllSoundOff,

    /// Configure an LFO on a part.
    SetLfo {
        part: u8,
        lfo: u8,       // 0 = LFO1, 1 = LFO2
        shape: String, // TransitionShape name
        rate: f32,     // Hz
        mode: String,  // "loop" or "trig"
    },

    /// Set a step sequencer step value on a part.
    SetSeqStep {
        part: u8,
        seq: u8,    // 0 = SEQ1, 1 = SEQ2
        step: u8,   // 0-7
        value: f32, // 0.0-1.0
    },

    /// Configure a step sequencer on a part.
    SetSeqConfig {
        part: u8,
        seq: u8,
        steps: u8,       // 1-8
        trigger: String, // "note", "beat", "half", "quarter"
    },

    /// Add a modulation assignment to a part.
    AddAssignment {
        part: u8,
        source: String, // "Lfo1", "Lfo2", "Seq1", "Seq2"
        dest: String,   // ParameterId name
        depth: f32,
    },

    /// Remove a modulation assignment by index.
    RemoveAssignment { part: u8, index: u8 },

    /// Set the depth of an existing assignment.
    SetAssignmentDepth { part: u8, index: u8, depth: f32 },

    /// Set a part's mixer level (0.0-1.0).
    SetPartLevel { part: u8, level: f32 },

    /// Mute/unmute a part.
    SetPartMute { part: u8, muted: bool },

    /// Set a part's delay send level (0.0-1.0).
    SetPartDelaySend { part: u8, level: f32 },

    /// Set a part's reverb send level (0.0-1.0).
    SetPartReverbSend { part: u8, level: f32 },

    /// Set an FX parameter by slot name and param name.
    SetFxParam {
        slot: String,
        param: String,
        value: f32,
    },

    /// Remove a named FX slot from the chain.
    RemoveFxSlot(String),

    /// Set the internal clock BPM.
    SetTempo(f32),

    /// Set clock mode: "internal" or "external".
    SetClockMode(String),

    /// MIDI clock tick (0xF8) — 24 per quarter note.
    MidiClockTick,

    /// MIDI Start (0xFA).
    MidiStart,

    /// MIDI Stop (0xFC).
    MidiStop,

    /// MIDI Continue (0xFB).
    MidiContinue,

    /// Switch a part's oscillator engine mode.
    SetOscillatorMode { part: u8, mode: String },

    /// Request the audio subsystem switch to a different output device.
    /// The app layer (not the engine) tears down and re-opens the cpal
    /// stream, then persists the choice to `~/.brume/settings.json`.
    /// `device_id` is a cpal `Device::name()` string.
    SetOutputDevice { device_id: String },

    /// Request the current list of available audio output devices.
    /// The app layer replies by pushing an `EngineToUi::OutputDeviceList`.
    RequestOutputDeviceList,

    /// Request the audio subsystem rebuild the cpal stream at a new
    /// latency preset (buffer size). The app layer tears down and
    /// re-opens the stream, persists the choice to settings, and echoes
    /// back an `EngineToUi::AudioLatency`. Ignored on non-gadget output
    /// devices, which keep their own buffer.
    SetAudioLatency { preset: brume_common::LatencyPreset },

    /// Tells the engine which part the UI is currently watching on the
    /// SYNTH page. The engine only emits `ScopeFrame` / `ModFrame` for
    /// this part — the UI never renders scope/mod for inactive tabs, so
    /// serializing frames for them wastes bridge bandwidth + CPU.
    /// Sent on every tab change and once at startup. `u8::MAX` signals
    /// "UI is not on the synth page" and disables all scope/mod pushes.
    WatchPart { part: u8 },
}

/// Messages sent from the audio engine back to the UI layer.
///
/// Published by the engine via a bounded, non-blocking channel. The UI
/// polls these to update displays, meters, and transport indicators.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineToUi {
    /// A parameter value changed on some `part`. Emitted by the engine
    /// every time `UiToEngine::SetParameter` is applied — from any source
    /// (UI itself, Lua scripts, MIDI ControlMatrix, modulation router,
    /// smoothing updates). The UI listens and reconciles its slider DOM
    /// state so external sources visibly move the corresponding bar.
    /// UI-originated changes echo back but the resulting DOM update is
    /// idempotent (value already matches slider state).
    ParameterChanged {
        part: u8,
        id: ParameterId,
        value: f32,
    },

    /// Authoritative full snapshot of every parameter the engine has
    /// recorded a value for. Emitted periodically (~1 Hz) so the UI can
    /// reconcile its `param_values` against the engine's actual state
    /// even if individual `SetParameter` messages or `ParameterChanged`
    /// echoes were dropped on a saturated channel.
    ///
    /// Merge semantics on the UI side: each entry overwrites the
    /// matching `(part, id)` slot in `param_values`. Entries the
    /// engine has never been told about don't appear here, so the
    /// UI's startup hydration of per-tab defaults survives.
    ParameterSnapshot { params: Vec<ParameterSnapshotEntry> },

    /// Periodic engine status for the status bar.
    EngineStatus { voice_count: u8, cpu_percent: f32 },

    /// Current transport state — pushed by the engine roughly in sync
    /// with the UI's 50ms timer. In External clock mode, `bpm` reflects
    /// the measured external clock tempo.
    TransportState {
        /// Effective BPM — internal setting or external clock estimate.
        bpm: f32,
        /// Internal clock or external MIDI clock.
        mode: TransportMode,
        /// Whether the transport is currently rolling.
        playing: bool,
        /// Fractional beat position from start (for beat indicators).
        beat: f64,
    },

    /// Enumerated audio output devices + which one is currently active.
    /// Pushed by the app layer in response to `RequestOutputDeviceList`
    /// and after any successful `SetOutputDevice`. UI uses this to
    /// populate the SYS page selector.
    OutputDeviceList {
        /// Currently-open device id (cpal `Device::name()` string).
        /// May be empty on failure/early-startup edge cases.
        current: String,
        /// Every available output device.
        devices: Vec<OutputDeviceEntry>,
    },

    /// Current audio output latency preset. Pushed at startup, after a
    /// `SetAudioLatency`, and alongside the device list so the SYS page
    /// selector reflects the persisted/active value.
    AudioLatency { preset: brume_common::LatencyPreset },

    /// Per-part audio level summary — peak and RMS computed by the
    /// engine over a short window of recent dry mono samples. The UI
    /// renders a ballistic dB level meter from these two floats, so the
    /// bridge only needs two floats per tick instead of a sample array.
    /// Pushed at the same cadence as ModFrame; only for the part the UI
    /// is currently watching.
    ScopeFrame {
        /// Part index (0 = FM, 1 = Harmonic, 2 = Timbral, 3 = Granular).
        part: u8,
        /// Peak absolute value over the window. Bounded to a +6 dB
        /// ceiling (linear 2.0) rather than clamped at 0 dBFS, so the UI
        /// can show and redline signal that runs over full scale.
        peak: f32,
        /// RMS over the window, bounded to the same +6 dB ceiling.
        rms: f32,
    },

    /// Per-part modulation-source snapshot — raw source values plus the
    /// currently-enabled assignment table. Raw values are in [0, 1]
    /// (pre-shape, pre-depth); the UI maintains its own history ring for
    /// each source to draw a short rolling trace, and composes labels
    /// from the assignments (e.g. "LFO1 > FilterCutoff"). Pushed at the
    /// same ~20 Hz cadence as TransportState.
    ///
    /// `assignments` is a fixed-size `Option` array, not a `Vec`, so the
    /// audio thread never allocates to publish mod state. The UI stringifies
    /// the enum variants for display off the RT thread.
    ModFrame {
        part: u8,
        lfo1: f32,
        lfo2: f32,
        seq1: f32,
        seq2: f32,
        assignments: [Option<ModAssignmentSnapshot>; MAX_MOD_ASSIGNMENTS],
    },

    /// Raw CC event from a recognised hardware control surface.
    /// Bypasses the MIDI ControlMatrix — the UI owns the mapping
    /// logic because it knows which engine and sub-tab are currently
    /// visible. `kind` identifies the surface so the UI can dispatch
    /// to the matching driver.
    ///
    /// `Cow` so shipped drivers (whose `id()` returns `&'static str`)
    /// produce `Borrowed` and never allocate on the MIDI input
    /// thread; future Lua- or community-defined surfaces can
    /// produce `Owned` if they need to.
    ControllerCc {
        kind: Cow<'static, str>,
        cc: u8,
        value: f32,
    },

    /// Raw Note event from a recognised control surface. Same
    /// `Cow` rationale as `ControllerCc`.
    ControllerNote {
        kind: Cow<'static, str>,
        note: u8,
        velocity: f32,
        on: bool,
    },
}

/// One entry in [`EngineToUi::ParameterSnapshot`]. Stable wire shape
/// for the UI's reconciliation pass; same `(part, id, value)` triple
/// as `ParameterChanged`, just batched.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ParameterSnapshotEntry {
    pub part: u8,
    pub id: ParameterId,
    pub value: f32,
}

/// A single enabled modulation assignment as it appears on the wire.
/// All-Copy so the audio thread can populate an array of these without
/// touching the allocator. The UI formats `source` and `destination`
/// into display strings off the RT thread.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModAssignmentSnapshot {
    pub source: ModSource,
    pub destination: ParameterId,
    /// Signed depth, -1.0..1.0. Sign indicates invert; magnitude is the
    /// modulation amount.
    pub depth: f32,
}

/// One audio output device entry for `EngineToUi::OutputDeviceList`.
/// Wire-format mirror of `brume_audio_io::OutputDeviceInfo`, kept in
/// `app-protocol` to avoid a cross-crate dependency from protocol → audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputDeviceEntry {
    /// Stable identifier (cpal `Device::name()`).
    pub id: String,
    /// Friendly display label ("Meridian (USB to DAW)", etc.).
    pub label: String,
    /// Whether this device is the host's current default.
    pub is_default: bool,
}
