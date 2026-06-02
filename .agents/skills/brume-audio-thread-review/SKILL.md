---
name: brume-audio-thread-review
description: >
  Real-time-safety checklist for any change touching the audio thread:
  process_block, the four oscillators (FM/Harmonic/Timbral/Granular),
  Voice, the FX chain, transport, or any DSP primitive in dsp-core.
  Catches allocations, syscalls, panics, denormal traps, and lock-
  discipline regressions before they ship as audio glitches. Triggers
  on "audio thread", "process_block", "RT-safe", "real-time safe",
  "audio safety", "audio path review", "DSP review", "engine.rs
  changes", "voice.rs changes", "FX chain change", "oscillator
  change", "is this RT-safe", "audio glitch", "xrun".
---

# Audio-thread review

Run this checklist on any change that touches code that runs from
the audio callback. The audio thread has a hard deadline (one buffer
period — at 256 samples × 48 kHz that's 5.3 ms) and any miss surfaces
as an audible glitch (xrun on Linux/cpal, dropout on CoreAudio).
Every item below is a real failure mode that's bitten Brume at some
point or is documented in `crates/engine-runtime/src/engine.rs`'s
inline comments.

## What counts as the audio thread

Anything called from `BrumeEngine::process_block` or the cpal stream
callback in `apps/brume-main/src/main.rs::open_audio_stream`. The
chain:

```
cpal callback
  → engine.try_lock()                  (apps/brume-main/src/main.rs:73)
  → BrumeEngine::process_block          (engine-runtime/src/engine.rs)
    → drain UiToEngine messages
    → for each block of CHUNK frames:
      → Part::process per part
        → Voice::process per voice
          → OscillatorCore::process     (FM/Harmonic/Timbral/Granular)
          → StateVariableFilter::process_multi
          → DcBlocker::process
          → ADSR::process
      → FxChain::process
        → DampedComb, Allpass, OnePole, etc.
      → master_volume.process, limiter.process
    → flush_parameter_echoes
    → periodic UI pushes (TransportState, ScopeFrame, ModFrame,
      ParameterSnapshot)
```

The MIDI drainer thread (`midi_drainer_loop`) calls
`engine.handle_message` and `engine.flush_parameter_echoes` under
the same mutex, so anything called from `handle_message` is
audio-thread-adjacent and gets the same restrictions.

## Hard rules

### 1. No allocations

Forbidden in any audio-thread code path:

- `Vec::new`, `Vec::with_capacity` (per-call), `vec![]`.
- `String::from`, `format!`, `to_string`.
- `Box::new`.
- `HashMap::insert` past initial capacity (rehashing allocates).
- `Arc::new`, `Rc::new`, `clone()` on owning containers like
  `Vec<f32>` (refcount-bump clones on `Arc`/`Rc` are fine).

The exception: pre-allocated buffers held on a struct field, used
via `.clear() + .extend_from_slice()`. Capacity-stable after
warmup. See `LuaFxSlot::process_stereo` for the canonical pattern
(`crates/scripting/src/lua_fx.rs`).

The periodic `ParameterSnapshot` Vec allocation in `process_block`
(once per ~1 second, ~3 KB) is a documented exception — see the
inline comment near `% 200 == 0`.

### 2. No syscalls

Forbidden:

- `eprintln!`, `println!`, `dbg!`. These are stderr/stdout writes,
  which can block on a slow journald or tty.
- Any `std::fs` operation.
- `std::time::Instant::now()` is fine (it's a vDSO read on Linux,
  no syscall in steady state).
- `std::thread::sleep` is forbidden but obviously you wouldn't.

If you need to surface state, use `EngineToUi` messages over the
existing channel. See `crates/engine-runtime/src/engine.rs` for
how `flush_parameter_echoes` and `TransportState` do it.

### 3. No panics

`unwrap`, `expect`, `panic!`, indexing past length, integer
overflow in debug. The audio callback can't recover from a panic
— cpal handles it by terminating the stream, and the user gets
silence.

Use `parking_lot::Mutex` over `std::sync::Mutex` so a panic on
another thread holding the lock doesn't poison it. Use `.get(i)`
over `arr[i]` for indices that might be out of range. For numeric
clamping, use `.clamp()` rather than asserting bounds.

### 4. No locks held across long work

`engine.try_lock()` from the cpal callback fails fast and
zero-fills the buffer if contention is detected (see
`apps/brume-main/src/main.rs:73`). That's the safety net. Don't
defeat it by holding a different mutex during DSP — for example,
don't call `Arc<Mutex<Foo>>::lock()` inside the per-sample loop
where Foo lives on the engine. The engine mutex is the one and
only RT-side lock; everything else inside it should be plain
fields or atomics.

### 5. Denormal flush at asymptote sites

Filters with state that decays asymptotically toward zero (SVF
integrators, DcBlocker output, DampedComb feedback) must call
`brume_dsp_core::flush_subnormal` on their state assignment. See
`crates/dsp-core/src/denormal.rs` for the rationale and the helper.

A new filter that has decaying state and isn't covered by an
existing flush is a real RT-safety regression — subnormal floats
on Cortex-A76 cost ~10× normal floats per operation. After a few
seconds of silence the filter is in the slow path forever.

### 6. NaN / Inf doesn't propagate to the limiter

A NaN in master_l or master_r reaches ALSA, which can return
`EIO` and kill the stream. The limiter clamps to its threshold
but doesn't catch NaN. If you're introducing a code path that
could produce NaN (division, transcendentals on out-of-range
input), guard at the producer.

## Style guide for audio-thread code

- **Avoid transcendentals in the inner loop.** `phase.sin()` is ~30
  cycles on Cortex-A76; the shared LUT in
  `dsp_core::sine_normalized` is ~5-7. Use the LUT unless you've
  measured and the call site is cold.
- **Keep critical sections short.** Anything inside `engine.lock()`
  (which is everything in `process_block`) blocks the MIDI drainer.
- **Prefer fixed-size arrays to Vec.** `[f32; CHUNK]` on the stack
  beats `Vec<f32>` allocated once. CHUNK is 2048 frames in the
  engine; `[f32; 2048]` is 8 KB, fits the stack comfortably.
- **No floating-point comparisons of denormal-prone values.** A
  filter state at 1e-40 compared with `< 1e-30` is a slow-path
  read. `flush_subnormal` first.

## How to verify

1. **`cargo build --release`** — release builds catch panics that
   debug builds hide (overflow checks). Required for any DSP
   change.
2. **Run on a CM5.** A change that's correct but allocates will
   pass tests and CI; only running on the device under live audio
   surfaces xruns.
3. **Listen to a long silent tail.** A new filter without
   denormal flush manifests as the engine getting *slower* over
   tens of seconds of silence, not as glitchy audio. Watch
   `top` on the device after letting a reverb tail decay to zero
   for 30+ seconds.
4. **Hammer with controller automation.** A tight CC sweep tests
   the engine→UI channel saturation path. Brief glitches under
   sustained automation usually mean an alloc or syscall in the
   `SetParameter` handling.

## What to do when an existing change breaks RT-safety

Don't ship it. Either:

- Restructure to satisfy the rules (almost always possible).
- Move the work off the audio thread. The MIDI drainer or a
  dedicated worker thread can do allocations; the audio thread
  picks up the result via a channel or an atomic.
- If the work fundamentally must happen on the audio thread and
  fundamentally needs to allocate (rare), open an issue and
  align with the maintainer before the PR. There's almost
  always a better design.

## Out of scope for this skill

- Lock-free engine refactor (replacing `Arc<Mutex<BrumeEngine>>`
  with a snapshot-based design). That's documented in
  `apps/brume-main/src/main.rs` near `try_lock` and is a
  larger effort with its own design discussion.
- DSP correctness review (filter stability, oscillator aliasing,
  envelope shape correctness). This skill is about RT-safety,
  not signal quality.
