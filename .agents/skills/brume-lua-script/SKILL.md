---
name: brume-lua-script
description: >
  Writing or editing Lua scripts for Brume — both control scripts
  (the `brume` API: notes, params, transport, modulation, screen)
  and FX scripts (`fx` table with init/process/reset and the
  LuaBuffer userdata for audio buffers). Documents the sandbox
  bounds (memory cap, time budget, restricted stdlib), the
  available DSP primitives, and the contracts each script type must
  satisfy. Triggers on "lua", "lua script", "scripting", "write a
  script", "edit a script", "fx script", "lua fx", "brume api",
  "brume.note", "brume.param", "clock.run", "screen.draw", "lua
  sandbox", "scripting subsystem", "audio fx in lua".
---

# Brume Lua scripting

Brume embeds Lua 5.5 via mlua 0.11. Scripts run in a sandboxed VM
with hard bounds on memory, wall-clock time, and stdlib access. Two
script flavors exist: **control scripts** (drive notes / parameters
/ modulation from a programmable timeline) and **FX scripts**
(implement an audio processor that lives in the FX chain).

## Sandbox bounds

Every script VM is built via `crates/scripting/src/sandbox.rs`'s
`build_sandboxed_lua`. Three hardening measures apply:

- **Restricted stdlib.** `os`, `io`, `debug`, and the dynamic-loading
  half of `package` are not loaded. `coroutine`, `table`, `string`,
  `utf8`, `math`, and the safe parts of `base` and `package` are
  available. `load`, `loadstring`, `loadfile`, `dofile` are nilled.
- **32 MB memory cap.** A runaway `string.rep` or table-of-tables
  fails fast with `out of memory`.
- **500 ms per-call wall-clock budget** (2 s for `init`). A debug
  hook fires every 10 000 instructions and aborts the call if
  exceeded. An infinite loop in `on_tick` can't wedge the engine.

You can't `require` a C library, spawn a process, read or write
files outside what the API exposes, or hook the VM internals.

## Where scripts live

- **User scripts**: `~/brume/scripts/` on the device. Visible (no
  leading dot — distinct from engine-internal `~/.brume/`).
- **Bundled examples**: `crates/scripting/scripts/` in the repo.
- **FX scripts**: by convention under `~/brume/scripts/fx/`. The
  user picks them from the UI's FX page.

## Control scripts: the `brume` API

A control script defines callback functions that the engine invokes
on events. All the available callbacks:

```lua
function init() end
  -- Called once on script load. Set up state, register params,
  -- start clock coroutines.

function on_beat(beat) end
  -- Called on every beat boundary (1/4 note). `beat` is a float
  -- counting from script load.

function on_tick(beat) end
  -- Called ~50 ms (the UI tick rate). Use for animation, polling,
  -- or anything that wants to update faster than per-beat.

function on_note(part, note, velocity) end
  -- Called when a MIDI note hits a part the script is interested in.
  -- velocity == 0 means note-off.

function on_cc(part, cc, value) end
  -- Called on incoming CC. `value` is normalized 0..1.

function cleanup() end
  -- Called when the script unloads. Cancel outstanding state.
```

The `brume` global table is the API surface:

```lua
-- Engine constants
brume.FM, brume.HARMONIC, brume.TIMBRAL, brume.GRANULAR  -- part indices
brume.ALG.STACK, brume.ALG.FAN_IN, brume.ALG.CHAIN, ...  -- FM algorithms

-- Triggering notes
brume.note_on(part, note, velocity)
brume.note_off(part, note)

-- Parameters (values clamped to ParameterId's documented range)
brume.set_param(part, "FmIndex", 3.5)
brume.set_param(part, "FilterCutoff", 8000.0)

-- High-level FM patch helper
brume.set_fm_patch(brume.FM, {
  algorithm = brume.ALG.FAN_IN,
  ratios    = {1.0, 2.0, 3.0, 4.0, 5.0, 6.0},
  levels    = {0.6, 0.5, 0.4, 0.3, 0.2, 0.1},
  feedback  = 0.3,
  index     = 2.5,
})

-- Transport
brume.bpm()         -- current BPM
brume.beat()        -- fractional beats from start

-- Music helpers
brume.chord(60, 'maj')        -- {60, 64, 67}
brume.chord(60, 'min7')       -- {60, 63, 67, 70}
brume.midi_to_freq(69)        -- 440.0
note.A4, note.C5              -- MIDI numbers

-- Scheduling
brume.after(beats, function(b) ... end)  -- fire callback after `beats`
```

The `clock` global, defined by `crates/scripting/src/clock.lua`,
adds coroutine-based musical timing:

```lua
clock.run(function()
  while true do
    brume.note_on(brume.FM, 60, 0.5)
    clock.sync(1/4)              -- wait a quarter note
    brume.note_off(brume.FM, 60)
    clock.sync(1/4)
  end
end)

clock.sleep(seconds)             -- wall-clock wait inside a coroutine
clock.cancel(thread_id)          -- stop a running thread
clock.cancel_all()               -- stop all (called automatically on unload)
```

The `screen` module (in `crates/scripting/src/screen.lua`) draws to
the device's status line / overlay area. Use sparingly; the main UI
is iced-rendered and the screen API is for ambient script-driven
indicators only.

## FX scripts: the `fx` table

An FX script defines a global `fx` table the host pulls from. The
contract:

```lua
fx = {
  name = "My Effect",          -- shown in the UI

  params = {                    -- optional; UI builds sliders from this
    { name = "drive", label = "DRIVE", min = 0, max = 1, default = 0.3 },
    { name = "color", label = "COLOR", min = 20, max = 20000, default = 1000 },
  },

  init = function(self, sr)
    -- Called once. Build any DSP graph. `sr` is the sample rate.
    self.lp = dsp.lowpass(self.color)
    self.sat = dsp.saturator()
  end,

  process = function(self, left, right)
    -- Called per audio block. left and right are LuaBuffer userdata
    -- with __index/__newindex/__len metamethods, so:
    for i = 1, #left do
      left[i] = self.sat:process(self.lp:process(left[i]))
      right[i] = left[i]  -- mono-mix to both channels
    end
    -- No return value needed — host reads buffers directly.
  end,

  reset = function(self)
    -- Optional. Called on transport stop / clear. Reset DSP state.
    self.lp:reset()
    self.sat:set_drive(0)
  end,
}
```

A `mix` parameter is automatically added if not declared. The host
applies wet/dry blending after `process` returns:

```
output = dry_input * (1 - mix) + script_output * mix
```

So the script's `process` writes the fully-wet signal into
`left`/`right`. Don't apply your own dry/wet inside the script.

If `mix < 0.001` the host skips the call entirely (early-return),
so a fully-disabled FX costs no Lua runtime.

## DSP primitives — the `dsp` global

Available to any script (control or FX) via the `dsp` global:

```lua
dsp.delay(max_samples)          -- DelayLine
  :process(input)               -- returns the delayed sample
  :set_time(samples)            -- set delay length (≤ max_samples)
  :reset()

dsp.allpass(max_samples, gain)  -- AllpassDelay
  :process(input)
  :set_time(samples)
  :set_gain(gain)
  :reset()

dsp.lowpass(cutoff)             -- OnePole lowpass at sample-rate
  :process(input)
  :set_cutoff(hz)
  :reset()

dsp.highpass(cutoff)            -- OnePole highpass

dsp.saturator()                 -- Saturator
  :process(input)
  :set_drive(amount)            -- 0..many
  :set_type(0|1|2|3)            -- Soft / Hard / Tape / Tube
  :set_mix(amount)              -- internal wet/dry

dsp.wavefolder()                -- Wavefolder
  :process(input)
  :set_drive(amount)
  :set_stages(n)                -- u32

dsp.sample_rate                 -- number, the active sample rate
```

Each primitive is a userdata wrapping a `parking_lot::Mutex`-guarded
DSP object. Internal cost: ~50-100 ns per `process` call (lock +
DSP work + unlock).

## LuaBuffer userdata in detail

Inside `process(self, left, right)`, the buffers are not Lua tables
— they're `LuaBuffer` userdata backed by host-owned `Vec<f32>`. The
metamethods make them behave like 1-indexed arrays:

```lua
left[i]           -- read sample i (1..#left). Out of range → nil.
left[i] = value   -- write sample i. Out of range → silent no-op.
#left             -- length
```

You **cannot** insert beyond the end (the buffer is fixed-size for
the call). You **cannot** return a different table from `process`
— the host reads the buffers directly after `process` returns, so
mutating them in place is the only way to affect output.

## Performance notes

- **Don't allocate Lua tables in `process`.** Each `process` call
  is on the audio thread or RT-adjacent; per-block allocation is
  the per-buffer-table tax we deliberately removed by introducing
  `LuaBuffer`. Reuse `self.*` fields instead.
- **The 500 ms time budget is generous.** A typical 256-sample
  block at 48 kHz needs to complete in 5.3 ms; 500 ms means a
  runaway loop has plenty of room before the budget kicks in.
  Don't rely on the budget as a bug-finder.
- **GC is stopped after `init`.** `lua_fx` calls `lua.gc_stop()` so
  the audio path doesn't trigger GC. A `gc_step` runs once per
  `process` call to keep memory bounded over time. Don't allocate
  lots of garbage from `process` and trust the GC to catch up;
  it won't, in audio time.
- **Test with a long run.** A leak that takes minutes to OOM looks
  fine in a unit test. Run a script in the UI for 5+ minutes
  watching `top` on the device to catch slow leaks.

## Testing scripts

The scripting crate has tests in
`crates/scripting/src/loader.rs::tests` that exercise the API end-
to-end. Add tests there for new API surface:

```rust
#[test]
fn your_new_api_works() {
    let (tx, rx) = bounded(256);
    let dir = std::env::temp_dir().join("brume-test-yourapi");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("test.lua"),
        r#"
        function init()
            brume.your_new_api(...)
        end
        "#,
    ).unwrap();
    let mut engine = ScriptEngine::new(tx, &dir).unwrap();
    engine.load_script("test").unwrap();
    // Assert the right UiToEngine messages appeared on rx.
}
```

For new FX-script behavior, the pattern is in
`crates/scripting/src/lua_fx.rs::tests`. The `lua_fx_passthrough`
and `lua_fx_with_dsp` tests are good templates.

## Anti-patterns

- **Don't** call `os.execute` or `io.open`. They're sandbox-blocked
  and the script will error out — but more importantly, scripts
  are user-authored content and should never need filesystem or
  process access. If a script wants persistent state, that's a
  feature gap; open an issue.
- **Don't** allocate per-block in `process`. Inflates the GC step
  cost and can underrun the audio buffer.
- **Don't** call `brume.set_param` at audio rate from a control
  script. The engine channel is bounded; thousands of CCs per
  second saturate it. Coalesce or rate-limit.
- **Don't** rely on `print` for diagnostics in a long-running
  script. The print buffer caps at a sensible size; spammy
  scripts lose their tail.
- **Don't** assume scripts are persistent across restarts. Brume
  doesn't auto-load the last script on launch — that's a
  deliberate design choice. Scripts are explicit user intent.

## Where to find more

- `crates/scripting/src/api.rs` — the canonical `brume` table
  registration code. Read this when you need to know exactly what
  the API does.
- `crates/scripting/src/clock.lua` — clock module source.
- `crates/scripting/src/screen.lua` — screen module source.
- `crates/scripting/scripts/` — example user scripts.
- `crates/scripting/src/sandbox.rs` — the hardening
  implementation, including the test suite that pins each bound.
