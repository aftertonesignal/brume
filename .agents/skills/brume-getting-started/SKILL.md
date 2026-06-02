---
name: brume-getting-started
description: >
  Orient a coding agent that's new to the Brume codebase. Names what
  Brume is, where to find what, the load-bearing invariants, and the
  reading order for any contribution scope. Triggers on "new to
  brume", "starting on brume", "orient me on brume", "what is brume",
  "tour the brume codebase", "first time working on brume", "where
  do i start with brume", "brume codebase overview", "introduce me
  to brume", "load brume context".
---

# Getting started with the Brume codebase

This skill is the first one to load when an agent is opening Brume
for the first time, or returning after long enough that the context
needs a refresh. It supersedes `AGENTS.md` for procedural use; that
file is the always-loaded floor and this one fills in detail.

## What Brume is, in one paragraph

Brume is a four-part multi-timbral synthesizer running on a Raspberry
Pi Compute Module 5. It has four engines (FM, Harmonic, Timbral,
Granular), six voices per part, one filter, an effects chain, a
sandboxed Lua scripting layer, a 10.1-inch touchscreen UI built on
iced + wgpu, and a USB bridge that presents Brume to a host
computer as a class-compliant audio + MIDI device. The whole thing
is one Rust workspace plus a `brumectl` companion CLI for the host
side. Hobby project, GPL-3.0-only, single maintainer, no commercial
pressure.

## Hard rules to load before any change

These are non-negotiable. Internalize them before reading code:

- `unsafe_code = "forbid"` workspace-wide. Don't add `unsafe`. CI
  fails the build if you do.
- No allocations on the audio thread (`process_block` and its
  callees). Pre-allocate, reuse buffers.
- No syscalls on the audio thread — that includes `eprintln!`.
  Use the engine→UI channel for diagnostics.
- `parking_lot::Mutex` over `std::sync::Mutex` on hot paths. The
  former doesn't poison; the latter can panic the audio thread on
  the next sample after any other thread panics holding the lock.
- Atomic commits: one logical change per commit. Don't bundle
  unrelated work.
- Schema-version every persisted format. `Patch`, `Perf`, and
  `ControlMatrix` carry `version: u32` and reject futures.
  See the `brume-schema-version-bump` skill before touching them.

The full set lives in `CONTRIBUTING.md` under "House style."

## Crate map (15 crates + 2 apps)

```
crates/
  app-protocol/     UiToEngine + EngineToUi messages on the bridge.
                    The wire shape between the audio thread and the UI;
                    edits here ripple to both sides.
  audio-io/         cpal/ALSA backend, output device enumeration.
                    Feeds audio to the host or to USB Meridian stems.
  common/           ParameterId enum, IPC macros, OscillatorMode.
                    Used by every other crate; keep this thin.
  control-model/    MIDI ChannelMap, ControlMatrix, CC bindings.
                    Persisted to ~/.brume/cc-bindings.json.
  dsp-core/         filters, delays, saturation, sine LUT, denormal
                    flush, smoothers — pure DSP primitives. No
                    Brume-specific knowledge.
  engine-runtime/   BrumeEngine, Voice, Part, transport, the four
                    oscillators (FM/Harmonic/Timbral/Granular). The
                    audio thread runs here.
  fx-chain/         Saturator/Delay/Chorus/Reverb chain + dattorro
                    plate.
  midi-io/          ALSA MIDI input thread, drainer, activity tracker.
                    Drains into UiToEngine for the audio thread.
  modulation/       LFO/Seq sources, ModRouter, transition shapes.
                    24 named shapes (Caffeinated, Silk, Whiplash, ...).
  patch-store/      ~/.brume/library/ on-disk Patch/Perf serialization.
                    Schema-versioned.
  platform-rpi/     Pi-specific bring-up + SCHED_FIFO elevation.
                    Linux-only, gated by cfg.
  scripting/        sandboxed Lua 5.5 VM, brume API, FX userdata.
                    See `brume-lua-script` skill.
  settings/         settings.json (output device id, etc.).
  ui-native/        iced + wgpu touchscreen UI, persist workers,
                    control surface drivers (nanoKONTROL2, LCXL3).
                    Largest crate; lib.rs is ~2400 LOC.
apps/
  brume-main/       The synthesizer process. Wires everything up.
  brumectl/         Host-side companion CLI (macOS + Linux).
deploy/scripts/     CM5 systemd / USB gadget / labwc setup.
.github/workflows/  CI (ci.yml) + release matrix (release.yml).
```

## Reading order by contribution scope

**Touching DSP / audio path** (oscillator, filter, FX):
1. The relevant crate's `lib.rs` doc-comment.
2. `crates/engine-runtime/src/engine.rs` — especially `process_block`
   to understand the per-block control flow.
3. The `brume-audio-thread-review` skill before opening a PR.

**Adding a new control / parameter:**
1. `crates/common/src/lib.rs` — the `ParameterId` enum.
2. Walk through the `brume-add-parameter` skill end-to-end.

**Touching persistence** (Patch, Perf, cc-bindings, transport.json):
1. `crates/patch-store/src/lib.rs` `Patch` definition.
2. The `brume-schema-version-bump` skill.

**Writing or editing a Lua script:**
1. `crates/scripting/src/lib.rs` doc-comment.
2. The `brume-lua-script` skill.

**Deploying to a CM5:**
1. `DEPLOY.md` for the high-level loop.
2. The `brume-cm5-deploy` skill for the procedural detail.

**UI work** (touchscreen, iced):
1. `crates/ui-native/src/lib.rs` — start with the `Message` enum and
   the `update` arm.
2. `crates/ui-native/src/pages/` — one file per page (engine, midi,
   mix, mod_page, library, sys).

## Threading model

Five threads of significance:

1. **Audio thread** (cpal callback). Owns `BrumeEngine` via
   `Arc<parking_lot::Mutex<BrumeEngine>>`. Calls `process_block`.
   Real-time. Allocations forbidden.
2. **MIDI drainer** (`apps/brume-main/src/main.rs::midi_drainer_loop`).
   Drains `UiToEngine` into the engine when the audio callback
   isn't running (e.g. when Mac end of Meridian USB isn't pulling).
3. **UI thread** (iced). Drives the touchscreen. Sends `UiToEngine`
   messages and drains `EngineToUi` echoes.
4. **Persist workers** (`crates/ui-native/src/persist.rs`). Two
   debounced threads: one for `ControlMatrix`, one for `Transport`.
   Write `~/.brume/*.json` atomically.
5. **MIDI input thread** (`crates/midi-io`). midir-owned thread that
   parses ALSA events and forwards to the engine via the same
   `UiToEngine` channel.

Channels are bounded crossbeam channels with `try_send_or_log!` —
saturation drops messages silently. The
`EngineToUi::ParameterSnapshot` path (every ~1 s) reconciles any
state drift; see `crates/engine-runtime/src/engine.rs::process_block`
near the `% 200 == 0` branch.

## State of the codebase as of mid-2026

Recent significant work landed in this order:

1. Lua sandbox hardening (memory + time + parking_lot).
2. mlua 0.10 → 0.11.
3. Lua 5.4 → Lua 5.5 (with `LuaBuffer` userdata for FX audio paths).
4. Engine→UI `ParameterSnapshot` reconciliation against state drift.
5. A quality-skeptic review pass that closed a dozen WARNING-tier
   findings (persist error visibility, schema version gates, BPM
   clamp, denormal flush at filter asymptote sites, sine LUT for
   the Harmonic oscillator, install-path atomicity, etc.).

The codebase is in active development but stable enough that
`cargo test --workspace` is green at every commit on `main`.

## How to verify your environment is sane

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- \
  -D clippy::correctness -D clippy::suspicious -D clippy::perf \
  -A clippy::pedantic
```

If any of these fail before you've made changes, your toolchain is
off. Pinned: Rust 1.87 via `rust-toolchain.toml`. `rustup show` from
the repo root resolves it.

## When in doubt

- `CONTRIBUTING.md` for house style + PR flow.
- Inline `//!` doc-comments at crate roots and on every load-bearing
  type. Brume-specific patterns are documented at the call site
  more than in any single document.
- Open an issue if the change is non-trivial. The
  maintainer prefers issue-first alignment over surprise PRs.
