// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Lua sandbox builder + per-call time-budget tracker.
//!
//! Brume embeds an mlua-backed Lua 5.5 VM and runs user-authored
//! scripts inside it. Three hardening measures bound what those scripts
//! can do, so a hostile or buggy script can't take down the engine
//! process:
//!
//! 1. **Restricted standard library.** `os`, `io`, `debug`, and the
//!    dynamic-loading half of `package` are not loaded. Scripts can't
//!    spawn processes, touch the filesystem, install their own debug
//!    hook (which would defeat the time budget), or `require` a C
//!    shared object. `coroutine`, `table`, `string`, `utf8`, `math`,
//!    and the safe parts of `base` and `package` remain — these are
//!    what the embedded `clock.lua` and the brume API rely on.
//!
//! 2. **Memory cap.** `set_memory_limit` clamps total VM allocation to
//!    [`MEMORY_LIMIT_BYTES`]. A runaway `string.rep` or table-of-tables
//!    explosion fails fast with `out of memory` instead of OOM-killing
//!    the engine.
//!
//! 3. **Time budget.** A debug hook fires every
//!    [`HOOK_EVERY_N_INSTRUCTIONS`] instructions and compares wall-
//!    clock elapsed against the budget held in [`ScriptBudget`]. If the
//!    budget is exceeded the hook returns an error and Lua aborts the
//!    current call. The host arms the budget immediately before each
//!    entry into Lua via [`ScriptBudget::arm`] (which returns an RAII
//!    guard) and the budget clears automatically on guard drop —
//!    panic-safe. Outside an armed window the hook is a no-op so
//!    sandbox-build code (registering bindings, loading clock.lua) and
//!    test fixtures don't accidentally trip it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::prelude::*;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, VmState};
use parking_lot::Mutex;

/// Per-VM allocation cap. 32 MB is generous for control scripts and FX
/// DSP graphs (wavetables, allocated coefficient arrays) on a CM5
/// while still bounding the worst case.
const MEMORY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// Default per-call wall-clock budget. Tight enough to kill an
/// `while true do end` within human reaction time; loose enough that
/// `init()` doing real setup work finishes comfortably.
const DEFAULT_BUDGET: Duration = Duration::from_millis(500);

/// Hook firing rate. Lua 5.5 on a CM5 core executes roughly 100M
/// instructions per second, so 10 000 instructions ≈ 100 µs between
/// checks — fine-grained enough to enforce a 500 ms budget within a
/// percent or so without burning real cycles in the hook itself.
const HOOK_EVERY_N_INSTRUCTIONS: u32 = 10_000;

/// Shared time-budget marker used by the sandbox debug hook.
///
/// Cheap to clone (it's an `Arc`). The host typically holds one
/// `ScriptBudget` alongside the `Lua` it built, and the same handle is
/// stored inside the VM's hook closure.
#[derive(Clone, Default)]
pub struct ScriptBudget {
    inner: Arc<Mutex<BudgetState>>,
}

#[derive(Default)]
struct BudgetState {
    start: Option<Instant>,
    budget: Duration,
}

impl ScriptBudget {
    /// Arm the budget for an upcoming entry into Lua and return an
    /// RAII guard. The guard's `Drop` impl clears the budget, so the
    /// hook reverts to a no-op even if the script call panics.
    pub fn arm(&self) -> BudgetGuard<'_> {
        self.arm_with(DEFAULT_BUDGET)
    }

    /// Arm with a custom budget. Useful for `init` callbacks that may
    /// legitimately do more work than a per-tick callback.
    pub fn arm_with(&self, budget: Duration) -> BudgetGuard<'_> {
        let mut state = self.inner.lock();
        state.start = Some(Instant::now());
        state.budget = budget;
        BudgetGuard { budget: self }
    }

    fn clear(&self) {
        self.inner.lock().start = None;
    }

    fn check(&self) -> LuaResult<VmState> {
        let state = self.inner.lock();
        if let Some(start) = state.start {
            if start.elapsed() > state.budget {
                return Err(LuaError::RuntimeError(format!(
                    "script exceeded {} ms time budget",
                    state.budget.as_millis()
                )));
            }
        }
        Ok(VmState::Continue)
    }
}

/// RAII guard that disarms the [`ScriptBudget`] on drop.
pub struct BudgetGuard<'a> {
    budget: &'a ScriptBudget,
}

impl Drop for BudgetGuard<'_> {
    fn drop(&mut self) {
        self.budget.clear();
    }
}

/// Build a Lua VM with restricted stdlib, a memory cap, and an armed
/// time-budget hook installed.
///
/// Returns the VM and a clone of its [`ScriptBudget`] so the caller
/// can arm/disarm the budget around each entry into user code.
pub fn build_sandboxed_lua() -> LuaResult<(Lua, ScriptBudget)> {
    // Whitelist of standard libraries safe for user scripts. BASE
    // gives us `pairs`, `ipairs`, `print`, `type`, `pcall`, `error`,
    // `tostring`, `tonumber`, `assert` — all required for any
    // reasonable script. The dangerous parts of BASE (`load`,
    // `loadstring`, `loadfile`, `dofile`) are nilled out below.
    //
    // Notable exclusions:
    //   - IO   — file I/O, popen, lines (filesystem + process spawn)
    //   - OS   — execute, exit, remove, rename, getenv (process spawn,
    //            arbitrary fs writes, environment leaks)
    //   - DEBUG — sethook (would let a script disable our budget hook),
    //            getlocal, setlocal, getupvalue, setupvalue (sandbox
    //            escape via stack walking)
    //
    // PACKAGE is loaded separately below so we can keep `package.path`
    // (the script-search side) while nilling `package.loadlib` (C-lib
    // loading).
    let safe_libs =
        StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;

    let lua = Lua::new_with(safe_libs, LuaOptions::default())?;

    lua.load_std_libs(StdLib::PACKAGE)?;
    if let Ok(pkg) = lua.globals().get::<LuaTable>("package") {
        // C-library loading + cpath.
        let _ = pkg.set("loadlib", LuaNil);
        let _ = pkg.set("cpath", "");
        let _ = pkg.set("searchpath", LuaNil);
    }

    // Strip the dangerous loaders from the global namespace. Each
    // `set` is independent — a missing symbol on some Lua build
    // shouldn't fail sandbox creation.
    let globals = lua.globals();
    for sym in &["load", "loadstring", "loadfile", "dofile"] {
        let _ = globals.set(*sym, LuaNil);
    }

    lua.set_memory_limit(MEMORY_LIMIT_BYTES)?;

    let budget = ScriptBudget::default();
    let budget_for_hook = budget.clone();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_EVERY_N_INSTRUCTIONS),
        move |_lua, _debug| budget_for_hook.check(),
    )?;

    Ok((lua, budget))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_version_is_5_5() {
        // Sanity check pinning the running VM to the 5.5 series. A
        // future accidental flip back to lua54 (or forward to a 5.6
        // beta) should fail this test rather than silently shipping
        // mismatched semantics.
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        let version: String = lua.load("return _VERSION").eval().unwrap();
        assert!(
            version.starts_with("Lua 5.5"),
            "expected Lua 5.5.x, got {version}"
        );
    }

    #[test]
    fn os_and_io_are_unavailable() {
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        // `os` and `io` should be nil — a user script trying to spawn
        // a process or read a file gets a runtime error, not a
        // privilege escape.
        for global in &["os", "io", "debug"] {
            let v: LuaValue = lua.globals().get(*global).unwrap();
            assert!(
                matches!(v, LuaValue::Nil),
                "{global} should be nil in the sandbox, got {v:?}"
            );
        }
    }

    #[test]
    fn loadstring_family_is_unavailable() {
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        for sym in &["load", "loadstring", "loadfile", "dofile"] {
            let v: LuaValue = lua.globals().get(*sym).unwrap();
            assert!(
                matches!(v, LuaValue::Nil),
                "{sym} should be nil in the sandbox, got {v:?}"
            );
        }
    }

    #[test]
    fn package_loadlib_is_disabled() {
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        let pkg: LuaTable = lua.globals().get("package").unwrap();
        let loadlib: LuaValue = pkg.get("loadlib").unwrap();
        assert!(matches!(loadlib, LuaValue::Nil));
    }

    #[test]
    fn time_budget_aborts_infinite_loop() {
        let (lua, budget) = build_sandboxed_lua().unwrap();
        let _g = budget.arm_with(Duration::from_millis(50));
        let result: LuaResult<()> = lua.load("while true do end").exec();
        assert!(
            result.is_err(),
            "infinite loop should be aborted by the time budget"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("time budget"),
            "error should mention the budget: {err}"
        );
    }

    #[test]
    fn budget_unarmed_allows_long_running() {
        // Without arming, the hook is a no-op even on a long loop.
        // This is the property that lets us register bindings,
        // include clock.lua, and run test fixtures without accidental
        // timeouts.
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        // 200_000 iterations of trivial work; should complete well
        // under 50 ms on any machine that can build the test.
        let result: LuaResult<()> = lua
            .load("local x = 0 for i = 1, 200000 do x = x + 1 end")
            .exec();
        assert!(
            result.is_ok(),
            "unarmed budget should not abort: {result:?}"
        );
    }

    #[test]
    fn memory_limit_blocks_runaway_allocation() {
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        // Try to allocate a string larger than the cap. Lua should
        // return a memory error before the host process is in
        // trouble.
        let result: LuaResult<()> = lua
            .load("local s = string.rep('x', 100 * 1024 * 1024)")
            .exec();
        assert!(result.is_err(), "100 MB string under a 32 MB cap must fail");
    }

    #[test]
    fn safe_stdlib_remains_available() {
        // Budget intentionally not armed: simple stdlib usage should
        // work without time pressure, exercising math, string, and
        // base-library access through the sandbox.
        let (lua, _budget) = build_sandboxed_lua().unwrap();
        let result: i64 = lua
            .load("return math.floor(string.len('hello') * 2)")
            .eval()
            .unwrap();
        assert_eq!(result, 10);
    }
}
