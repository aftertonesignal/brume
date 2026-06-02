// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! LuaFxSlot — a Lua-scripted audio effect that implements FxSlot.
//!
//! Each LuaFxSlot has its own isolated Lua VM with DSP primitives
//! available. The script defines:
//!
//! ```lua
//! fx = {
//!   name = "My Effect",
//!   params = {
//!     { name = "mix", label = "MIX", min = 0, max = 1, default = 0.3 },
//!   },
//!   init = function(self, sr) ... end,
//!   process = function(self, left, right)
//!     for i = 1, #left do
//!       left[i] = left[i] * 0.5      -- mutate in place
//!       right[i] = right[i] * 0.5
//!     end
//!     -- no return value needed; the host reads the buffers directly
//!   end,
//! }
//! ```
//!
//! `left` and `right` are `LuaBuffer` userdata (not Lua tables) backed
//! by host-owned `Vec<f32>`. Idiomatic indexing (`left[i]`, `#left`,
//! and assignment) works through `__index` / `__newindex` / `__len`
//! metamethods, so existing scripts written against the old
//! table-based API continue to work without modification — the only
//! behavioral change is that scripts no longer need to return tables;
//! mutation is in-place.
//!
//! The buffers are constructed once per `LuaFxSlot` and reused for
//! every audio block, so the audio thread no longer pays a
//! per-block Lua-table allocation tax.

use std::time::Duration;

use brume_fx_chain::{FxParamDef, FxSlot};
use crossbeam_channel::Sender;
use mlua::prelude::*;

use crate::sandbox::{self, ScriptBudget};

/// `from_file` / `from_source` allow longer than the default budget
/// because they include script load + `init` (allocating delay lines,
/// building wavetables). 2 s mirrors `loader::INIT_BUDGET`.
const FX_INIT_BUDGET: Duration = Duration::from_secs(2);

/// Initial capacity for the LuaBuffer backing Vecs. Chosen to comfortably
/// fit a typical 256-sample audio block without an early grow; if a host
/// runs us with larger blocks the Vec just grows once and stays grown.
const BUFFER_CAPACITY_HINT: usize = 1024;

/// Userdata wrapping a host-owned `Vec<f32>`, exposed to Lua as a
/// 1-indexed sample buffer. Mutation through `__newindex` writes
/// directly into the Vec; `__index` reads return `nil` for
/// out-of-range indices to match Lua table semantics, but in-range
/// scripts (using `for i = 1, #left do`) never see this.
struct LuaBuffer {
    samples: Vec<f32>,
}

impl LuaUserData for LuaBuffer {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method("__index", |_, this, idx: usize| {
            // Lua arrays are 1-based.
            Ok(idx
                .checked_sub(1)
                .and_then(|i| this.samples.get(i).copied()))
        });
        methods.add_meta_method_mut("__newindex", |_, this, (idx, value): (usize, f32)| {
            if let Some(slot) = idx.checked_sub(1).and_then(|i| this.samples.get_mut(i)) {
                *slot = value;
            }
            // Out-of-range writes are silently ignored — matches the
            // safety profile of the prior LuaTable approach where a
            // script writing past the end would have grown the table
            // without affecting host audio output.
            Ok(())
        });
        methods.add_meta_method("__len", |_, this, ()| Ok(this.samples.len()));
    }
}

/// A Lua-scripted FX slot with its own sandboxed VM.
pub struct LuaFxSlot {
    lua: Lua,
    budget: ScriptBudget,
    /// Engine-side slot name. Routes `UiToEngine::SetFxParam {
    /// slot, ... }` and `UiToEngine::RemoveFxSlot(slot)` to this
    /// instance. Defaults to whatever the script declared in
    /// `fx.name` (the "display name"); the UI overrides this to a
    /// stable engine-side name (e.g. "LuaFx") via `with_slot_name`
    /// so slot routing doesn't churn when the user swaps which
    /// script is loaded into the LUA tab.
    name: String,
    /// Display name shown to the user, taken from the script's
    /// `fx.name` field. Stays separate from `name` so we can keep
    /// the engine slot routing stable while still showing the
    /// script's own label in the UI.
    display_name: String,
    param_defs: Vec<FxParamDef>,
    /// Persistent userdata buffers passed to `process(self, left, right)`
    /// every audio block. Constructed once at FX init; refilled in-place
    /// via `borrow_mut` per block. `AnyUserData` is itself a refcounted
    /// handle so cloning into the call args is cheap.
    buf_l: LuaAnyUserData,
    buf_r: LuaAnyUserData,
    /// Optional non-blocking sink for runtime errors hit inside
    /// `process_stereo` on the audio thread. Replaces the previous
    /// `eprintln!` (which would syscall + potentially stall on
    /// journald) with a wait-free `try_send`. The receiver is owned
    /// by the dispatcher tick, which forwards drained messages to
    /// the UI as toasts. Drops on full — error reporting is best-
    /// effort by design; a flooded channel means the script is
    /// erroring every block and one or two surfaced messages tell
    /// the user enough to unload it.
    errors_tx: Option<Sender<String>>,
}

impl LuaFxSlot {
    /// Creates a new `LuaFxSlot` by loading a script file.
    ///
    /// The script must define a global `fx` table with `name`, `process`,
    /// and optionally `params` and `init`. Any future change to the
    /// sandbox / budget / param-parsing pipeline lands in `from_source`
    /// alone — this constructor is a thin file-IO wrapper around it.
    pub fn from_file(path: &std::path::Path, sample_rate: f32) -> Result<Self, String> {
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_source(&source, sample_rate)
    }

    /// Creates from a Lua source string. Owns the sandbox + budget +
    /// param-table parsing + `init` call + GC stop sequence; both
    /// `from_file` and direct callers (tests) flow through here.
    pub fn from_source(source: &str, sample_rate: f32) -> Result<Self, String> {
        let (lua, budget) =
            sandbox::build_sandboxed_lua().map_err(|e| format!("sandbox init: {e}"))?;

        crate::dsp_bindings::register_dsp(&lua, sample_rate)
            .map_err(|e| format!("dsp init: {e}"))?;

        {
            let _g = budget.arm_with(FX_INIT_BUDGET);
            lua.load(source).exec().map_err(|e| format!("load: {e}"))?;
        }

        let fx: LuaTable = lua
            .globals()
            .get("fx")
            .map_err(|e| format!("script must define global 'fx' table: {e}"))?;

        let name: String = fx.get("name").unwrap_or_else(|_| "Lua FX".to_string());

        let mut param_defs = Vec::new();
        if let Ok(params) = fx.get::<LuaTable>("params") {
            for pair in params.sequence_values::<LuaTable>() {
                if let Ok(p) = pair {
                    let def = FxParamDef {
                        name: p.get("name").unwrap_or_default(),
                        label: p.get("label").unwrap_or_default(),
                        min: p.get("min").unwrap_or(0.0),
                        max: p.get("max").unwrap_or(1.0),
                        default: p.get("default").unwrap_or(0.0),
                        unit: p.get("unit").ok(),
                    };
                    param_defs.push(def);
                }
            }
        }
        if !param_defs.iter().any(|p| p.name == "mix") {
            param_defs.push(FxParamDef {
                name: "mix".into(),
                label: "MIX".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                unit: None,
            });
        }

        if let Ok(init_fn) = fx.get::<LuaFunction>("init") {
            let _g = budget.arm_with(FX_INIT_BUDGET);
            init_fn
                .call::<()>((fx.clone(), sample_rate))
                .map_err(|e| format!("init: {e}"))?;
        }

        let buf_l = make_buffer(&lua)?;
        let buf_r = make_buffer(&lua)?;

        lua.gc_stop();

        Ok(Self {
            lua,
            budget,
            display_name: name.clone(),
            name,
            param_defs,
            buf_l,
            buf_r,
            errors_tx: None,
        })
    }

    /// Wires up a non-blocking error sink. Builder-style so existing
    /// call sites (and tests) that don't care about runtime errors
    /// stay unchanged. The dispatcher tick (in ui-native) creates a
    /// bounded channel pair, hands the sender to every LuaFxSlot it
    /// instantiates, and drains the receiver each frame.
    #[must_use]
    pub fn with_errors_tx(mut self, tx: Sender<String>) -> Self {
        self.errors_tx = Some(tx);
        self
    }

    /// Overrides the engine-side slot name. The default `name` comes
    /// from the script's `fx.name` field, which is fine for ad-hoc
    /// use but unstable when the user swaps which script is loaded
    /// — every swap would route SetFxParam to a new slot label and
    /// the UI's tab state would churn. Calling
    /// `with_slot_name("LuaFx")` (or whatever the host chose) keeps
    /// the engine routing stable across swaps; the script's
    /// declared name is preserved separately as the display name.
    #[must_use]
    pub fn with_slot_name(mut self, slot_name: impl Into<String>) -> Self {
        self.name = slot_name.into();
        self
    }

    /// The user-facing label declared by the script's `fx.name`
    /// field. Distinct from `name()` (the engine slot identifier);
    /// the UI shows this in the LUA tab heading.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

fn make_buffer(lua: &Lua) -> Result<LuaAnyUserData, String> {
    lua.create_userdata(LuaBuffer {
        samples: Vec::with_capacity(BUFFER_CAPACITY_HINT),
    })
    .map_err(|e| format!("buffer init: {e}"))
}

/// Refill a persistent buffer userdata from a host slice. Returns
/// `false` if the userdata can't be borrowed (would only happen if a
/// script kept a reference inside Lua and we somehow re-entered, which
/// the audio thread doesn't do — but we degrade safely rather than
/// panic).
fn load_buffer(ud: &LuaAnyUserData, src: &[f32]) -> bool {
    match ud.borrow_mut::<LuaBuffer>() {
        Ok(mut buf) => {
            buf.samples.clear();
            buf.samples.extend_from_slice(src);
            true
        }
        Err(_) => false,
    }
}

impl FxSlot for LuaFxSlot {
    fn name(&self) -> &str {
        &self.name
    }
    fn params(&self) -> &[FxParamDef] {
        &self.param_defs
    }

    fn set_param(&mut self, name: &str, value: f32) {
        if let Ok(fx) = self.lua.globals().get::<LuaTable>("fx") {
            let _ = fx.set(name, value);
        }
    }

    fn get_param(&self, name: &str) -> f32 {
        self.lua
            .globals()
            .get::<LuaTable>("fx")
            .and_then(|fx| fx.get(name))
            .unwrap_or(0.0)
    }

    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        let fx: LuaTable = match self.lua.globals().get("fx") {
            Ok(t) => t,
            Err(_) => return,
        };

        let mix: f32 = fx.get("mix").unwrap_or(0.0);
        if mix < 0.001 {
            return;
        }

        let process_fn: LuaFunction = match fx.get("process") {
            Ok(f) => f,
            Err(_) => return,
        };

        // Refill the persistent userdata buffers in place. After warmup
        // these `extend_from_slice` calls don't allocate — the Vec
        // capacity is held by `LuaBuffer` across calls.
        if !load_buffer(&self.buf_l, left) || !load_buffer(&self.buf_r, right) {
            return;
        }

        // Call process(fx, left, right). The script mutates the
        // buffers in-place via `__newindex`; any return value is
        // ignored. Budgeted: a runaway loop in `process` can't wedge
        // the audio thread — at most it consumes its budget worth of
        // wall-clock and returns a runtime error, after which we skip
        // the wet output for this audio block (dry signal is still
        // intact in `left` / `right` since we never mutated those).
        let _g = self.budget.arm();
        let result: LuaResult<()> = process_fn.call((fx, self.buf_l.clone(), self.buf_r.clone()));
        drop(_g);

        match result {
            Ok(()) => {
                let dry = 1.0 - mix;
                let bl = match self.buf_l.borrow::<LuaBuffer>() {
                    Ok(b) => b,
                    Err(_) => return,
                };
                let br = match self.buf_r.borrow::<LuaBuffer>() {
                    Ok(b) => b,
                    Err(_) => return,
                };
                let n = left.len().min(bl.samples.len()).min(br.samples.len());
                for i in 0..n {
                    left[i] = left[i] * dry + bl.samples[i] * mix;
                    right[i] = right[i] * dry + br.samples[i] * mix;
                }
            }
            Err(e) => {
                // Honest contract: no stderr / no syscall;
                // best-effort channel send; allocates only on this
                // error path. The steady-state ok branch above
                // remains allocation-free. Wait-free `try_send`
                // drops on full or absent — surfacing one or two
                // errors per script is enough for the user to act
                // on, and a flooding script gets unloaded anyway.
                if let Some(ref tx) = self.errors_tx {
                    let _ = tx.try_send(format!("lua fx '{}': {e}", self.name));
                }
            }
        }

        // Occasional GC step to prevent unbounded growth
        self.lua.gc_step().ok();
    }

    fn reset(&mut self) {
        if let Ok(fx) = self.lua.globals().get::<LuaTable>("fx") {
            if let Ok(reset_fn) = fx.get::<LuaFunction>("reset") {
                let _g = self.budget.arm();
                let _ = reset_fn.call::<()>(fx.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_fx_passthrough() {
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Test Pass",
                mix = 1.0,
                process = function(self, left, right)
                    return left, right
                end,
            }
        "#,
            48000.0,
        )
        .unwrap();

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);

        let mut left = vec![0.5_f32; 64];
        let mut right = vec![0.3_f32; 64];
        slot.process_stereo(&mut left, &mut right);

        assert!((left[0] - 0.5).abs() < 0.01, "passthrough L: {}", left[0]);
        assert!((right[0] - 0.3).abs() < 0.01, "passthrough R: {}", right[0]);
    }

    #[test]
    fn lua_fx_with_dsp() {
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Test Filter",
                mix = 1.0,
                init = function(self, sr)
                    self.lp = dsp.lowpass(200)
                end,
                process = function(self, left, right)
                    for i = 1, #left do
                        left[i] = self.lp:process(left[i])
                        right[i] = left[i]
                    end
                    return left, right
                end,
            }
        "#,
            48000.0,
        )
        .unwrap();

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);

        // Feed white-ish noise
        let mut left: Vec<f32> = (0..256)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let mut right = left.clone();
        slot.process_stereo(&mut left, &mut right);

        // Low-pass at 200Hz should significantly reduce this alternating signal
        let peak = left.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!(peak < 0.4, "lowpass should attenuate HF: peak={peak}");
    }

    #[test]
    fn lua_fx_buffer_reused_across_many_blocks() {
        // The whole point of the userdata buffer: a single `LuaFxSlot`
        // processes many audio blocks in a row without allocating a
        // fresh Lua object per block. We can't observe "no allocation"
        // directly from a unit test, but we can prove functional
        // correctness across repeated calls — the same buffer must
        // accept fresh input data and produce the right output every
        // time, with no state bleed between blocks.
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Test Gain",
                mix = 1.0,
                process = function(self, left, right)
                    for i = 1, #left do
                        left[i] = left[i] * 2.0
                        right[i] = right[i] * 2.0
                    end
                end,
            }
        "#,
            48000.0,
        )
        .unwrap();

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);

        for block in 0..16 {
            let v = block as f32 * 0.05;
            let mut left = vec![v; 64];
            let mut right = vec![-v; 64];
            slot.process_stereo(&mut left, &mut right);
            assert!(
                (left[0] - v * 2.0).abs() < 1e-4,
                "block {block}: expected {} got {}",
                v * 2.0,
                left[0]
            );
            assert!(
                (right[0] - (-v) * 2.0).abs() < 1e-4,
                "block {block}: expected {} got {}",
                -v * 2.0,
                right[0]
            );
        }
    }

    #[test]
    fn lua_fx_buffer_out_of_range_writes_are_silent() {
        // A misbehaving script writing past the end shouldn't crash
        // the audio thread or corrupt host memory. The legitimate
        // samples it touched in-range should still land correctly.
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Naughty",
                mix = 1.0,
                process = function(self, left, right)
                    -- Walk past the end deliberately. Out-of-range
                    -- writes should be no-ops, in-range writes apply.
                    for i = 1, #left + 100 do
                        left[i] = 0.7
                        right[i] = 0.3
                    end
                end,
            }
        "#,
            48000.0,
        )
        .unwrap();

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);

        let mut left = vec![0.0_f32; 32];
        let mut right = vec![0.0_f32; 32];
        slot.process_stereo(&mut left, &mut right);

        for i in 0..32 {
            assert!((left[i] - 0.7).abs() < 1e-4, "left[{i}] = {}", left[i]);
            assert!((right[i] - 0.3).abs() < 1e-4, "right[{i}] = {}", right[i]);
        }
    }

    #[test]
    fn lua_fx_name_and_params() {
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Cool Reverb",
                params = {
                    { name = "decay", label = "DECAY", min = 0.1, max = 10, default = 2 },
                    { name = "size", label = "SIZE", min = 0, max = 1, default = 0.5 },
                },
                process = function(self, l, r) return l, r end,
            }
        "#,
            48000.0,
        )
        .unwrap();

        assert_eq!(fx.name(), "Cool Reverb");
        // params includes user-defined + auto-added mix
        assert!(fx.params().len() >= 3);
    }

    #[test]
    fn lua_fx_runtime_errors_route_to_channel_not_stderr() {
        // Regression test for the audio-thread eprintln! that codex
        // flagged: a script whose process() raises a Lua error must
        // surface that through the errors_tx channel, not via a
        // stderr syscall under the audio callback. We can't observe
        // "no syscall" directly, but we can prove the channel
        // delivers exactly one message per erroring process call.
        let (tx, rx) = crossbeam_channel::bounded::<String>(8);
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Boom",
                mix = 1.0,
                process = function(self, left, right)
                    error("deliberate failure")
                end,
            }
        "#,
            48000.0,
        )
        .unwrap()
        .with_errors_tx(tx);

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);

        let mut left = vec![0.5_f32; 32];
        let mut right = vec![0.5_f32; 32];
        slot.process_stereo(&mut left, &mut right);

        // Dry signal must survive an erroring process — the slot
        // contract is "wet output skipped on error, dry passes
        // through." We assert dry survival here too because it
        // would be a regression if the error path mutated input.
        for v in &left {
            assert!((*v - 0.5).abs() < 1e-4);
        }

        // Exactly one error message on the channel, mentioning the
        // FX name so the UI surface can attribute it.
        let msg = rx.try_recv().expect("error message must arrive");
        assert!(msg.contains("Boom"), "expected name in error: {msg}");
        assert!(rx.try_recv().is_err(), "channel should now be empty");
    }

    #[test]
    fn lua_fx_runtime_errors_drop_silently_when_channel_full() {
        // The try_send in the error branch is best-effort — a
        // flooding script with a full channel must not panic, must
        // not block, must not leak resources. Build a 1-slot
        // channel, fill it, run the slot through a deliberate
        // error, and confirm we exit cleanly with no panic.
        let (tx, rx) = crossbeam_channel::bounded::<String>(1);
        tx.try_send("preexisting".into()).unwrap();
        let fx = LuaFxSlot::from_source(
            r#"
            fx = {
                name = "Boom",
                mix = 1.0,
                process = function(self, left, right)
                    error("again")
                end,
            }
        "#,
            48000.0,
        )
        .unwrap()
        .with_errors_tx(tx);

        let mut slot: Box<dyn FxSlot> = Box::new(fx);
        slot.set_param("mix", 1.0);
        let mut left = vec![0.5_f32; 16];
        let mut right = vec![0.5_f32; 16];
        // Should not panic even though the channel is full.
        slot.process_stereo(&mut left, &mut right);
        assert_eq!(rx.try_recv().unwrap(), "preexisting");
        assert!(rx.try_recv().is_err());
    }
}
