// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Script engine — manages the Lua VM and script lifecycle.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brume_app_protocol::UiToEngine;
use crossbeam_channel::Sender;
use mlua::prelude::*;

use crate::api::{self, BrumeApi};
use crate::sandbox::{self, ScriptBudget};

/// Time budget for `init()` calls. Loosened relative to the per-tick
/// default because legitimate scripts may load helper modules, build
/// wavetables, or run setup work; 2 s is still well shorter than any
/// human-noticeable hang.
const INIT_BUDGET: Duration = Duration::from_secs(2);

/// The Brume scripting engine.
///
/// Manages a Lua 5.5 VM with the Brume API exposed. Scripts can be
/// loaded from disk and define callback functions that are invoked
/// at appropriate moments (init, beat, note, CC).
/// A queued event for script processing. Wraps both note/CC traffic
/// (from MIDI input or the engine's own routing) and transport
/// state-change signals (start/stop/continue), all of which scripts
/// receive as named hook callbacks: `on_note`, `on_cc`, `on_start`,
/// `on_stop`, `on_continue`.
#[derive(Clone)]
pub enum ScriptMidiEvent {
    NoteOn { part: u8, note: u8, velocity: f32 },
    NoteOff { part: u8, note: u8 },
    Cc { part: u8, cc: u8, value: f32 },
    TransportStart,
    TransportStop,
    TransportContinue,
}

/// A scheduled event — fires when beat position passes the target.
struct ScheduledEvent {
    target_beat: f64,
    callback: mlua::RegistryKey,
}

pub struct ScriptEngine {
    lua: Lua,
    budget: ScriptBudget,
    api: Arc<Mutex<BrumeApi>>,
    scripts_dir: PathBuf,
    loaded_script: Option<String>,
    midi_queue: Vec<ScriptMidiEvent>,
    scheduled: Vec<ScheduledEvent>,
    errors: Vec<String>,
    last_modified: Option<std::time::SystemTime>,
}

impl ScriptEngine {
    /// Creates a new script engine.
    ///
    /// The `engine_tx` channel is used to send messages to the audio engine.
    /// Scripts loaded by this engine can control parameters, send notes, etc.
    pub fn new(engine_tx: Sender<UiToEngine>, scripts_dir: &Path) -> Result<Self, ScriptError> {
        Self::with_sample_rate(engine_tx, scripts_dir, 48000.0)
    }

    /// Creates a new script engine with a specified sample rate for DSP bindings.
    pub fn with_sample_rate(
        engine_tx: Sender<UiToEngine>,
        scripts_dir: &Path,
        sample_rate: f32,
    ) -> Result<Self, ScriptError> {
        // The VM is built with a restricted stdlib, a 32 MB memory cap,
        // and a time-budget hook installed (see `sandbox` module).
        // Until we arm the budget the hook is a no-op, so registering
        // bindings and loading the trusted clock/screen modules below
        // runs unbounded — only user-script entry points get budgeted.
        let (lua, budget) =
            sandbox::build_sandboxed_lua().map_err(|e| ScriptError::Init(e.to_string()))?;

        let api_state = Arc::new(Mutex::new(BrumeApi {
            engine_tx,
            bpm: 120.0,
            beat_position: 0.0,
            playing: true,
            pending_schedules: Vec::new(),
            script_params: Vec::new(),
            params_changed: false,
        }));

        api::register_api(&lua, api_state.clone()).map_err(|e| ScriptError::Init(e.to_string()))?;

        crate::dsp_bindings::register_dsp(&lua, sample_rate)
            .map_err(|e| ScriptError::Init(e.to_string()))?;

        // Add the scripts directory to Lua's package.path
        let path_str = scripts_dir.to_string_lossy();
        lua.load(format!(
            "package.path = '{path_str}/?.lua;' .. package.path"
        ))
        .exec()
        .map_err(|e| ScriptError::Init(e.to_string()))?;

        Ok(Self {
            lua,
            budget,
            api: api_state,
            scripts_dir: scripts_dir.to_path_buf(),
            loaded_script: None,
            midi_queue: Vec::new(),
            scheduled: Vec::new(),
            errors: Vec::new(),
            last_modified: None,
        })
    }

    /// Returns the scripts directory path.
    #[must_use]
    pub fn scripts_dir(&self) -> &std::path::Path {
        &self.scripts_dir
    }

    /// Lists available script files in the scripts directory, including subdirectories.
    /// Subdirectory scripts are prefixed: "studies/study-1-sound".
    #[must_use]
    pub fn list_scripts(&self) -> Vec<String> {
        let mut scripts = Vec::new();
        self.scan_dir(&self.scripts_dir.clone(), "", &mut scripts);
        scripts.sort();
        scripts
    }

    /// Lists FX scripts (the `fx/` subdirectory of `scripts_dir`).
    /// Returns stems (no `.lua` extension), sorted alphabetically.
    /// FX scripts are kept in their own subdirectory so the SCRIPT
    /// page tree (which calls `list_scripts`) doesn't mix them in
    /// with control scripts — the two have different lifecycles
    /// and load through different code paths. The MIX page's LUA
    /// tab is the canonical surface for these.
    #[must_use]
    pub fn list_fx_scripts(&self) -> Vec<String> {
        let fx_dir = self.scripts_dir.join("fx");
        let Ok(entries) = std::fs::read_dir(&fx_dir) else {
            return Vec::new();
        };
        let mut scripts: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "lua") {
                    path.file_stem().map(|s| s.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect();
        scripts.sort();
        scripts
    }

    /// Returns the absolute path to a named FX script, suitable for
    /// `LuaFxSlot::from_file`. Validates the name against the same
    /// safe-name rule the control-script loader uses, so subdir
    /// traversal can't reach into other parts of the user's home.
    pub fn fx_script_path(&self, name: &str) -> Result<PathBuf, ScriptError> {
        if !is_safe_script_name(name) {
            return Err(ScriptError::InvalidName(name.to_string()));
        }
        Ok(self.scripts_dir.join("fx").join(format!("{name}.lua")))
    }

    fn scan_dir(&self, dir: &std::path::Path, prefix: &str, scripts: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Skip the fx/ directory (handled separately)
                    if let Some(name) = path.file_name() {
                        let dir_name = name.to_string_lossy();
                        if dir_name != "fx" {
                            let sub_prefix = if prefix.is_empty() {
                                dir_name.to_string()
                            } else {
                                format!("{prefix}/{dir_name}")
                            };
                            self.scan_dir(&path, &sub_prefix, scripts);
                        }
                    }
                } else if path.extension().is_some_and(|e| e == "lua") {
                    if let Some(stem) = path.file_stem() {
                        let name = if prefix.is_empty() {
                            stem.to_string_lossy().into_owned()
                        } else {
                            format!("{prefix}/{}", stem.to_string_lossy())
                        };
                        scripts.push(name);
                    }
                }
            }
        }
    }

    /// Loads and executes a script file by name (without .lua extension).
    ///
    /// Unloads any currently loaded script first, then calls `init()`.
    /// `name` is validated by [`is_safe_script_name`] before any path
    /// join; everything downstream (hot-reload, command file, brumectl)
    /// flows through this entry point, so a single guard here covers
    /// the whole script-name surface.
    pub fn load_script(&mut self, name: &str) -> Result<(), ScriptError> {
        if !is_safe_script_name(name) {
            return Err(ScriptError::InvalidName(name.to_string()));
        }
        if self.loaded_script.is_some() {
            self.unload();
        }
        let path = self.scripts_dir.join(format!("{name}.lua"));
        let source = std::fs::read_to_string(&path)
            .map_err(|e| ScriptError::Load(path.display().to_string(), e.to_string()))?;

        // Top-level script body and `init()` both run user code.
        // `init` is allowed a longer budget because helper-loading and
        // wavetable setup are legitimately heavier than a per-tick tick.
        {
            let _g = self.budget.arm_with(INIT_BUDGET);
            self.lua
                .load(&source)
                .set_name(name)
                .exec()
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;
        }

        self.loaded_script = Some(name.to_string());

        // Track file modification time for hot-reload
        self.last_modified = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());

        // Call init() if defined.
        {
            let _g = self.budget.arm_with(INIT_BUDGET);
            self.call_hook_no_arm("init", ())?;
        }

        Ok(())
    }

    /// Unloads the current script by clearing global callbacks and scheduled events.
    pub fn unload(&mut self) {
        // Cancel clock coroutines + run cleanup. Both touch user code,
        // so they're budgeted; failure is logged via the same `errors`
        // channel rather than propagating.
        {
            let _g = self.budget.arm();
            if let Ok(clock) = self.lua.globals().get::<LuaTable>("clock") {
                if let Ok(cancel_fn) = clock.get::<LuaFunction>("cancel_all") {
                    let _ = cancel_fn.call::<()>(());
                }
            }
        }

        // Clear script params
        if let Ok(mut api) = self.api.lock() {
            api.script_params.clear();
            api.params_changed = true;
        }

        // Call cleanup if defined
        let _ = self.call_hook("cleanup", ());

        let globals = self.lua.globals();
        for name in &[
            "init",
            "on_beat",
            "on_note",
            "on_cc",
            "on_tick",
            "on_start",
            "on_stop",
            "on_continue",
            "cleanup",
        ] {
            let _ = globals.set(*name, LuaNil);
        }

        // Clear scheduled events
        for event in self.scheduled.drain(..) {
            let _ = self.lua.remove_registry_value(event.callback);
        }

        self.loaded_script = None;
    }

    /// Returns the name of the currently loaded script, if any.
    #[must_use]
    pub fn loaded_script(&self) -> Option<&str> {
        self.loaded_script.as_deref()
    }

    /// Updates the transport state visible to Lua (bpm, beat position).
    /// Call this periodically from the main thread.
    pub fn update_transport(&self, bpm: f32, beat_position: f64) {
        if let Ok(mut api) = self.api.lock() {
            api.bpm = bpm;
            api.beat_position = beat_position;
        }
    }

    /// Returns whether the transport is playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.api.lock().map(|a| a.playing).unwrap_or(true)
    }

    /// Set transport playing state.
    pub fn set_playing(&self, playing: bool) {
        if let Ok(mut api) = self.api.lock() {
            api.playing = playing;
        }
    }

    /// Queues a MIDI event for processing on the next tick.
    pub fn queue_midi(&mut self, event: ScriptMidiEvent) {
        self.midi_queue.push(event);
    }

    /// Processes all queued events, dispatching to the matching script
    /// hook (`on_note`, `on_cc`, `on_start`, `on_stop`, `on_continue`).
    /// Transport hooks are parameterless — scripts that care about the
    /// distinction define separate `on_start` / `on_stop` / `on_continue`
    /// functions; the engine doesn't try to fan a single `on_transport`
    /// hook with a discriminator argument because that's harder to type
    /// correctly from a hot-reload reload mid-arrangement.
    pub fn process_midi_queue(&mut self) {
        let events: Vec<ScriptMidiEvent> = self.midi_queue.drain(..).collect();
        for event in events {
            match event {
                ScriptMidiEvent::NoteOn {
                    part,
                    note,
                    velocity,
                } => {
                    self.call_hook_safe("on_note", (part, note, velocity));
                }
                ScriptMidiEvent::NoteOff { part, note } => {
                    self.call_hook_safe("on_note", (part, note, 0.0_f32));
                }
                ScriptMidiEvent::Cc { part, cc, value } => {
                    self.call_hook_safe("on_cc", (part, cc, value));
                }
                ScriptMidiEvent::TransportStart => {
                    self.call_hook_safe("on_start", ());
                }
                ScriptMidiEvent::TransportStop => {
                    self.call_hook_safe("on_stop", ());
                }
                ScriptMidiEvent::TransportContinue => {
                    self.call_hook_safe("on_continue", ());
                }
            }
        }
    }

    /// Called on each beat boundary. Invokes the script's `on_beat(beat)`.
    pub fn on_beat(&mut self, beat: f64) {
        self.call_hook_safe("on_beat", beat);
    }

    /// Called on each transport tick (~50ms). Invokes `on_tick(beat)`.
    /// Also processes any scheduled events whose target beat has passed.
    pub fn on_tick(&mut self, beat: f64) {
        // Process scheduled events
        if let Err(e) = self.process_scheduled(beat) {
            let msg = e.to_string();
            eprintln!("brume script: {msg}");
            self.errors.push(msg);
        }

        // Drain any new schedules from the API into our list
        if let Ok(mut api) = self.api.lock() {
            for pending in api.pending_schedules.drain(..) {
                self.scheduled.push(ScheduledEvent {
                    target_beat: pending.target_beat,
                    callback: pending.callback,
                });
            }
        }

        // Resume clock coroutines. The coroutine bodies are user code,
        // so the call is budgeted.
        if let Ok(clock) = self.lua.globals().get::<LuaTable>("clock") {
            if let Ok(tick_fn) = clock.get::<LuaFunction>("_tick") {
                let _g = self.budget.arm();
                if let Err(e) = tick_fn.call::<()>(beat) {
                    self.errors.push(format!("clock._tick: {e}"));
                }
            }
        }

        self.call_hook_safe("on_tick", beat);
    }

    /// Fires and removes any scheduled events whose beat has arrived.
    fn process_scheduled(&mut self, current_beat: f64) -> Result<(), ScriptError> {
        let mut i = 0;
        while i < self.scheduled.len() {
            if current_beat >= self.scheduled[i].target_beat {
                let event = self.scheduled.remove(i);
                if let Ok(func) = self.lua.registry_value::<LuaFunction>(&event.callback) {
                    let _g = self.budget.arm();
                    func.call::<()>(current_beat)
                        .map_err(|e| ScriptError::Runtime(format!("scheduled: {e}")))?;
                }
                self.lua
                    .remove_registry_value(event.callback)
                    .map_err(|e| ScriptError::Runtime(e.to_string()))?;
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// Called when a MIDI note is received. Invokes `on_note(part, note, velocity)`.
    /// Velocity of 0 indicates note-off.
    pub fn on_note(&self, part: u8, note: u8, velocity: f32) -> Result<(), ScriptError> {
        self.call_hook("on_note", (part, note, velocity))
    }

    /// Called when a MIDI CC is received. Invokes `on_cc(part, cc, value)`.
    pub fn on_cc(&self, part: u8, cc: u8, value: f32) -> Result<(), ScriptError> {
        self.call_hook("on_cc", (part, cc, value))
    }

    /// Calls a named Lua function if it exists, with the given
    /// arguments. The default per-call time budget is armed for the
    /// duration of the call, so a misbehaving callback can't wedge the
    /// engine indefinitely.
    fn call_hook<A: IntoLuaMulti>(&self, name: &str, args: A) -> Result<(), ScriptError> {
        let globals = self.lua.globals();
        if let Ok(func) = globals.get::<LuaFunction>(name) {
            let _g = self.budget.arm();
            func.call::<()>(args)
                .map_err(|e| ScriptError::Runtime(format!("{name}: {e}")))?;
        }
        Ok(())
    }

    /// Same as `call_hook` but assumes the caller has already armed
    /// the budget (e.g. with [`INIT_BUDGET`] for `init`). Avoids the
    /// arm-twice clobber where the inner default budget would
    /// overwrite the caller's longer one.
    fn call_hook_no_arm<A: IntoLuaMulti>(&self, name: &str, args: A) -> Result<(), ScriptError> {
        let globals = self.lua.globals();
        if let Ok(func) = globals.get::<LuaFunction>(name) {
            func.call::<()>(args)
                .map_err(|e| ScriptError::Runtime(format!("{name}: {e}")))?;
        }
        Ok(())
    }

    /// Poll for a `<scripts_dir>/.cmd` file dropped by an external tool
    /// (e.g. `brumectl scripts load`). The file is one line of the form
    ///
    ///   load <name>
    ///   unload
    ///
    /// On any command, write a one-line result to `<scripts_dir>/.cmd.result`
    /// (`ok` or `err <message>`) and delete the command file. The result
    /// file is written atomically via a .tmp-rename so brumectl can't
    /// observe half-written contents.
    ///
    /// Line-oriented text rather than JSON so the scripting crate
    /// doesn't pick up a serde dependency just for this channel.
    /// Returns true if a command was processed (whether ok or err).
    pub fn check_command_file(&mut self) -> bool {
        let cmd_path = self.scripts_dir.join(".cmd");
        let Ok(contents) = std::fs::read_to_string(&cmd_path) else {
            return false;
        };

        let mut parts = contents.split_whitespace();
        let action = parts.next().unwrap_or("");
        let name = parts.next();

        let result = match action {
            "load" => match name {
                None => "err load requires a script name".to_string(),
                Some(n) => {
                    if self.loaded_script.is_some() {
                        self.unload();
                    }
                    match self.load_script(n) {
                        Ok(()) => "ok".to_string(),
                        Err(e) => format!("err {e}"),
                    }
                }
            },
            "unload" => {
                if self.loaded_script.is_some() {
                    self.unload();
                }
                "ok".to_string()
            }
            "" => "err empty command".to_string(),
            other => format!("err unknown action: {other}"),
        };

        Self::write_cmd_result(&self.scripts_dir, &result);
        // Remove the command file regardless of success — a stuck .cmd
        // would otherwise re-run on every tick.
        let _ = std::fs::remove_file(&cmd_path);
        true
    }

    fn write_cmd_result(dir: &Path, msg: &str) {
        let result_path = dir.join(".cmd.result");
        let tmp_path = dir.join(".cmd.result.tmp");
        if std::fs::write(&tmp_path, msg).is_ok() {
            let _ = std::fs::rename(&tmp_path, &result_path);
        }
    }

    /// Checks if the loaded script file has been modified on disk.
    /// If so, reloads it automatically. Returns true if a reload happened.
    pub fn check_hot_reload(&mut self) -> bool {
        let name = match &self.loaded_script {
            Some(n) => n.clone(),
            None => return false,
        };

        let path = self.scripts_dir.join(format!("{name}.lua"));
        let current_modified = match std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
        {
            Some(t) => t,
            None => return false,
        };

        let changed = match self.last_modified {
            Some(prev) => current_modified > prev,
            None => false,
        };

        if changed {
            eprintln!("brume script: hot-reloading {name}.lua");
            self.errors.push(format!("[hot-reload: {name}.lua]"));
            match self.load_script(&name) {
                Ok(()) => true,
                Err(e) => {
                    self.errors.push(format!("reload error: {e}"));
                    false
                }
            }
        } else {
            false
        }
    }

    /// Returns script-defined parameters if they've changed since last check.
    pub fn drain_params(&mut self) -> Option<Vec<(String, String, f32, f32, f32)>> {
        if let Ok(mut api) = self.api.lock() {
            if api.params_changed {
                api.params_changed = false;
                return Some(
                    api.script_params
                        .iter()
                        .map(|p| (p.name.clone(), p.label.clone(), p.min, p.max, p.value))
                        .collect(),
                );
            }
        }
        None
    }

    /// Sets a script parameter value (called from UI slider).
    pub fn set_script_param(&self, name: &str, value: f32) {
        if let Ok(mut api) = self.api.lock() {
            if let Some(p) = api.script_params.iter_mut().find(|p| p.name == name) {
                p.value = value;
            }
        }
    }

    /// Drains screen drawing commands from the Lua screen module.
    #[must_use]
    pub fn drain_screen(&self) -> Option<String> {
        if let Ok(screen) = self.lua.globals().get::<LuaTable>("screen") {
            if let Ok(drain_fn) = screen.get::<LuaFunction>("_drain") {
                // `_drain` is defined by us in screen.lua, but a user
                // script could monkey-patch it. Arm the budget so that
                // an injected infinite loop can't wedge the UI tick.
                let _g = self.budget.arm();
                if let Ok(result) = drain_fn.call::<Option<String>>(()) {
                    return result;
                }
            }
        }
        None
    }

    /// Returns true if the screen module has been activated (any draw call made).
    #[must_use]
    pub fn screen_active(&self) -> bool {
        self.lua
            .globals()
            .get::<LuaTable>("screen")
            .and_then(|s| s.get::<bool>("_active"))
            .unwrap_or(false)
    }

    /// Drains any pending error messages from script callbacks.
    #[must_use]
    pub fn drain_errors(&mut self) -> Vec<String> {
        self.errors.drain(..).collect()
    }

    /// Runs a callback hook, capturing errors instead of propagating.
    fn call_hook_safe(&mut self, name: &str, args: impl IntoLuaMulti + Clone) {
        if let Err(e) = self.call_hook(name, args) {
            let msg = e.to_string();
            eprintln!("brume script: {msg}");
            self.errors.push(msg);
        }
    }

    /// Drains any pending print() output from scripts.
    /// Returns lines that were printed since the last drain.
    #[must_use]
    pub fn drain_print_output(&self) -> Vec<String> {
        if let Ok(ud) = self
            .lua
            .named_registry_value::<mlua::AnyUserData>("print_buffer")
        {
            if let Ok(buf) = ud.borrow::<Arc<Mutex<Vec<String>>>>() {
                if let Ok(mut lines) = buf.lock() {
                    return lines.drain(..).collect();
                }
            }
        }
        Vec::new()
    }

    /// Executes a Lua string directly (for REPL/console use).
    pub fn eval(&self, code: &str) -> Result<String, ScriptError> {
        let _g = self.budget.arm();
        let result: LuaValue = self
            .lua
            .load(code)
            .eval()
            .map_err(|e| ScriptError::Runtime(e.to_string()))?;

        Ok(format_lua_value(&result))
    }
}

fn format_lua_value(value: &LuaValue) -> String {
    match value {
        LuaValue::Nil => "nil".to_string(),
        LuaValue::Boolean(b) => b.to_string(),
        LuaValue::Integer(i) => i.to_string(),
        LuaValue::Number(n) => format!("{n:.4}"),
        LuaValue::String(s) => s.to_string_lossy().to_string(),
        _ => format!("{value:?}"),
    }
}

/// Errors from the scripting engine.
#[derive(Debug)]
pub enum ScriptError {
    Init(String),
    Load(String, String),
    Runtime(String),
    InvalidName(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init(e) => write!(f, "script init: {e}"),
            Self::Load(path, e) => write!(f, "script load {path}: {e}"),
            Self::Runtime(e) => write!(f, "script error: {e}"),
            Self::InvalidName(name) => write!(
                f,
                "invalid script name {name:?}: each '/'-separated segment must be ASCII alphanumerics, '-', or '_' (non-empty, ≤64 chars)"
            ),
        }
    }
}

impl std::error::Error for ScriptError {}

/// Returns true if `name` is safe to use as the stem of a script
/// filename, supporting subdirectory layout via `/` separators.
///
/// Each `/`-separated segment must be:
/// - non-empty (rejects leading/trailing/repeated `/`)
/// - ≤64 chars
/// - ASCII alphanumeric, `-`, or `_` only
///
/// And the whole name is capped at 256 chars (typical filesystem
/// path-component budget; enough for `subdir/sub/sub/script_name`).
///
/// This intentionally rejects:
/// - path traversal: `.` is excluded per-char, so `..` and `.foo`
///   never match; `\` is excluded so Windows-style traversal
///   doesn't slip through; absolute paths fail because a leading
///   `/` produces an empty first segment.
/// - shell metacharacters (`;`, `|`, `&`, `$`, backtick, quote,
///   space): excluded per-char so a name reaching `format!`'d
///   shell paths downstream in brumectl can't inject commands.
/// - bare extensions: `.` excluded prevents `name.lua.bak`-style
///   confusion since the loader always appends `.lua` itself.
/// - non-ASCII (visually-confusable Cyrillic, RTL, NFD/NFC variants).
///
/// Subdirectory support is restored after a brief regression: the
/// initial guard (which only allowed flat names) broke `list_scripts`
/// callers that show subdirectory-organized scripts in the UI tree,
/// since `scan_dir` returns names like `studies/01_metronome`.
///
/// Mirrors `patch-store::is_safe_name` in spirit but tighter — script
/// names land in stack traces, command files, and remote shell paths;
/// patch names only land in JSON filenames.
fn is_safe_script_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 256 {
        return false;
    }
    name.split('/').all(|segment| {
        !segment.is_empty()
            && segment.len() <= 64
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    fn test_engine() -> ScriptEngine {
        let (tx, _rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-scripts");
        let _ = std::fs::create_dir_all(&dir);
        ScriptEngine::new(tx, &dir).unwrap()
    }

    #[test]
    fn eval_basic() {
        let engine = test_engine();
        assert_eq!(engine.eval("1 + 2").unwrap(), "3");
        assert_eq!(engine.eval("'hello'").unwrap(), "hello");
    }

    #[test]
    fn load_script_rejects_unsafe_names() {
        let mut engine = test_engine();
        // Regression test for the path-traversal class on the
        // `scripts_dir.join(format!("{name}.lua"))` call. None of
        // these names should reach the filesystem at all — the
        // guard must catch them first and return InvalidName.
        let bad_names = [
            "",
            "../etc/passwd",
            "..",
            "studies/..", // segment-level traversal — '..' has '.' which is excluded
            "studies/../leak", // mid-path traversal
            "/abs/path",  // absolute path → empty first segment
            "studies//double", // empty middle segment
            "trailing/",  // empty trailing segment
            r"foo\bar",
            "name with spaces",
            "name;with;semicolons",
            "name`with`backticks",
            "name$(injection)",
            "name.with.dots",
            "name|pipe",
            "café", // non-ASCII alphanumeric
        ];
        for bad in bad_names {
            match engine.load_script(bad) {
                Err(ScriptError::InvalidName(_)) => {}
                other => panic!("expected InvalidName for {bad:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn is_safe_script_name_accepts_typical_names() {
        for ok in [
            "diffuser",
            "study_01_metronome",
            "clock-driver",
            "FX42",
            "_underscore_lead",
            // Subdirectory layouts — what list_scripts returns for
            // scripts organized under ~/brume/scripts/<subfolder>/.
            "studies/01_metronome",
            "user_scripts/drone",
            "deep/nested/path",
        ] {
            assert!(is_safe_script_name(ok), "{ok:?} should be safe");
        }
    }

    #[test]
    fn brume_api_accessible() {
        let engine = test_engine();
        assert_eq!(engine.eval("brume.FM").unwrap(), "0");
        assert_eq!(engine.eval("brume.HARMONIC").unwrap(), "1");
        assert_eq!(engine.eval("brume.TIMBRAL").unwrap(), "2");
        assert_eq!(engine.eval("brume.GRANULAR").unwrap(), "3");
    }

    #[test]
    fn alg_constants_are_normalized() {
        let engine = test_engine();
        let parse = |expr: &str| engine.eval(expr).unwrap().parse::<f32>().unwrap();
        // Endpoints — STACK is 0.0, CHAIN is 1.0.
        assert!((parse("brume.ALG.STACK") - 0.0).abs() < 1e-4);
        assert!((parse("brume.ALG.CHAIN") - 1.0).abs() < 1e-4);
        // Mid (FAN_IN is index 4 of 12 → 4/11).
        assert!((parse("brume.ALG.FAN_IN") - 4.0 / 11.0).abs() < 1e-4);
    }

    #[test]
    fn set_fm_patch_sends_expected_messages() {
        use brume_common::ParameterId;
        let (tx, rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-set-fm-patch");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("t.lua"),
            r#"
            function init()
              brume.set_fm_patch(brume.FM, {
                algorithm = brume.ALG.FAN_IN,
                ratios    = {1.0, 2.0, 3.0, 4.0, 5.0, 6.0},
                levels    = {0.6, 0.5, 0.4, 0.3, 0.2, 0.1},
                feedback  = 0.3,
                index     = 2.5,
              })
            end
        "#,
        )
        .unwrap();

        let mut engine = ScriptEngine::new(tx, &dir).unwrap();
        engine.load_script("t").unwrap();

        // Drain every SetParameter → (id, value) so we can assert the
        // full patch landed; expecting 15 messages (1 alg + 1 fb + 1 idx
        // + 6 ratios + 6 levels).
        let mut seen: Vec<(ParameterId, f32)> = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let brume_app_protocol::UiToEngine::SetParameter { id, value, .. } = msg {
                seen.push((id, value));
            }
        }
        assert_eq!(
            seen.len(),
            15,
            "expected 15 SetParameter messages, got {} — {seen:?}",
            seen.len()
        );

        let find = |id: ParameterId| seen.iter().find(|(i, _)| *i == id).map(|(_, v)| *v);
        assert!((find(ParameterId::Algorithm).unwrap() - 4.0 / 11.0).abs() < 1e-4);
        assert_eq!(find(ParameterId::FmFeedback), Some(0.3));
        assert_eq!(find(ParameterId::FmIndex), Some(2.5));
        assert_eq!(find(ParameterId::Op1Ratio), Some(1.0));
        assert_eq!(find(ParameterId::Op6Ratio), Some(6.0));
        assert_eq!(find(ParameterId::Op1Level), Some(0.6));
        assert_eq!(find(ParameterId::Op6Level), Some(0.1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bpm_query() {
        let engine = test_engine();
        engine.update_transport(140.0, 0.0);
        let result = engine.eval("brume.bpm()").unwrap();
        assert!(result.contains("140"), "bpm should be 140: got {result}");
    }

    #[test]
    fn load_and_call_init() {
        let (tx, rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-init");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("test.lua"),
            r#"
            function init()
                brume.set_param(brume.FM, "FmIndex", 5.0)
            end
        "#,
        )
        .unwrap();

        let mut engine = ScriptEngine::new(tx, &dir).unwrap();
        engine.load_script("test").unwrap();

        // init() should have sent a SetParameter message
        let msg = rx.try_recv();
        assert!(msg.is_ok(), "init should have sent a message");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn on_beat_callback() {
        let (tx, rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-beat");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("beat.lua"),
            r#"
            function on_beat(beat)
                if beat >= 4.0 then
                    brume.note_on(brume.TIMBRAL, 60, 0.5)
                end
            end
        "#,
        )
        .unwrap();

        let mut engine = ScriptEngine::new(tx, &dir).unwrap();
        engine.load_script("beat").unwrap();

        // Beat 2: should not trigger
        engine.on_beat(2.0);
        assert!(rx.try_recv().is_err());

        // Beat 4: should trigger note_on
        engine.on_beat(4.0);
        assert!(rx.try_recv().is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_midi_queue_dispatches_transport_hooks() {
        // The queue accepts TransportStart / TransportStop /
        // TransportContinue alongside the note/CC traffic; each
        // dispatches to its own parameterless hook in the loaded
        // script. The script proves which hook fired by sending a
        // distinct parameter value through the engine_tx channel.
        let (tx, rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-transport");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("transport.lua"),
            r#"
            function on_start()    brume.set_param(brume.FM, "FmIndex", 1.0) end
            function on_stop()     brume.set_param(brume.FM, "FmIndex", 2.0) end
            function on_continue() brume.set_param(brume.FM, "FmIndex", 3.0) end
        "#,
        )
        .unwrap();

        let mut engine = ScriptEngine::new(tx, &dir).unwrap();
        engine.load_script("transport").unwrap();

        // Drain any messages from load (init() can publish).
        while rx.try_recv().is_ok() {}

        engine.queue_midi(ScriptMidiEvent::TransportStart);
        engine.queue_midi(ScriptMidiEvent::TransportStop);
        engine.queue_midi(ScriptMidiEvent::TransportContinue);
        engine.process_midi_queue();

        // Three SetParameter messages, one per hook firing, in order.
        let mut values = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let UiToEngine::SetParameter { value, .. } = msg {
                values.push(value);
            }
        }
        assert_eq!(values, vec![1.0, 2.0, 3.0], "got {values:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_scripts() {
        let dir = std::env::temp_dir().join("brume-test-list");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("alpha.lua"), "").unwrap();
        std::fs::write(dir.join("beta.lua"), "").unwrap();
        std::fs::write(dir.join("notlua.txt"), "").unwrap();

        let engine = test_engine();
        // Use a new engine with the right dir
        let (tx, _) = bounded(256);
        let eng = ScriptEngine::new(tx, &dir).unwrap();
        let scripts = eng.list_scripts();
        assert_eq!(scripts, vec!["alpha", "beta"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chord_and_scale_helpers() {
        let engine = test_engine();
        // C major chord
        let result = engine
            .eval("table.concat(brume.chord(60, 'maj'), ',')")
            .unwrap();
        assert_eq!(result, "60,64,67");

        // C minor 7
        let result = engine
            .eval("table.concat(brume.chord(60, 'min7'), ',')")
            .unwrap();
        assert_eq!(result, "60,63,67,70");

        // Note constants
        let result = engine.eval("note.A4").unwrap();
        assert_eq!(result, "69");
    }

    #[test]
    fn schedule_after() {
        let (tx, rx) = bounded(256);
        let dir = std::env::temp_dir().join("brume-test-schedule");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("sched.lua"),
            r#"
            function init()
                brume.after(2.0, function(beat)
                    brume.note_on(brume.FM, 72, 0.5)
                end)
            end
        "#,
        )
        .unwrap();

        let mut engine = ScriptEngine::new(tx, &dir).unwrap();
        engine.load_script("sched").unwrap();

        // Tick at beat 1: too early
        engine.update_transport(120.0, 1.0);
        engine.on_tick(1.0);
        assert!(rx.try_recv().is_err(), "should not fire at beat 1");

        // Tick at beat 2.5: past the target (init was at beat 0, after 2 = beat 2)
        engine.update_transport(120.0, 2.5);
        engine.on_tick(2.5);
        assert!(rx.try_recv().is_ok(), "should fire at beat 2.5");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn midi_to_freq() {
        let engine = test_engine();
        let result = engine
            .eval("string.format('%.1f', brume.midi_to_freq(69))")
            .unwrap();
        assert_eq!(result, "440.0");
    }
}
