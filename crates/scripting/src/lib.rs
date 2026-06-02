// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Lua scripting engine for Brume.
//!
//! Embeds Lua 5.5 via mlua and exposes the Brume API:
//!
//! ```lua
//! -- Control parameters
//! brume.set_param(part, "FmIndex", 3.5)
//! brume.note_on(part, 60, 0.8)
//! brume.note_off(part, 60)
//!
//! -- Query transport
//! local bpm = brume.bpm()
//! local beat = brume.beat()
//!
//! -- Callbacks (defined by the script)
//! function init() ... end
//! function on_beat(beat) ... end
//! function on_note(part, note, vel) ... end
//! function on_cc(part, cc, value) ... end
//! ```
//!
//! Scripts run on the main thread (not the audio thread). They send
//! `UiToEngine` messages through the same bounded channel as the UI.

mod api;
pub mod dsp_bindings;
mod loader;
pub mod lua_fx;
mod sandbox;

pub use loader::{ScriptEngine, ScriptError, ScriptMidiEvent};
pub use lua_fx::LuaFxSlot;
