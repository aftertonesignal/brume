// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MIDI input for Brume — scans USB MIDI devices and routes events to the engine.
//!
//! Connects to **every** available MIDI input port (excluding ALSA's virtual
//! "Midi Through" loopback), parses MIDI bytes from each, and sends
//! `UiToEngine` messages on the bounded channel. All ports feed the same
//! engine; this lets Brume simultaneously accept, for example, a CC
//! control surface AND the Meridian USB gadget's `f_midi` endpoint that
//! carries notes/clock from the host DAW.
//!
//! Note events are routed by channel via `MidiChannelMap`. CC events are
//! looked up in the `ControlMatrix`. Recognised control surfaces (matched
//! via `ControlSurfaceRegistry::by_port`) bypass the matrix and reach the
//! UI as raw `ControllerCc` / `ControllerNote` events for the driver's
//! `handle_cc` / `handle_note` to dispatch.

pub mod activity;
pub mod control_surface;

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use parking_lot::Mutex;

use brume_app_protocol::{EngineToUi, UiToEngine};
use brume_common::{KnobMapping, try_send_or_log};
use brume_control_model::ControlMatrix;
use crossbeam_channel::Sender;

pub use activity::MidiActivity;
pub use control_surface::{ControlSurface, ControlSurfaceApi, ControlSurfaceRegistry};

/// Returns true when `BRUME_LOG_CC=1` is set in the environment.
/// Cached on first call to keep the per-CC overhead at one atomic load.
/// Off by default — knob automation at 200 Hz × 8 CCs would otherwise
/// flood the journal during normal play.
fn log_cc_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var("BRUME_LOG_CC").as_deref() == Ok("1"))
}

/// A lightweight MIDI event for forwarding to the script engine.
#[derive(Debug, Clone)]
pub enum MidiEvent {
    NoteOn { part: u8, note: u8, velocity: f32 },
    NoteOff { part: u8, note: u8 },
    Cc { part: u8, cc: u8, value: f32 },
    TransportStart,
    TransportStop,
    TransportContinue,
}

/// Raw CC event captured during MIDI Learn mode. Forwarded to the UI for
/// binding creation instead of being routed to the engine via
/// `ControlMatrix::lookup`.
#[derive(Debug, Clone, Copy)]
pub struct CcCaptured {
    pub channel: u8,
    pub cc: u8,
    pub value: u8,
}

/// Shared state used to implement MIDI Learn without tearing down the
/// MIDI thread. When `active` is set, the MIDI input handler diverts
/// the next incoming CC event through `capture_tx` instead of consulting
/// the `ControlMatrix`. The UI's LEARN button flips `active`; the
/// handler clears `active` itself after capturing one event so the very
/// next CC turn locks the binding.
#[derive(Clone)]
pub struct MidiLearnState {
    pub active: Arc<AtomicBool>,
    pub capture_tx: Sender<CcCaptured>,
}

/// One or more running MIDI input connections.
///
/// MIDI processing stops when this is dropped (all held connections drop
/// together).
pub struct BrumeMidiInput {
    _connections: Vec<midir::MidiInputConnection<()>>,
    port_names: Vec<String>,
}

impl BrumeMidiInput {
    /// Scans for MIDI input ports and connects to every available port
    /// except ALSA's virtual `Midi Through` loopback.
    ///
    /// The callback parses MIDI bytes and sends `UiToEngine` messages.
    /// Returns `None` if no MIDI input port is found or every attempt to
    /// connect failed (not an error — the instrument works without MIDI).
    #[must_use]
    pub fn start(
        engine_tx: Sender<UiToEngine>,
        control_matrix: Arc<RwLock<ControlMatrix>>,
        activity: Arc<Mutex<MidiActivity>>,
    ) -> Option<Self> {
        let empty_mapping: Arc<RwLock<KnobMapping>> = Arc::new(RwLock::new([None; 8]));
        let empty_surfaces = Arc::new(ControlSurfaceRegistry::new(vec![]));
        Self::start_with_script_tx(
            engine_tx,
            control_matrix,
            activity,
            None,
            None,
            None,
            empty_mapping,
            empty_surfaces,
        )
    }

    /// Start with an optional channel for forwarding MIDI events to the
    /// script engine, an optional `MidiLearnState` for MIDI Learn, an
    /// optional `EngineToUi` sender that recognised control surfaces
    /// push raw `ControllerCc` / `ControllerNote` events into so the
    /// UI can apply screen-follows mapping, a shared `KnobMapping`
    /// that lets recognised surfaces' audio-priority knobs route
    /// directly to the engine's `SetParameter` (no UI round-trip),
    /// and a `ControlSurfaceRegistry` that decides which port belongs
    /// to which surface and which CCs are knob-priority on that
    /// surface.
    #[must_use]
    pub fn start_with_script_tx(
        engine_tx: Sender<UiToEngine>,
        control_matrix: Arc<RwLock<ControlMatrix>>,
        activity: Arc<Mutex<MidiActivity>>,
        script_tx: Option<Sender<MidiEvent>>,
        learn: Option<MidiLearnState>,
        ui_tx: Option<Sender<EngineToUi>>,
        knob_mapping: Arc<RwLock<KnobMapping>>,
        surfaces: Arc<ControlSurfaceRegistry>,
    ) -> Option<Self> {
        // midir's MidiInput is consumed by .connect(), so one scan decides
        // which names to target; each target gets a fresh MidiInput below.
        let scan = match midir::MidiInput::new("Brume-scan") {
            Ok(m) => m,
            Err(e) => {
                eprintln!("brume midi: failed to create input: {e}");
                return None;
            }
        };

        let scan_ports = scan.ports();
        if scan_ports.is_empty() {
            eprintln!("brume midi: no input ports found");
            return None;
        }

        let targets: Vec<String> = scan_ports
            .iter()
            .filter_map(|p| scan.port_name(p).ok())
            .filter(|name| !name.contains("Midi Through"))
            .collect();
        drop(scan);

        if targets.is_empty() {
            eprintln!("brume midi: no connectable ports (all were 'Midi Through')");
            return None;
        }

        let mut connections: Vec<midir::MidiInputConnection<()>> =
            Vec::with_capacity(targets.len());
        let mut connected_names: Vec<String> = Vec::with_capacity(targets.len());

        for target_name in &targets {
            let midi_in = match midir::MidiInput::new("Brume") {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("brume midi: failed to create input for {target_name:?}: {e}");
                    continue;
                }
            };
            let ports = midi_in.ports();
            let port = ports
                .iter()
                .find(|p| midi_in.port_name(p).ok().as_deref() == Some(target_name.as_str()))
                .cloned();
            let Some(port) = port else {
                eprintln!("brume midi: port {target_name:?} disappeared between scan and connect");
                continue;
            };

            let tx = engine_tx.clone();
            let matrix = control_matrix.clone();
            let act = activity.clone();
            let stx = script_tx.clone();
            let lrn = learn.clone();
            let utx = ui_tx.clone();
            let kmap = knob_mapping.clone();
            let surf = Arc::clone(&surfaces);

            // Ask the registry which surface (if any) owns this port.
            // The id is `&'static str` so we can capture it straight
            // into the closure; the Arc-cloned registry resolves it
            // back to a `&dyn ControlSurface` per CC for `rt_knob_slot`.
            let surface_id: Option<&'static str> =
                surfaces.by_port(target_name).map(ControlSurface::id);
            if let Some(id) = surface_id {
                eprintln!("brume midi: {target_name:?} recognised as control surface ({id})");
            }

            eprintln!("brume midi: connecting to {target_name:?}");
            let conn = match midi_in.connect(
                &port,
                "brume-input",
                move |_timestamp, data, _| {
                    handle_midi_message(
                        data,
                        &tx,
                        &matrix,
                        &act,
                        &stx,
                        lrn.as_ref(),
                        surface_id,
                        utx.as_ref(),
                        &kmap,
                        &surf,
                    );
                },
                (),
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("brume midi: connect to {target_name:?} failed: {e}");
                    continue;
                }
            };
            eprintln!("brume midi: connected to {target_name:?}");

            connections.push(conn);
            connected_names.push(target_name.clone());
        }

        if connections.is_empty() {
            eprintln!("brume midi: no ports connected");
            return None;
        }

        // Publish the joined name list to activity so the UI can surface which
        // devices are live. Keep the existing single-string field format for
        // backward compat with the JS side.
        activity.lock().device_name = connected_names.join(" + ");

        Some(Self {
            _connections: connections,
            port_names: connected_names,
        })
    }

    /// Names of every connected MIDI port, in connection order.
    #[must_use]
    pub fn port_names(&self) -> &[String] {
        &self.port_names
    }
}

/// Parses a raw MIDI message and sends the appropriate engine message.
/// If `surface_id` is Some, CC and Note events are additionally
/// mirrored to the UI as raw `ControllerCc` / `ControllerNote` events
/// (via `ui_tx`) — this is how recognised surfaces reach the
/// screen-follows mapping logic without going through the MIDI
/// ControlMatrix. The `surfaces` registry resolves the id back to a
/// `&dyn ControlSurface` for the RT short-circuit (`rt_knob_slot`).
#[allow(clippy::too_many_arguments)]
fn handle_midi_message(
    data: &[u8],
    tx: &Sender<UiToEngine>,
    matrix: &Arc<RwLock<ControlMatrix>>,
    activity: &Arc<Mutex<MidiActivity>>,
    script_tx: &Option<Sender<MidiEvent>>,
    learn: Option<&MidiLearnState>,
    surface_id: Option<&'static str>,
    ui_tx: Option<&Sender<EngineToUi>>,
    knob_mapping: &Arc<RwLock<KnobMapping>>,
    surfaces: &ControlSurfaceRegistry,
) {
    if data.is_empty() {
        return;
    }

    let status = data[0];
    let msg_type = status & 0xF0;
    let channel = status & 0x0F;

    match msg_type {
        // Note Off
        0x80 if data.len() >= 3 => {
            activity.lock().note_off(channel, data[1]);

            // Control surface: forward to UI as raw ControllerNote (off).
            if let (Some(id), Some(utx)) = (surface_id, ui_tx) {
                try_send_or_log!(
                    utx,
                    EngineToUi::ControllerNote {
                        kind: Cow::Borrowed(id),
                        note: data[1],
                        velocity: 0.0,
                        on: false,
                    }
                );
                return;
            }

            let matrix_guard = matrix.read().ok();
            let part = matrix_guard
                .as_ref()
                .and_then(|m| m.channel_map.part_for_channel(channel));

            if let Some(part) = part {
                try_send_or_log!(
                    tx,
                    UiToEngine::NoteOff {
                        part,
                        note: data[1]
                    }
                );
                if let Some(stx) = script_tx {
                    try_send_or_log!(
                        stx,
                        MidiEvent::NoteOff {
                            part,
                            note: data[1]
                        }
                    );
                }
            }
        }

        // Note On
        0x90 if data.len() >= 3 => {
            {
                let mut act = activity.lock();
                if data[2] == 0 {
                    act.note_off(channel, data[1]);
                } else {
                    act.note_on(channel, data[1], data[2]);
                }
            }

            // Control surface: forward to UI as raw ControllerNote.
            if let (Some(id), Some(utx)) = (surface_id, ui_tx) {
                let velocity = f32::from(data[2]) / 127.0;
                try_send_or_log!(
                    utx,
                    EngineToUi::ControllerNote {
                        kind: Cow::Borrowed(id),
                        note: data[1],
                        velocity,
                        on: data[2] > 0,
                    }
                );
                return;
            }

            let matrix_guard = matrix.read().ok();
            let part = matrix_guard
                .as_ref()
                .and_then(|m| m.channel_map.part_for_channel(channel));

            if let Some(part) = part {
                let velocity = f32::from(data[2]) / 127.0;
                if data[2] == 0 {
                    try_send_or_log!(
                        tx,
                        UiToEngine::NoteOff {
                            part,
                            note: data[1]
                        }
                    );
                    if let Some(stx) = script_tx {
                        try_send_or_log!(
                            stx,
                            MidiEvent::NoteOff {
                                part,
                                note: data[1]
                            }
                        );
                    }
                } else {
                    try_send_or_log!(
                        tx,
                        UiToEngine::NoteOn {
                            part,
                            note: data[1],
                            velocity
                        }
                    );
                    if let Some(stx) = script_tx {
                        try_send_or_log!(
                            stx,
                            MidiEvent::NoteOn {
                                part,
                                note: data[1],
                                velocity
                            }
                        );
                    }
                }
            }
        }

        // Control Change
        0xB0 if data.len() >= 3 => {
            let cc = data[1];
            let cc_value = data[2];

            match cc {
                // All Sound Off — kill everything
                120 => {
                    try_send_or_log!(tx, UiToEngine::AllSoundOff);
                }
                // All Notes Off — release notes on this channel's part
                123 => {
                    let matrix_guard = matrix.read().ok();
                    if let Some(part) = matrix_guard
                        .as_ref()
                        .and_then(|m| m.channel_map.part_for_channel(channel))
                    {
                        try_send_or_log!(tx, UiToEngine::AllNotesOff { part });
                    }
                }
                // Regular CC — track activity, look up in control matrix, forward to script
                _ => {
                    activity.lock().cc(channel, cc, cc_value);

                    // MIDI Learn diversion — checked BEFORE the
                    // surface auto-routing below. Recognised surfaces
                    // normally short-circuit into ControllerCc with an
                    // early return, which would mean Learn could never
                    // see their knobs — the user would hear the sound
                    // change but the LISTENING card would stay empty.
                    // Learn always wins.
                    if let Some(l) = learn {
                        if l.active.load(Ordering::Acquire) {
                            // Continuous capture — each arriving CC updates
                            // the UI card so the user can wiggle the
                            // intended knob and watch the capture follow.
                            // SAVE / CANCEL in the UI sends stop_midi_learn
                            // to clear the flag explicitly.
                            try_send_or_log!(
                                l.capture_tx,
                                CcCaptured {
                                    channel,
                                    cc,
                                    value: cc_value,
                                }
                            );
                            return;
                        }
                    }

                    // Built-in controller dispatch. The driver's
                    // `rt_knob_slot` decides which CCs route directly
                    // to the engine via the shared `KnobMapping` —
                    // bypasses the UI thread so the audio path is not
                    // gated by paint pressure. The slider visual
                    // updates via the engine's `ParameterChanged`
                    // echo. CCs the driver doesn't claim as knobs
                    // (buttons, unclaimed knobs) flow to the UI as
                    // `ControllerCc` for the driver's `handle_cc` to
                    // dispatch.
                    if let (Some(id), Some(utx)) = (surface_id, ui_tx) {
                        let ratio = f32::from(cc_value) / 127.0;
                        if let Some(surface) = surfaces.by_id(id) {
                            if let Some(slot) = surface.rt_knob_slot(cc) {
                                let binding = knob_mapping.read().ok().and_then(|m| m[slot]);
                                if let Some(b) = binding {
                                    try_send_or_log!(
                                        tx,
                                        UiToEngine::SetParameter {
                                            part: b.part,
                                            id: b.id,
                                            value: b.apply(ratio),
                                        }
                                    );
                                    return;
                                }
                                // Slot known but unbound for this
                                // sub-tab — fall through to ControllerCc
                                // so handle_cc still sees the event.
                            }
                        }
                        try_send_or_log!(
                            utx,
                            EngineToUi::ControllerCc {
                                kind: Cow::Borrowed(id),
                                cc,
                                value: ratio,
                            }
                        );
                        return;
                    }

                    let matrix_guard = matrix.read().ok();
                    let part = matrix_guard
                        .as_ref()
                        .and_then(|m| m.channel_map.part_for_channel(channel));

                    if let Some(matrix_ref) = matrix_guard.as_ref() {
                        let hit = matrix_ref.lookup(channel, cc, cc_value);
                        if log_cc_enabled() {
                            match &hit {
                                Some((p, id, v)) => eprintln!(
                                    "brume cc: ch{} cc{}={} → part={} {:?}={}",
                                    channel + 1,
                                    cc,
                                    cc_value,
                                    p,
                                    id,
                                    v,
                                ),
                                None => eprintln!(
                                    "brume cc: ch{} cc{}={} (no binding)",
                                    channel + 1,
                                    cc,
                                    cc_value,
                                ),
                            }
                        }
                        if let Some((part, param_id, value)) = hit {
                            try_send_or_log!(
                                tx,
                                UiToEngine::SetParameter {
                                    part,
                                    id: param_id,
                                    value,
                                }
                            );
                        }
                    }

                    // Check FX CC bindings
                    if let Some(matrix_ref) = matrix_guard.as_ref() {
                        for (slot, param, value) in matrix_ref.lookup_fx(cc, cc_value) {
                            try_send_or_log!(tx, UiToEngine::SetFxParam { slot, param, value });
                        }
                    }

                    // Forward to script engine
                    if let (Some(part), Some(stx)) = (part, script_tx) {
                        let value = f32::from(cc_value) / 127.0;
                        try_send_or_log!(stx, MidiEvent::Cc { part, cc, value });
                    }

                    // Mod wheel (CC1): drive vibrato by default. Skipped
                    // when the user has Learned CC1 to a parameter — the
                    // matrix lookup above already handled it — so the two
                    // don't double-fire.
                    if cc == 1 {
                        let learned = matrix_guard
                            .as_ref()
                            .is_some_and(|m| m.lookup(channel, cc, cc_value).is_some());
                        if !learned {
                            if let Some(part) = part {
                                let value = f32::from(cc_value) / 127.0;
                                try_send_or_log!(tx, UiToEngine::ModWheel { part, value });
                            }
                        }
                    }
                }
            }
        }

        // Pitch bend — 14-bit, centered at 8192. Routed to the channel's
        // part like notes; the engine scales it by its bend range.
        0xE0 if data.len() >= 3 => {
            let raw = (u16::from(data[2] & 0x7F) << 7) | u16::from(data[1] & 0x7F);
            let bend = ((f32::from(raw) - 8192.0) / 8192.0).clamp(-1.0, 1.0);
            let matrix_guard = matrix.read().ok();
            if let Some(part) = matrix_guard
                .as_ref()
                .and_then(|m| m.channel_map.part_for_channel(channel))
            {
                try_send_or_log!(tx, UiToEngine::PitchBend { part, bend });
            }
        }

        _ => {}
    }

    // System real-time messages (single byte, no channel)
    match status {
        0xF8 => {
            try_send_or_log!(tx, UiToEngine::MidiClockTick);
        }
        0xFA => {
            try_send_or_log!(tx, UiToEngine::MidiStart);
            if let Some(stx) = script_tx {
                try_send_or_log!(stx, MidiEvent::TransportStart);
            }
        }
        0xFB => {
            try_send_or_log!(tx, UiToEngine::MidiContinue);
            if let Some(stx) = script_tx {
                try_send_or_log!(stx, MidiEvent::TransportContinue);
            }
        }
        0xFC => {
            try_send_or_log!(tx, UiToEngine::MidiStop);
            if let Some(stx) = script_tx {
                try_send_or_log!(stx, MidiEvent::TransportStop);
            }
        }
        _ => {}
    }
}
