// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Brume Lua API — the `brume` table exposed to scripts.

use std::sync::{Arc, Mutex};

use brume_app_protocol::UiToEngine;
use brume_common::ParameterId;
use crossbeam_channel::Sender;
use mlua::prelude::*;

/// A pending scheduled event from Lua.
pub struct PendingSchedule {
    pub target_beat: f64,
    pub callback: mlua::RegistryKey,
}

/// Shared state accessible from Lua callbacks.
/// A script-defined parameter exposed in the UI.
pub struct ScriptParam {
    pub name: String,
    pub label: String,
    pub min: f32,
    pub max: f32,
    pub value: f32,
}

pub struct BrumeApi {
    pub engine_tx: Sender<UiToEngine>,
    pub bpm: f32,
    pub beat_position: f64,
    pub playing: bool,
    pub pending_schedules: Vec<PendingSchedule>,
    pub script_params: Vec<ScriptParam>,
    pub params_changed: bool,
}

/// Registers the `brume` table on the Lua state.
pub fn register_api(lua: &Lua, api: Arc<Mutex<BrumeApi>>) -> LuaResult<()> {
    let brume = lua.create_table()?;

    // brume.set_param(part, name, value)
    let api_ref = api.clone();
    brume.set(
        "set_param",
        lua.create_function(move |_, (part, name, value): (u8, String, f32)| {
            if let Ok(id) = name.parse::<ParameterId>() {
                if let Ok(api) = api_ref.lock() {
                    let _ = api
                        .engine_tx
                        .try_send(UiToEngine::SetParameter { part, id, value });
                }
            }
            Ok(())
        })?,
    )?;

    // brume.note_on(part, note, velocity)
    let api_ref = api.clone();
    brume.set(
        "note_on",
        lua.create_function(move |_, (part, note, velocity): (u8, u8, f32)| {
            if let Ok(api) = api_ref.lock() {
                let _ = api.engine_tx.try_send(UiToEngine::NoteOn {
                    part,
                    note,
                    velocity,
                });
            }
            Ok(())
        })?,
    )?;

    // brume.note_off(part, note)
    let api_ref = api.clone();
    brume.set(
        "note_off",
        lua.create_function(move |_, (part, note): (u8, u8)| {
            if let Ok(api) = api_ref.lock() {
                let _ = api.engine_tx.try_send(UiToEngine::NoteOff { part, note });
            }
            Ok(())
        })?,
    )?;

    // brume.set_volume(value)
    let api_ref = api.clone();
    brume.set(
        "set_volume",
        lua.create_function(move |_, value: f32| {
            if let Ok(api) = api_ref.lock() {
                let _ = api.engine_tx.try_send(UiToEngine::SetMasterVolume(value));
            }
            Ok(())
        })?,
    )?;

    // brume.set_tempo(bpm)
    let api_ref = api.clone();
    brume.set(
        "set_tempo",
        lua.create_function(move |_, bpm: f32| {
            if let Ok(api) = api_ref.lock() {
                let _ = api.engine_tx.try_send(UiToEngine::SetTempo(bpm));
            }
            Ok(())
        })?,
    )?;

    // brume.set_part_level(part, level)
    let api_ref = api.clone();
    brume.set(
        "set_part_level",
        lua.create_function(move |_, (part, level): (u8, f32)| {
            if let Ok(api) = api_ref.lock() {
                let _ = api
                    .engine_tx
                    .try_send(UiToEngine::SetPartLevel { part, level });
            }
            Ok(())
        })?,
    )?;

    // brume.set_fx(slot, param, value)
    let api_ref = api.clone();
    brume.set(
        "set_fx",
        lua.create_function(move |_, (slot, param, value): (String, String, f32)| {
            if let Ok(api) = api_ref.lock() {
                let _ = api
                    .engine_tx
                    .try_send(UiToEngine::SetFxParam { slot, param, value });
            }
            Ok(())
        })?,
    )?;

    // brume.bpm() → number
    let api_ref = api.clone();
    brume.set(
        "bpm",
        lua.create_function(move |_, ()| {
            let bpm = api_ref.lock().map(|a| a.bpm).unwrap_or(120.0);
            Ok(bpm)
        })?,
    )?;

    // brume.beat() → number
    let api_ref = api.clone();
    brume.set(
        "beat",
        lua.create_function(move |_, ()| {
            let beat = api_ref.lock().map(|a| a.beat_position).unwrap_or(0.0);
            Ok(beat)
        })?,
    )?;

    // brume.all_notes_off(part)
    let api_ref = api.clone();
    brume.set(
        "all_notes_off",
        lua.create_function(move |_, part: u8| {
            if let Ok(api) = api_ref.lock() {
                let _ = api.engine_tx.try_send(UiToEngine::AllNotesOff { part });
            }
            Ok(())
        })?,
    )?;

    // brume.clear_modulation(part) — remove all modulation assignments on a part
    let api_ref = api.clone();
    brume.set(
        "clear_modulation",
        lua.create_function(move |_, part: u8| {
            if let Ok(api) = api_ref.lock() {
                // Send remove assignments for indices 31 down to 0 (safe even if fewer exist)
                for i in (0..32).rev() {
                    let _ = api
                        .engine_tx
                        .try_send(UiToEngine::RemoveAssignment { part, index: i });
                }
            }
            Ok(())
        })?,
    )?;

    // brume.add_param(name, label, min, max, default) — register a script parameter
    let api_ref = api.clone();
    brume.set(
        "add_param",
        lua.create_function(
            move |_, (name, label, min, max, default): (String, String, f32, f32, f32)| {
                if let Ok(mut api) = api_ref.lock() {
                    // Don't add duplicates
                    if !api.script_params.iter().any(|p| p.name == name) {
                        api.script_params.push(ScriptParam {
                            name,
                            label,
                            min,
                            max,
                            value: default,
                        });
                        api.params_changed = true;
                    }
                }
                Ok(())
            },
        )?,
    )?;

    // brume.get_param_value(name) — read current value of a script parameter
    let api_ref = api.clone();
    brume.set(
        "get_param_value",
        lua.create_function(move |_, name: String| {
            let val = api_ref
                .lock()
                .ok()
                .and_then(|api| {
                    api.script_params
                        .iter()
                        .find(|p| p.name == name)
                        .map(|p| p.value)
                })
                .unwrap_or(0.0);
            Ok(val)
        })?,
    )?;

    // brume.schedule(beat, fn) — call fn at an absolute beat position
    let api_ref = api.clone();
    brume.set(
        "schedule",
        lua.create_function(move |lua, (beat, func): (f64, LuaFunction)| {
            let key = lua.create_registry_value(func)?;
            if let Ok(mut api) = api_ref.lock() {
                api.pending_schedules.push(PendingSchedule {
                    target_beat: beat,
                    callback: key,
                });
            }
            Ok(())
        })?,
    )?;

    // brume.after(beats, fn) — call fn after N beats from now
    let api_ref = api.clone();
    brume.set(
        "after",
        lua.create_function(move |lua, (beats, func): (f64, LuaFunction)| {
            let key = lua.create_registry_value(func)?;
            if let Ok(mut api) = api_ref.lock() {
                let target = api.beat_position + beats;
                api.pending_schedules.push(PendingSchedule {
                    target_beat: target,
                    callback: key,
                });
            }
            Ok(())
        })?,
    )?;

    // brume.FM / brume.HARMONIC / brume.TIMBRAL / brume.GRANULAR — part constants
    brume.set("FM", 0u8)?;
    brume.set("HARMONIC", 1u8)?;
    brume.set("TIMBRAL", 2u8)?;
    brume.set("GRANULAR", 3u8)?;

    // brume.ALG — algorithm-name constants for the 6-op FM engine.
    // Values are the normalized 0..1 tap values the engine expects on
    // ParameterId::Algorithm (keeping brume.set_param working directly:
    //   brume.set_param(brume.FM, "Algorithm", brume.ALG.FAN_IN)).
    // Sourced from the engine's ALGORITHMS table — adding an algorithm
    // there extends the Lua surface automatically. Hyphens in names
    // (FAN-IN) become underscores so they're valid Lua identifiers.
    let alg = lua.create_table()?;
    let algs = brume_engine_runtime::ALGORITHMS;
    let max_idx = (algs.len() - 1) as f32;
    for (i, a) in algs.iter().enumerate() {
        let key = a.name.replace('-', "_");
        alg.set(key, i as f32 / max_idx)?;
    }
    brume.set("ALG", alg)?;

    // brume.set_fm_patch(part, { algorithm, ratios, levels, feedback, index })
    // Declarative setter for a full FM patch — expands to the ~15
    // SetParameter calls you'd otherwise write by hand. Every field is
    // optional; anything omitted leaves the current value alone.
    //
    // Fields:
    //   algorithm — normalized 0..1 (use brume.ALG.STACK etc.)
    //   ratios    — array of 1-6 per-op frequency ratios
    //   levels    — array of 1-6 per-op levels (FM modulation depth)
    //   feedback  — 0..1, FmFeedback
    //   index     — 0..10, global FmIndex
    //
    // Only meaningful on the FM part (0); still accepts any `part` so
    // a script mistakenly targeting HARMONIC etc. is a no-op, not an
    // error.
    let api_ref = api.clone();
    brume.set(
        "set_fm_patch",
        lua.create_function(move |_, (part, patch): (u8, LuaTable)| {
            let api = match api_ref.lock() {
                Ok(g) => g,
                Err(_) => return Ok(()),
            };
            let tx = &api.engine_tx;
            let send = |id: ParameterId, value: f32| {
                let _ = tx.try_send(UiToEngine::SetParameter { part, id, value });
            };

            if let Ok(v) = patch.get::<f32>("algorithm") {
                send(ParameterId::Algorithm, v);
            }
            if let Ok(v) = patch.get::<f32>("feedback") {
                send(ParameterId::FmFeedback, v);
            }
            if let Ok(v) = patch.get::<f32>("index") {
                send(ParameterId::FmIndex, v);
            }
            if let Ok(ratios) = patch.get::<LuaTable>("ratios") {
                let ids = [
                    ParameterId::Op1Ratio,
                    ParameterId::Op2Ratio,
                    ParameterId::Op3Ratio,
                    ParameterId::Op4Ratio,
                    ParameterId::Op5Ratio,
                    ParameterId::Op6Ratio,
                ];
                for (i, r) in ratios.sequence_values::<f32>().enumerate() {
                    if i >= 6 {
                        break;
                    }
                    if let Ok(v) = r {
                        send(ids[i], v);
                    }
                }
            }
            if let Ok(levels) = patch.get::<LuaTable>("levels") {
                let ids = [
                    ParameterId::Op1Level,
                    ParameterId::Op2Level,
                    ParameterId::Op3Level,
                    ParameterId::Op4Level,
                    ParameterId::Op5Level,
                    ParameterId::Op6Level,
                ];
                for (i, l) in levels.sequence_values::<f32>().enumerate() {
                    if i >= 6 {
                        break;
                    }
                    if let Ok(v) = l {
                        send(ids[i], v);
                    }
                }
            }
            Ok(())
        })?,
    )?;

    // brume.midi_to_freq(note) → Hz
    brume.set(
        "midi_to_freq",
        lua.create_function(|_, note: u8| {
            Ok(440.0_f32 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0))
        })?,
    )?;

    // brume.scale(root, intervals) → table of MIDI notes
    // e.g. brume.scale(60, {0,2,4,5,7,9,11}) for C major
    brume.set(
        "scale",
        lua.create_function(|lua, (root, intervals): (u8, LuaTable)| {
            let result = lua.create_table()?;
            for (i, interval) in intervals.sequence_values::<u8>().enumerate() {
                if let Ok(iv) = interval {
                    result.set(i + 1, root + iv)?;
                }
            }
            Ok(result)
        })?,
    )?;

    // brume.chord(root, type) → table of MIDI notes
    // Types: "maj", "min", "maj7", "min7", "dom7", "dim", "aug", "sus2", "sus4"
    brume.set(
        "chord",
        lua.create_function(|lua, (root, chord_type): (u8, String)| {
            let intervals: &[u8] = match chord_type.as_str() {
                "maj" => &[0, 4, 7],
                "min" => &[0, 3, 7],
                "maj7" => &[0, 4, 7, 11],
                "min7" => &[0, 3, 7, 10],
                "dom7" | "7" => &[0, 4, 7, 10],
                "dim" => &[0, 3, 6],
                "aug" => &[0, 4, 8],
                "sus2" => &[0, 2, 7],
                "sus4" => &[0, 5, 7],
                "9" => &[0, 4, 7, 10, 14],
                _ => &[0, 4, 7],
            };
            let result = lua.create_table()?;
            for (i, &iv) in intervals.iter().enumerate() {
                result.set(i + 1, root + iv)?;
            }
            Ok(result)
        })?,
    )?;

    // Note name constants
    let notes = lua.create_table()?;
    let names = [
        "C", "Cs", "D", "Ds", "E", "F", "Fs", "G", "Gs", "A", "As", "B",
    ];
    for octave in 0..=8 {
        for (i, name) in names.iter().enumerate() {
            let midi = (octave + 1) * 12 + i as u8; // C1=24, A4=69
            if midi <= 127 {
                notes.set(format!("{name}{octave}"), midi)?;
            }
        }
    }
    lua.globals().set("note", notes)?;

    lua.globals().set("brume", brume)?;

    // Wall clock time function for clock.sleep()
    let start_time = std::time::Instant::now();
    lua.globals().set(
        "_clock_time",
        lua.create_function(move |_, ()| Ok(start_time.elapsed().as_secs_f64()))?,
    )?;

    // Load the clock module
    let clock_src = include_str!("clock.lua");
    lua.load(clock_src).set_name("clock").exec()?;

    // Load the screen module
    let screen_src = include_str!("screen.lua");
    lua.load(screen_src).set_name("screen").exec()?;

    // Override print() to collect output for the UI console
    let print_buf: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let pb = print_buf.clone();
    lua.globals().set(
        "print",
        lua.create_function(move |_, args: mlua::Variadic<LuaValue>| {
            let parts: Vec<String> = args
                .iter()
                .map(|v| match v {
                    LuaValue::Nil => "nil".to_string(),
                    LuaValue::Boolean(b) => b.to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    LuaValue::Number(n) => format!("{n}"),
                    LuaValue::String(s) => s.to_string_lossy().to_string(),
                    other => format!("{other:?}"),
                })
                .collect();
            let line = parts.join("\t");
            eprintln!("lua: {line}");
            if let Ok(mut buf) = pb.lock() {
                buf.push(line);
            }
            Ok(())
        })?,
    )?;

    // Store print buffer on the Lua registry for retrieval
    lua.set_named_registry_value("print_buffer", lua.create_any_userdata(print_buf)?)?;

    Ok(())
}
