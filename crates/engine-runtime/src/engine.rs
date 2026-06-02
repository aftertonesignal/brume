// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Top-level audio engine: 4-part multi-timbral orchestration.

use std::collections::HashMap;

use brume_app_protocol::{
    EngineToUi, MAX_MOD_ASSIGNMENTS, ModAssignmentSnapshot, ParameterSnapshotEntry, TransportMode,
    UiToEngine,
};
use brume_common::{OscillatorMode, ParameterId};
use brume_dsp_core::{Limiter, Smoother};
use brume_fx_chain::FxChain;

use crate::transport::{ClockMode, TimeDivision, Transport};

use brume_modulation::{ModSource, TransitionShape};

use crate::part::Part;

fn parse_shape(name: &str) -> Option<TransitionShape> {
    Some(match name {
        "ChaosHeavy" | "Caffeinated" => TransitionShape::ChaosHeavy,
        "LinearUp" | "RampUp" => TransitionShape::LinearUp,
        "LinearDown" | "RampDown" => TransitionShape::LinearDown,
        "ExpAttack" | "SlowBurn" => TransitionShape::ExpAttack,
        "ExpDecay" | "Freefall" => TransitionShape::ExpDecay,
        "LogAttack" | "QuickDraw" => TransitionShape::LogAttack,
        "LogDecay" | "LongTail" => TransitionShape::LogDecay,
        "CircularIn" | "Molasses" => TransitionShape::CircularIn,
        "CircularOut" | "Catapult" => TransitionShape::CircularOut,
        "SCurveSmooth" | "Silk" => TransitionShape::SCurveSmooth,
        "SCurveSharp" | "Whiplash" => TransitionShape::SCurveSharp,
        "BounceIn" | "Trampoline" => TransitionShape::BounceIn,
        "BounceOut" | "Overshoot" => TransitionShape::BounceOut,
        "ChaosLight" | "Jitters" => TransitionShape::ChaosLight,
        "Random" | "DiceRoll" => TransitionShape::Random,
        "DC" | "Guillotine" => TransitionShape::DC,
        "BloomIn" | "Unfurl" => TransitionShape::BloomIn,
        "BloomOut" | "ChunkWedge" => TransitionShape::BloomOut,
        "SlowStart" | "Reluctant" => TransitionShape::SlowStart,
        "SlowEnd" | "LazyLanding" => TransitionShape::SlowEnd,
        "FastStart" | "EagerBeaver" => TransitionShape::FastStart,
        "FastEnd" | "Coasting" => TransitionShape::FastEnd,
        "CircularInOut" | "Switchback" => TransitionShape::CircularInOut,
        "CircularOutIn" | "Foothill" => TransitionShape::CircularOutIn,
        _ => return None,
    })
}

fn parse_mod_source(name: &str) -> Option<ModSource> {
    Some(match name {
        "Lfo1" => ModSource::Lfo1,
        "Lfo2" => ModSource::Lfo2,
        "Seq1" => ModSource::Seq1,
        "Seq2" => ModSource::Seq2,
        _ => return None,
    })
}

const NUM_PARTS: usize = 4;
/// Scope ring size per part. 256 samples @ 48 kHz ≈ 5.3 ms.
const SCOPE_SAMPLES: usize = 256;

/// Ceiling for the `ScopeFrame` peak/RMS the meter renders: +6 dB
/// (linear 2.0). Bounding rather than clamping at 0 dBFS (1.0) lets
/// the UI meter show — and redline — a per-part stem running over full
/// scale (the condition that drives it into the stem soft-saturator),
/// while still keeping the value finite for the bridge.
const METER_CEILING: f32 = 2.0;
/// Max frames rendered in one inner loop iteration.
const CHUNK: usize = 2048;

/// 4-part multi-timbral audio engine. Moved into the audio callback —
/// must not block.
pub struct BrumeEngine {
    parts: [Part; NUM_PARTS],
    fx_chain: FxChain,
    transport: Transport,
    limiter: Limiter,
    master_volume: Smoother,
    sample_rate: f32,
    delay_sync: TimeDivision,
    fx_slot_rx: Option<crossbeam_channel::Receiver<Box<dyn brume_fx_chain::FxSlot>>>,
    ui_tx: Option<crossbeam_channel::Sender<EngineToUi>>,
    ui_push_counter: u32,
    /// Per-part mono scratch. Heaped — 4 × `CHUNK` f32s = 32 KB is too
    /// big for the cpal thread stack on some platforms.
    per_part_mono: Box<[[f32; CHUNK]; NUM_PARTS]>,
    scope_rings: Box<[[f32; SCOPE_SAMPLES]; NUM_PARTS]>,
    scope_pos: [usize; NUM_PARTS],
    /// Part the UI is currently viewing. Only this part's scope + mod
    /// frames get serialised — no point emitting for tabs that aren't
    /// rendered. `u8::MAX` = UI off the SYNTH page, no emissions.
    watch_part: u8,
    /// Coalesced ParameterChanged echoes. SetParameter handlers
    /// update or push into this Vec; `process_block` drains it at the
    /// end of each audio block. The result: at most one echo per
    /// `(part, id)` per block (~5 ms), so a 50 Hz CC stream from
    /// midi-io cannot back the engine→UI channel up with stale values
    /// faster than the UI tick can drain. The Vec is pre-allocated
    /// to its working capacity at construction; steady-state Push +
    /// drain stays allocation-free on the audio thread.
    pending_echoes: Vec<PendingEcho>,
    /// Authoritative record of every parameter the engine has ever
    /// accepted a `SetParameter` for, keyed by `(part, id)`. Updated
    /// in lock-step with the part's own state on every accepted
    /// `set_parameter`; read out wholesale ~1 Hz to emit
    /// `EngineToUi::ParameterSnapshot`, which the UI uses to
    /// reconcile `param_values` against the engine's reality. Catches
    /// drift from dropped `SetParameter` messages (UI→engine channel
    /// saturation) and dropped `ParameterChanged` echoes alike — the
    /// snapshot is the safety net that lets the UI converge to the
    /// engine's actual state within a second of any divergence.
    param_state: HashMap<(u8, ParameterId), f32>,
}

/// One pending parameter echo. Lives in `BrumeEngine::pending_echoes`
/// between SetParameter handling and end-of-block drain.
struct PendingEcho {
    part: u8,
    id: ParameterId,
    value: f32,
}

/// Soft-saturate a sample toward the ±1.0 ceiling. Identity for
/// `|x| ≤ 0.8` (transparent below the knee), then a smooth tanh
/// roll-off to the ±1.0 asymptote above. C¹-continuous at the
/// knee so there's no audible kink as a swelling envelope crosses
/// the threshold.
///
/// The knee is set deliberately wide (0.8 of full-scale) — most
/// signal stays below it and passes through unchanged, only the
/// transient peaks that would otherwise hard-clip see any shaping.
/// Used on the Meridian per-part stem path where the master
/// limiter doesn't run.
///
/// Cost: one branch, one subtraction, one `.tanh()` call per
/// sample (the `tanh` only fires for samples above the knee, and
/// stems-mode renders one sample per output frame per part — so
/// even at the worst case the per-block cost is well under a
/// percent of the audio budget).
#[inline]
fn soft_saturate(x: f32) -> f32 {
    let abs = x.abs();
    if abs <= 0.8 {
        x
    } else {
        let sign = x.signum();
        // tanh saturates toward ±1 as input grows, so 0.2 *
        // tanh((abs - 0.8) / 0.2) maps the range [0.8, ∞) onto
        // [0, 0.2). Adding the 0.8 base puts the asymptote at
        // ±1.0. Derivative at the knee = 1 (continuous with
        // the identity branch).
        sign * (0.8 + 0.2 * ((abs - 0.8) / 0.2).tanh())
    }
}

impl BrumeEngine {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let parts = [
            Part::new(OscillatorMode::Fm, sample_rate),
            Part::new(OscillatorMode::Harmonic, sample_rate),
            Part::new(OscillatorMode::Timbral, sample_rate),
            Part::new(OscillatorMode::Granular, sample_rate),
        ];

        let mut limiter = Limiter::new(sample_rate);
        limiter.set_threshold_db(-1.0);
        limiter.set_release_ms(100.0);

        Self {
            parts,
            fx_chain: FxChain::new(sample_rate),
            transport: Transport::new(sample_rate),
            limiter,
            master_volume: Smoother::new(0.8, 10.0, sample_rate),
            sample_rate,
            delay_sync: TimeDivision::Free,
            fx_slot_rx: None,
            ui_tx: None,
            ui_push_counter: 0,
            per_part_mono: Box::new([[0.0_f32; CHUNK]; NUM_PARTS]),
            scope_rings: Box::new([[0.0_f32; SCOPE_SAMPLES]; NUM_PARTS]),
            scope_pos: [0; NUM_PARTS],
            watch_part: 0,
            // Capacity sized for the worst realistic case: 8
            // simultaneously-spinning controller knobs + a few
            // touch-driven param changes + headroom. Linear search
            // for the (part, id) lookup is O(n) but n stays well
            // under 64 in practice — cheaper than the hashing cost
            // for a HashMap on the audio thread.
            pending_echoes: Vec::with_capacity(64),
            // ~50 ParameterIds × 4 parts = 200 entries upper bound;
            // 256 capacity covers that without ever rehashing.
            param_state: HashMap::with_capacity(256),
        }
    }

    /// Registers the engine→UI channel. `try_send` throughout; the
    /// channel is bounded and drops silently under back-pressure.
    pub fn set_ui_tx(&mut self, tx: crossbeam_channel::Sender<EngineToUi>) {
        self.ui_tx = Some(tx);
    }

    /// Flushes coalesced `ParameterChanged` echoes — at most one per
    /// (part, id) since the last call. `drain(..)` preserves the Vec's
    /// capacity so we stay allocation-free in steady state.
    ///
    /// Normally called at the end of `process_block` so the UI sees one
    /// echo per audio block per (part, id) regardless of how many
    /// SetParameter messages arrived in between. The MIDI drainer
    /// thread (in apps/brume-main, see #6) also calls this directly so
    /// echoes still flow when the audio callback isn't running because
    /// the host stopped consuming samples.
    pub fn flush_parameter_echoes(&mut self) {
        if let Some(ref tx) = self.ui_tx {
            for e in self.pending_echoes.drain(..) {
                let _ = tx.try_send(EngineToUi::ParameterChanged {
                    part: e.part,
                    id: e.id,
                    value: e.value,
                });
            }
        } else {
            self.pending_echoes.clear();
        }
    }

    /// Runs `f` against the `Part` at `part_index`, bounds-checked. Returns
    /// whether the part existed — lets callers gate follow-up side effects
    /// (UI echoes, logs) on a valid index without re-checking.
    fn with_part<F: FnOnce(&mut Part)>(&mut self, part_index: u8, f: F) -> bool {
        match self.parts.get_mut(part_index as usize) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        }
    }

    /// Dispatches a UI/control message. Called from the audio callback
    /// after draining the channel — must not allocate or block.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_message(&mut self, msg: UiToEngine) {
        match msg {
            UiToEngine::NoteOn {
                part,
                note,
                velocity,
            } => {
                self.with_part(part, |p| p.note_on(note, velocity));
            }
            UiToEngine::NoteOff { part, note } => {
                self.with_part(part, |p| p.note_off(note));
                // Promiscuous note-off across parts: prevents stuck notes
                // when the host switches MIDI channel mid-phrase.
                for (i, p) in self.parts.iter_mut().enumerate() {
                    if i != part as usize {
                        p.note_off(note);
                    }
                }
            }
            UiToEngine::PitchBend { part, bend } => {
                self.with_part(part, |p| p.set_pitch_bend(bend));
            }
            UiToEngine::ModWheel { part, value } => {
                self.with_part(part, |p| p.set_mod_wheel(value));
            }
            UiToEngine::SetParameter { part, id, value } => {
                let applied = self.with_part(part, |p| p.set_parameter(id, value));
                // Coalesce echo per (part, id) into pending_echoes;
                // process_block flushes at end of block. Caps emit
                // rate at the audio block rate (~200 Hz / parameter)
                // so a 50 Hz CC stream cannot back up the engine→UI
                // channel with stale values, which is what stomped
                // fresh slider paint in the prior naive-echo version
                // (engine.rs comment from before this commit).
                if applied {
                    // Record the new value as authoritative for the
                    // periodic ParameterSnapshot reconciliation.
                    self.param_state.insert((part, id), value);

                    let mut updated = false;
                    for e in self.pending_echoes.iter_mut() {
                        if e.part == part && e.id == id {
                            e.value = value;
                            updated = true;
                            break;
                        }
                    }
                    if !updated && self.pending_echoes.len() < self.pending_echoes.capacity() {
                        self.pending_echoes.push(PendingEcho { part, id, value });
                    }
                }
            }
            UiToEngine::SetMasterVolume(value) => {
                self.master_volume.set_target(value.clamp(0.0, 1.0));
            }
            UiToEngine::AllNotesOff { part } => {
                self.with_part(part, Part::all_notes_off);
            }
            UiToEngine::AllSoundOff => {
                for part in &mut self.parts {
                    part.all_notes_off();
                }
            }
            UiToEngine::SetLfo {
                part,
                lfo,
                shape,
                rate,
                mode,
            } => {
                self.with_part(part, |p| {
                    let lfo_ref = if lfo == 0 {
                        &mut p.modulation.lfo1
                    } else {
                        &mut p.modulation.lfo2
                    };
                    if let Some(s) = parse_shape(&shape) {
                        lfo_ref.set_shape(s);
                    }
                    lfo_ref.set_rate_hz(rate);
                    lfo_ref.set_mode(if mode == "trig" {
                        brume_modulation::LfoMode::Trig
                    } else {
                        brume_modulation::LfoMode::Loop
                    });
                });
            }
            UiToEngine::SetSeqStep {
                part,
                seq,
                step,
                value,
            } => {
                self.with_part(part, |p| {
                    let seq_ref = if seq == 0 {
                        &mut p.modulation.seq1
                    } else {
                        &mut p.modulation.seq2
                    };
                    seq_ref.set_step_value(step as usize, value);
                });
            }
            UiToEngine::SetSeqConfig {
                part,
                seq,
                steps,
                trigger,
            } => {
                self.with_part(part, |p| {
                    let seq_ref = if seq == 0 {
                        &mut p.modulation.seq1
                    } else {
                        &mut p.modulation.seq2
                    };
                    seq_ref.set_num_steps(steps as usize);
                    seq_ref.set_trigger(match trigger.as_str() {
                        "note" => brume_modulation::StepTrigger::NoteOn,
                        "half" => brume_modulation::StepTrigger::HalfBeat,
                        "quarter" => brume_modulation::StepTrigger::QuarterBeat,
                        _ => brume_modulation::StepTrigger::Beat,
                    });
                });
            }
            UiToEngine::AddAssignment {
                part,
                source,
                dest,
                depth,
            } => {
                let Some(src) = parse_mod_source(&source) else {
                    return;
                };
                let Ok(dst) = dest.parse::<brume_common::ParameterId>() else {
                    return;
                };
                self.with_part(part, |p| {
                    p.modulation
                        .add_assignment(brume_modulation::ModulationAssignment {
                            source: src,
                            destination: dst,
                            depth,
                            shape: brume_modulation::TransitionShape::LinearUp,
                            enabled: true,
                        });
                });
            }
            UiToEngine::RemoveAssignment { part, index } => {
                self.with_part(part, |p| {
                    let assignments = p.modulation.assignments_mut();
                    if (index as usize) < assignments.len() {
                        assignments.remove(index as usize);
                    }
                });
            }
            UiToEngine::SetPartLevel { part, level } => {
                self.with_part(part, |p| p.level.set_target(level.clamp(0.0, 1.0)));
            }
            UiToEngine::SetPartMute { part, muted } => {
                self.with_part(part, |p| p.muted = muted);
            }
            UiToEngine::SetAssignmentDepth { part, index, depth } => {
                self.with_part(part, |p| {
                    if let Some(a) = p.modulation.assignments_mut().get_mut(index as usize) {
                        a.depth = depth;
                    }
                });
            }
            UiToEngine::SetPartDelaySend { part, level } => {
                self.with_part(part, |p| p.delay_send.set_target(level.clamp(0.0, 1.0)));
            }
            UiToEngine::SetPartReverbSend { part, level } => {
                self.with_part(part, |p| p.reverb_send.set_target(level.clamp(0.0, 1.0)));
            }
            UiToEngine::SetFxParam {
                ref slot,
                ref param,
                value,
            } => {
                // Intercept delay sync changes
                if slot == "Delay" && param == "sync" {
                    // Index 0=FREE, 1=MIDI(=1/4), 2=1/1, 3=1/2, 4=1/4, 5=1/8, 6=1/16, 7=1/4d, 8=1/8d, 9=1/4t, 10=1/8t
                    let names = [
                        "FREE", "1/4", "1/1", "1/2", "1/4", "1/8", "1/16", "1/4d", "1/8d", "1/4t",
                        "1/8t",
                    ];
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let idx = value as usize;
                    let name = names.get(idx).unwrap_or(&"FREE");
                    self.delay_sync = TimeDivision::from_name(name);
                    // If synced, update delay time from transport
                    if let Some(beats) = self.delay_sync.beats() {
                        let ms = self.transport.division_to_ms(beats);
                        self.fx_chain.set_param("Delay", "time", ms);
                    }
                } else {
                    self.fx_chain.set_param(slot, param, value);
                }
            }
            UiToEngine::SetTempo(bpm) => {
                self.transport.set_bpm(bpm);
                // Update delay if synced
                if let Some(beats) = self.delay_sync.beats() {
                    let ms = self.transport.division_to_ms(beats);
                    self.fx_chain.set_param("Delay", "time", ms);
                }
            }
            UiToEngine::SetClockMode(ref mode) => {
                match mode.as_str() {
                    "external" | "slave" => self.transport.set_mode(ClockMode::External),
                    "master" => self.transport.set_mode(ClockMode::Internal), // master = internal + send (output handled separately)
                    _ => self.transport.set_mode(ClockMode::Internal),
                }
            }
            UiToEngine::MidiClockTick => {
                self.transport.midi_clock_tick();
                // Update delay if synced to external clock
                if self.transport.mode() == ClockMode::External {
                    if let Some(beats) = self.delay_sync.beats() {
                        let ms = self.transport.division_to_ms(beats);
                        self.fx_chain.set_param("Delay", "time", ms);
                    }
                }
            }
            UiToEngine::RemoveFxSlot(ref name) => {
                self.fx_chain.remove_by_name(name);
            }
            UiToEngine::MidiStart => {
                self.transport.midi_start();
            }
            UiToEngine::MidiStop => {
                self.transport.midi_stop();
            }
            UiToEngine::MidiContinue => {
                self.transport.midi_continue();
            }
            UiToEngine::SetOscillatorMode { part, ref mode } => {
                let osc_mode = match mode.as_str() {
                    "fm" => Some(brume_common::OscillatorMode::Fm),
                    "harmonic" => Some(brume_common::OscillatorMode::Harmonic),
                    "timbral" => Some(brume_common::OscillatorMode::Timbral),
                    "granular" => Some(brume_common::OscillatorMode::Granular),
                    _ => None,
                };
                if let Some(m) = osc_mode {
                    self.with_part(part, |p| p.set_mode(m));
                }
            }
            UiToEngine::WatchPart { part } => {
                self.watch_part = part;
            }
            _ => {}
        }
    }

    /// Renders one interleaved audio block.
    ///
    /// Output layout by channel count:
    /// - **2**: master mix (all parts → sat/chorus inserts → delay/reverb
    ///   sends → master volume → limiter).
    /// - **8**: Meridian stems. Each part's dry, pre-FX mono output is
    ///   duplicated to L/R of its dedicated pair — FM 1-2,
    ///   Harmonic 3-4, Timbral 5-6, Granular 7-8. Master bus still
    ///   computes (for local HDMI/DAC monitoring) but isn't emitted.
    /// - **other**: master L/R on channels 0-1, zero-fill beyond.
    pub fn process_block(&mut self, output: &mut [f32], channels: usize) {
        let mut part_buf = [0.0_f32; CHUNK];
        let mut dry_buf = [0.0_f32; CHUNK];
        let mut delay_bus = [0.0_f32; CHUNK];
        let mut reverb_bus = [0.0_f32; CHUNK];

        // Drain any new Lua FX slots into the chain. Replace-by-name
        // semantics: the UI sends one slot per script load, always
        // tagged with the same engine-side name ("LuaFx" by
        // convention). Removing first guarantees we never accumulate
        // dead Lua slots even if the UI's pre-load `RemoveFxSlot`
        // message is dropped on a saturated channel.
        //
        // (Previous logged each addition via `eprintln!`, which is a
        // stderr syscall on the audio thread — fine on a quiet tty,
        // can stall on a slow journald. The channel ack already
        // implies success; if a slot ever fails to register we
        // surface it through the existing engine→UI channel rather
        // than the audio path.)
        if let Some(ref rx) = self.fx_slot_rx {
            while let Ok(slot) = rx.try_recv() {
                let name = slot.name().to_string();
                self.fx_chain.remove_by_name(&name);
                self.fx_chain.push(slot);
            }
        }

        let mut frame_offset = 0;
        let channels_safe = channels.max(1);
        let total_frames = output.len() / channels_safe;

        while frame_offset < total_frames {
            let frames = (total_frames - frame_offset).min(CHUNK);

            // Advance transport clock
            self.transport.advance(frames);

            // Render each part. Save its mono output into per_part_mono so
            // the 8-channel stem layout can emit it directly later; also
            // accumulate into dry_buf / send buses for the master path.
            for (pi, part) in self.parts.iter_mut().enumerate() {
                part.process_buffer(frames, &mut part_buf);
                let stem = &mut self.per_part_mono[pi];
                for i in 0..frames {
                    let sample = part_buf[i];
                    let d_send = part.delay_send.process();
                    let r_send = part.reverb_send.process();

                    stem[i] = sample;
                    dry_buf[i] += sample;
                    delay_bus[i] += sample * d_send;
                    reverb_bus[i] += sample * r_send;
                    part_buf[i] = 0.0;
                }
            }

            // Tap each part's dry mono stem into its scope ring. The
            // ~20 Hz UI push below snapshots the ring into a ScopeFrame.
            for pi in 0..NUM_PARTS {
                let stem = &self.per_part_mono[pi];
                let ring = &mut self.scope_rings[pi];
                let mut pos = self.scope_pos[pi];
                for i in 0..frames {
                    ring[pos] = stem[i];
                    pos = (pos + 1) % SCOPE_SAMPLES;
                }
                self.scope_pos[pi] = pos;
            }

            // Expand dry mono to stereo L/R for FX processing
            let mut left_buf = [0.0_f32; CHUNK];
            let mut right_buf = [0.0_f32; CHUNK];

            // Apply master volume to dry signal
            for i in 0..frames {
                let volume = self.master_volume.process();
                left_buf[i] = dry_buf[i] * volume;
                right_buf[i] = left_buf[i]; // mono → stereo
                dry_buf[i] = 0.0;
            }

            // Process insert effects on the stereo dry bus: the two
            // built-in master inserts followed by any Lua FX slots.
            // Looked up by name so FxChain::new can reorder built-ins
            // or add new ones without this loop caring.
            if let Some(sat) = self
                .fx_chain
                .slot_mut_by_name(brume_fx_chain::SATURATOR_SLOT)
            {
                sat.process_stereo(&mut left_buf[..frames], &mut right_buf[..frames]);
            }
            if let Some(chorus) = self.fx_chain.slot_mut_by_name(brume_fx_chain::CHORUS_SLOT) {
                chorus.process_stereo(&mut left_buf[..frames], &mut right_buf[..frames]);
            }
            for slot in self.fx_chain.custom_slots_mut() {
                slot.process_stereo(&mut left_buf[..frames], &mut right_buf[..frames]);
            }

            // Process send effects — extract WET return only.
            // Send effects use their MIX param as return level.
            // We process the send bus, then subtract the original dry send
            // so only the wet component is added to the master mix.

            // Delay send
            let mut delay_l = [0.0_f32; CHUNK];
            let mut delay_r = [0.0_f32; CHUNK];
            delay_l[..frames].copy_from_slice(&delay_bus[..frames]);
            delay_r[..frames].copy_from_slice(&delay_bus[..frames]);
            if let Some(delay) = self.fx_chain.slot_mut_by_name(brume_fx_chain::DELAY_SLOT) {
                delay.process_stereo(&mut delay_l[..frames], &mut delay_r[..frames]);
            }
            // Subtract original dry to isolate wet return
            for i in 0..frames {
                delay_l[i] -= delay_bus[i];
                delay_r[i] -= delay_bus[i];
            }

            // Reverb send
            let mut reverb_l = [0.0_f32; CHUNK];
            let mut reverb_r = [0.0_f32; CHUNK];
            reverb_l[..frames].copy_from_slice(&reverb_bus[..frames]);
            reverb_r[..frames].copy_from_slice(&reverb_bus[..frames]);
            if let Some(reverb) = self.fx_chain.slot_mut_by_name(brume_fx_chain::REVERB_SLOT) {
                reverb.process_stereo(&mut reverb_l[..frames], &mut reverb_r[..frames]);
            }
            for i in 0..frames {
                reverb_l[i] -= reverb_bus[i];
                reverb_r[i] -= reverb_bus[i];
            }

            // Finalise master mix regardless of output channel count —
            // cheap, and keeps state (limiter, smoothers) consistent so
            // switching device layouts mid-session doesn't click.
            let mut master_l = [0.0_f32; CHUNK];
            let mut master_r = [0.0_f32; CHUNK];
            for i in 0..frames {
                let sum_l = left_buf[i] + delay_l[i] + reverb_l[i];
                let sum_r = right_buf[i] + delay_r[i] + reverb_r[i];

                // Clamp before limiter to prevent DSP state corruption
                let safe_l = sum_l.clamp(-2.0, 2.0);
                let safe_r = sum_r.clamp(-2.0, 2.0);

                let (out_l, out_r) = self.limiter.process_stereo(safe_l, safe_r);
                master_l[i] = out_l;
                master_r[i] = out_r;
            }

            // Emit to the interleaved output according to the requested
            // channel layout.
            let slice =
                &mut output[frame_offset * channels_safe..(frame_offset + frames) * channels_safe];
            match channels_safe {
                2 => {
                    // Stereo master mix.
                    for (i, frame) in slice.chunks_exact_mut(2).enumerate() {
                        frame[0] = master_l[i];
                        frame[1] = master_r[i];
                    }
                }
                8 => {
                    // Meridian per-part stems: dry, pre-send, mono→L+R
                    // of each part's dedicated channel pair. Master FX
                    // output stays internal here — the local HDMI / DAC
                    // monitor path opens its own stereo stream.
                    //
                    // Soft-saturate before stem output. Per-part stems
                    // skip the master path's FX chain + limiter, so
                    // peaks that approach or cross ±1.0 (legal under FM
                    // at moderate-to-high modulation index across many
                    // patch + algorithm combinations) need bounding
                    // before the cpal → UAC2 gadget endpoint converts
                    // to int. A hard `clamp(-1.0, 1.0)` prevented the
                    // int wraparound that was producing broken-glass
                    // fold-clip — but the hard discontinuity at ±1.0
                    // was itself audible whenever the signal envelope
                    // crowded the ceiling, regardless of whether the
                    // peak was above or just at ±1.0. `soft_saturate`
                    // is C¹-continuous, identity below a 0.8 knee, and
                    // asymptotic toward ±1.0 above — so signals
                    // comfortably below the knee pass through
                    // unchanged, peaks at or near ±1.0 round off
                    // musically instead of square off into harmonic
                    // distortion, and the cpal endpoint never sees a
                    // value outside (-1.0, 1.0).
                    for (f, frame) in slice.chunks_exact_mut(8).enumerate() {
                        for p in 0..NUM_PARTS {
                            let sample = soft_saturate(self.per_part_mono[p][f]);
                            frame[p * 2] = sample;
                            frame[p * 2 + 1] = sample;
                        }
                    }
                }
                n => {
                    // Fallback: route master L/R to channels 0/1 and
                    // zero-fill the remainder. Covers unexpected
                    // hardware (1ch mono, 3ch, etc.) safely.
                    for (i, frame) in slice.chunks_exact_mut(n).enumerate() {
                        frame[0] = master_l[i];
                        if n > 1 {
                            frame[1] = master_r[i];
                        }
                        for c in 2..n {
                            frame[c] = 0.0;
                        }
                    }
                }
            }

            // Clear send buses for next chunk
            for i in 0..frames {
                delay_bus[i] = 0.0;
                reverb_bus[i] = 0.0;
            }

            frame_offset += frames;
        }

        // try_send throughout — drops silently on a full channel so the
        // audio thread never blocks on the UI.
        self.ui_push_counter = self.ui_push_counter.wrapping_add(1);

        self.flush_parameter_echoes();

        // Transport + status: every 10th block (~20 Hz at typical buffer
        // sizes). Beat indicators want freshness; voice count doesn't.
        if self.ui_push_counter % 10 == 0 {
            if let Some(ref tx) = self.ui_tx {
                let mode = match self.transport.mode() {
                    ClockMode::Internal => TransportMode::Internal,
                    ClockMode::External => TransportMode::External,
                };
                let _ = tx.try_send(EngineToUi::TransportState {
                    bpm: self.transport.bpm(),
                    mode,
                    playing: self.transport.is_playing(),
                    beat: self.transport.beat_position(),
                });
                let _ = tx.try_send(EngineToUi::EngineStatus {
                    voice_count: self.active_voice_count(),
                    cpu_percent: 0.0,
                });
            }
        }

        // Parameter snapshot: every ~200th block (~1 Hz at 256-sample
        // blocks @ 48 kHz). Catches UI/engine state drift from any
        // dropped SetParameter / ParameterChanged on a saturated
        // channel. Allocation cost on the audio thread: one Vec of
        // ~250 × 12 bytes = ~3 KB, once per second. That's a
        // sub-microsecond malloc on a CM5 — sits well under the noise
        // floor next to the existing 20 Hz TransportState + 40 Hz
        // ScopeFrame stream. If profiling ever flags this, the buffer
        // can be moved to a non-audio thread by adding a snapshot
        // request channel.
        if self.ui_push_counter % 200 == 0 && !self.param_state.is_empty() {
            if let Some(ref tx) = self.ui_tx {
                let params: Vec<ParameterSnapshotEntry> = self
                    .param_state
                    .iter()
                    .map(|(&(part, id), &value)| ParameterSnapshotEntry { part, id, value })
                    .collect();
                let _ = tx.try_send(EngineToUi::ParameterSnapshot { params });
            }
        }

        // Scope + mod for the watched part only, every 5th block (~40 Hz).
        // 20 Hz read as visibly blocky on the LFO/scope bars even with
        // fractional block glyphs.
        if self.ui_push_counter % 5 == 0 && self.watch_part != u8::MAX {
            if let Some(ref tx) = self.ui_tx {
                let pi = self.watch_part as usize;
                if pi < NUM_PARTS {
                    // Peak + RMS over the ring — the UI renders a
                    // ballistic dB level meter from these two floats; no
                    // sample array crosses the bridge.
                    let ring = &self.scope_rings[pi];
                    let mut peak = 0.0_f32;
                    let mut sum_sq = 0.0_f32;
                    for &s in ring.iter() {
                        let a = s.abs();
                        if a > peak {
                            peak = a;
                        }
                        sum_sq += s * s;
                    }
                    let rms = (sum_sq / SCOPE_SAMPLES as f32).sqrt();
                    let _ = tx.try_send(EngineToUi::ScopeFrame {
                        part: self.watch_part,
                        peak: peak.min(METER_CEILING),
                        rms: rms.min(METER_CEILING),
                    });

                    // Mod — pack up to MAX_MOD_ASSIGNMENTS into a stack
                    // array, no allocation. UI formats the enum names for
                    // display off the audio thread.
                    let mr = &self.parts[pi].modulation;
                    let mut assignments: [Option<ModAssignmentSnapshot>; MAX_MOD_ASSIGNMENTS] =
                        [None; MAX_MOD_ASSIGNMENTS];
                    let mut idx = 0;
                    for a in mr.assignments() {
                        if !a.enabled {
                            continue;
                        }
                        if idx >= MAX_MOD_ASSIGNMENTS {
                            break;
                        }
                        assignments[idx] = Some(ModAssignmentSnapshot {
                            source: a.source,
                            destination: a.destination,
                            depth: a.depth,
                        });
                        idx += 1;
                    }
                    let _ = tx.try_send(EngineToUi::ModFrame {
                        part: self.watch_part,
                        lfo1: mr.lfo1.value(),
                        lfo2: mr.lfo2.value(),
                        seq1: mr.seq1.current_value(),
                        seq2: mr.seq2.current_value(),
                        assignments,
                    });
                }
            }
        }
    }

    /// Returns a mutable reference to a specific part's modulation router.
    pub fn part_modulation_mut(
        &mut self,
        part: usize,
    ) -> Option<&mut brume_modulation::ModulationRouter> {
        self.parts.get_mut(part).map(|p| &mut p.modulation)
    }

    /// Returns a mutable reference to a specific part.
    pub fn part_mut(&mut self, part: usize) -> Option<&mut Part> {
        self.parts.get_mut(part)
    }

    /// Total active voice count across all parts.
    #[must_use]
    pub fn active_voice_count(&self) -> u8 {
        self.parts.iter().map(Part::active_voice_count).sum()
    }

    /// Sets the receiver for Lua FX slots to be inserted into the chain.
    pub fn set_fx_slot_receiver(
        &mut self,
        rx: crossbeam_channel::Receiver<Box<dyn brume_fx_chain::FxSlot>>,
    ) {
        self.fx_slot_rx = Some(rx);
    }

    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brume_common::ParameterId;

    #[test]
    fn soft_saturate_is_identity_below_knee() {
        // The whole point of the knee at 0.8 is that signals below
        // it pass through bit-exact. Anything else would mean
        // unintentional shaping of nominal-level audio.
        for &x in &[-0.8, -0.5, -0.1, 0.0, 0.1, 0.5, 0.8] {
            assert_eq!(soft_saturate(x), x, "expected identity at {x}");
        }
    }

    #[test]
    fn soft_saturate_is_bounded_above_knee() {
        // Whatever we feed it, output must stay inside [-1.0, 1.0].
        // cpal's f32 → int conversion treats 1.0 as INT_MAX cleanly
        // (no wrap), so reaching the asymptote exactly is fine —
        // values above it would wrap, which is what we're preventing.
        // f32's `tanh` saturates to exactly 1.0 above ~10 input,
        // hence the inclusive bound here.
        for &x in &[0.81, 1.0, 1.5, 2.0, 5.0, 100.0, f32::INFINITY] {
            let y = soft_saturate(x);
            assert!(y > 0.0 && y <= 1.0, "x={x} → y={y} not in (0, 1]");
            let y_neg = soft_saturate(-x);
            assert!(
                y_neg < 0.0 && y_neg >= -1.0,
                "x={} → y={y_neg} not in [-1, 0)",
                -x
            );
        }
    }

    #[test]
    fn soft_saturate_is_continuous_at_knee() {
        // The piecewise function should join cleanly at |x| = 0.8.
        // Tiny step across the threshold should produce a tiny step
        // in output — a discontinuity here would manifest as audible
        // ticking when an envelope's amplitude crosses the knee.
        let just_below = soft_saturate(0.7999);
        let at_knee = soft_saturate(0.8);
        let just_above = soft_saturate(0.8001);
        assert!((just_below - at_knee).abs() < 1e-3);
        assert!((just_above - at_knee).abs() < 1e-3);
    }

    #[test]
    fn engine_produces_output() {
        let mut engine = BrumeEngine::new(48000.0);
        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 69,
            velocity: 0.8,
        });

        let mut buffer = vec![0.0_f32; 1024];
        engine.process_block(&mut buffer, 2);

        let energy: f32 = buffer.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "engine should produce non-zero output");
    }

    #[test]
    fn engine_voice_count() {
        let mut engine = BrumeEngine::new(48000.0);
        assert_eq!(engine.active_voice_count(), 0);

        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 60,
            velocity: 0.8,
        });
        assert_eq!(engine.active_voice_count(), 1);

        engine.handle_message(UiToEngine::NoteOn {
            part: 1,
            note: 64,
            velocity: 0.8,
        });
        assert_eq!(engine.active_voice_count(), 2);
    }

    #[test]
    fn parameter_snapshot_reflects_set_parameter() {
        let mut engine = BrumeEngine::new(48000.0);
        let (tx, rx) = crossbeam_channel::bounded(1024);
        engine.set_ui_tx(tx);

        engine.handle_message(UiToEngine::SetParameter {
            part: 0,
            id: ParameterId::FmIndex,
            value: 3.5,
        });
        engine.handle_message(UiToEngine::SetParameter {
            part: 1,
            id: ParameterId::HarmonicLevel1,
            value: 0.7,
        });

        // ui_push_counter increments at the end of each process_block,
        // and the snapshot emission fires every 200th block. Drive 200
        // blocks so we cross the threshold once. With a 512-frame
        // buffer (256 samples × 2 channels) each call is ~5 ms of
        // simulated audio — total runtime is sub-second.
        let mut buf = vec![0.0_f32; 512];
        for _ in 0..200 {
            engine.process_block(&mut buf, 2);
        }

        let mut snapshot: Option<Vec<ParameterSnapshotEntry>> = None;
        while let Ok(msg) = rx.try_recv() {
            if let EngineToUi::ParameterSnapshot { params } = msg {
                snapshot = Some(params);
            }
        }

        let params = snapshot.expect("expected at least one ParameterSnapshot");
        let find = |part: u8, id: ParameterId| {
            params
                .iter()
                .find(|e| e.part == part && e.id == id)
                .map(|e| e.value)
        };
        assert_eq!(
            find(0, ParameterId::FmIndex),
            Some(3.5),
            "FmIndex on part 0 should be present in the snapshot"
        );
        assert_eq!(
            find(1, ParameterId::HarmonicLevel1),
            Some(0.7),
            "HarmonicLevel1 on part 1 should be present in the snapshot"
        );
    }

    #[test]
    fn engine_note_off_releases() {
        let mut engine = BrumeEngine::new(48000.0);
        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 69,
            velocity: 0.8,
        });

        let mut buf = vec![0.0; 512];
        engine.process_block(&mut buf, 2);

        engine.handle_message(UiToEngine::NoteOff { part: 0, note: 69 });

        let mut buf = vec![0.0; 48000];
        engine.process_block(&mut buf, 2);

        assert_eq!(engine.active_voice_count(), 0, "voice should release");
    }

    #[test]
    fn engine_parts_are_independent() {
        let mut engine = BrumeEngine::new(48000.0);

        // Play on part 0 (FM)
        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 60,
            velocity: 0.8,
        });

        // Play on part 2 (Timbral)
        engine.handle_message(UiToEngine::NoteOn {
            part: 2,
            note: 72,
            velocity: 0.7,
        });

        assert_eq!(engine.active_voice_count(), 2);

        // Release only part 0
        engine.handle_message(UiToEngine::NoteOff { part: 0, note: 60 });

        let mut buf = vec![0.0; 48000];
        engine.process_block(&mut buf, 2);

        // Part 2 should still have a voice (or be releasing)
        // Part 0 should be done
        assert!(engine.active_voice_count() <= 1);
    }

    #[test]
    fn engine_three_parts_simultaneously() {
        let mut engine = BrumeEngine::new(48000.0);

        // All three parts playing
        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 60,
            velocity: 0.8,
        });
        engine.handle_message(UiToEngine::NoteOn {
            part: 1,
            note: 64,
            velocity: 0.7,
        });
        engine.handle_message(UiToEngine::NoteOn {
            part: 2,
            note: 67,
            velocity: 0.6,
        });

        let mut buffer = vec![0.0_f32; 1024];
        engine.process_block(&mut buffer, 2);

        let energy: f32 = buffer.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "three parts should produce output");
        assert_eq!(engine.active_voice_count(), 3);
    }

    #[test]
    fn engine_parameter_targets_part() {
        let mut engine = BrumeEngine::new(48000.0);

        // Set FM index on part 0 only
        engine.handle_message(UiToEngine::SetParameter {
            part: 0,
            id: ParameterId::FmIndex,
            value: 5.0,
        });

        // Set timbre on part 2 only
        engine.handle_message(UiToEngine::SetParameter {
            part: 2,
            id: ParameterId::Timbre,
            value: 0.8,
        });

        // Play both and verify they produce different sounds
        engine.handle_message(UiToEngine::NoteOn {
            part: 0,
            note: 60,
            velocity: 0.8,
        });
        engine.handle_message(UiToEngine::NoteOn {
            part: 2,
            note: 60,
            velocity: 0.8,
        });

        let mut buffer = vec![0.0_f32; 1024];
        engine.process_block(&mut buffer, 2);

        let energy: f32 = buffer.iter().map(|s| s * s).sum();
        assert!(energy > 0.0);
    }
}
