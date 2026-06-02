# Brume — agent onboarding

This file is the entry point for AI coding agents (Claude Code, Codex,
Cursor, OpenCode, etc.) working on Brume. Human contributors should
start at [`CONTRIBUTING.md`](CONTRIBUTING.md), which carries the
canonical house style, PR flow, and project scope. This document
adds the things an agent specifically needs at every interaction:
load-bearing invariants, a crate map, and an index of procedural
skills.

## What Brume is

A four-part multi-timbral synthesizer for the Raspberry Pi Compute
Module 5. Four engines (FM, Harmonic, Timbral, Granular), six voices
per part, one filter, one effects chain, a Lua scripting layer, a
10.1-inch touchscreen UI in iced + wgpu, and a USB bridge that
presents Brume to a host computer as a class-compliant audio + MIDI
device. Hobby project, GPL-3.0-only, no commercial pressure.

## Load-bearing invariants

These are absolute. Every change must respect them; CI enforces some
but not all.

- **`unsafe_code = "forbid"` workspace-wide.** No `unsafe` blocks.
  CI fails the build if you add one.
- **No allocations on the audio thread.** `process_block` and its
  callees must not call the allocator. Pre-allocate, reuse buffers,
  use `parking_lot::Mutex` (no poisoning) over `std::sync::Mutex`.
- **No syscalls on the audio thread.** That includes `eprintln!` /
  `println!`, file I/O, anything that can block on journald or a
  slow tty. If you need to surface state, use the engine→UI channel.
- **Atomic commits.** One logical change per commit. Don't batch
  unrelated work — five focused commits beat one sprawl. CHANGELOG
  bullet on every user-observable change. See `CONTRIBUTING.md`
  §"House style" for the full set.
- **Schema-version every persisted format.** `Patch`, `Perf`, and
  `ControlMatrix` all carry a `version: u32` and reject futures
  newer than this build supports. See the
  [`brume-schema-version-bump`](.agents/skills/brume-schema-version-bump/SKILL.md)
  skill before touching any of them.

## Workspace map

```
crates/
  app-protocol/     UiToEngine + EngineToUi messages on the bridge
  audio-io/         cpal/ALSA backend, output device enumeration
  common/           ParameterId enum, IPC macros, OscillatorMode
  control-model/    MIDI ChannelMap, ControlMatrix, CC bindings
  dsp-core/         filters, delays, saturation, sine LUT, denormal
                    flush, smoothers — pure DSP primitives
  engine-runtime/   BrumeEngine, Voice, Part, transport, FM/Harmonic/
                    Timbral/Granular oscillators — the audio thread
  fx-chain/         Saturator/Delay/Chorus/Reverb chain + dattorro
  midi-io/          ALSA MIDI input thread, drainer, activity tracker
  modulation/       LFO/Seq sources, ModRouter, transition shapes
  patch-store/      ~/.brume/library/ on-disk Patch/Perf serialization
  platform-rpi/     Pi-specific bring-up + SCHED_FIFO elevation
  scripting/        sandboxed Lua 5.5 VM, brume API, FX userdata
  settings/         settings.json (output device id, etc.)
  ui-native/        iced + wgpu touchscreen UI, persist workers,
                    control surface drivers (nanoKONTROL2, LCXL3)
apps/
  brume-main/       the synthesizer process
  brumectl/         host-side companion CLI (macOS + Linux)
deploy/scripts/     CM5 systemd / USB gadget / labwc setup
.github/workflows/  CI (ci.yml) + release matrix (release.yml)
```

## Procedural skills

Detailed playbooks live in [`.agents/skills/`](.agents/skills/). Claude
Code loads these on intent (the `description` field in each
`SKILL.md` carries trigger phrases). Other agents can load them
directly when the user names a relevant scenario.

- **[`brume-getting-started`](.agents/skills/brume-getting-started/SKILL.md)** —
  orient a fresh agent: this file in expanded form, plus pointers to
  the right reading order for any contribution scope.
- **[`brume-audio-thread-review`](.agents/skills/brume-audio-thread-review/SKILL.md)** —
  the checklist for any change touching `process_block` or its
  callees. Allocation, syscall, panic, denormal, and lock-discipline
  audit.
- **[`brume-add-parameter`](.agents/skills/brume-add-parameter/SKILL.md)** —
  end-to-end procedure for adding a new `ParameterId`: enum, dispatch,
  default, UI slider, persistence, smoothing.
- **[`brume-schema-version-bump`](.agents/skills/brume-schema-version-bump/SKILL.md)** —
  when to bump `SCHEMA_VERSION`, how to add a migration arm, the
  test pattern that pins the gate.
- **[`brume-lua-script`](.agents/skills/brume-lua-script/SKILL.md)** —
  scripting subsystem: sandbox bounds, brume API, FX vs control
  scripts, `LuaBuffer` userdata for audio paths.
- **[`brume-cm5-deploy`](.agents/skills/brume-cm5-deploy/SKILL.md)** —
  rsync, build-on-device, install to `/usr/bin/brume`,
  `systemctl restart` workflow. The atomic binary swap pattern.
- **[`atomic-commits`](.agents/skills/atomic-commits/SKILL.md)** —
  the commit-hygiene rule: when to split, when to bundle, what
  belongs in the message body.

The [`.agents/skills/README.md`](.agents/skills/README.md) documents
the skill format if you need to author a new one.

## Where else to look

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — house style, PR flow, scope,
  bug reports.
- [`INSTALL.md`](INSTALL.md) — first-time CM5 bring-up.
- [`DEPLOY.md`](DEPLOY.md) — iteration loop on a CM5.
- [`HARDWARE.md`](HARDWARE.md) — IO board, screen, audio HAT notes.
- [`CHANGELOG.md`](CHANGELOG.md) — every user-observable change. New
  entries go under `[Unreleased]`.

## What this file is not

`AGENTS.md` is the always-loaded floor, not an architecture deep-dive.
Per-subsystem detail belongs in skills (loaded on intent) or in
inline `//!` doc-comments at the top of each crate's `lib.rs`. If
you find yourself wanting to extend this file past ~150 lines,
consider whether the new material is a skill instead.
