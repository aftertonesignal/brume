# Contributing to Brume

Thank you for your interest. Brume is a hobby project: a four-part
synthesizer that runs on a Raspberry Pi Compute Module 5 with a
touchscreen UI, a Lua scripting layer, and a USB audio and MIDI
bridge to a host computer. The project draws on synthesizer and
circuit-design traditions from the modular and analog era, along
with the more recent culture of open, scriptable hardware
instruments. It aims to be a welcoming place to contribute, in the
spirit of those communities.

- The project ships under **GPL-3.0-only**, and contributions land
  under the same license without a CLA or copyright reassignment.
- There is no commercial pressure behind the project. The maintainer
  (one person at the moment) reviews contributions on a best-effort
  basis, with no committed review timeline.
- If you're unsure whether something is in scope, opening an issue
  first usually saves work on both sides. A short note like "I'm
  thinking about working on X, does that fit?" is enough.

This document is for outside contributors. The maintainer-side
release procedure lives in [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md).

## Ways to contribute

The single most valuable contribution to Brume right now is a clear
bug report from someone who actually hit the problem. Code is
welcome too, but the project asks one thing of would-be code
contributors: see [Submitting changes](#submitting-changes) for the
issue-first rule before you start writing.

The lanes below describe ways to help, ordered roughly by the
technical barrier to entry. Several require neither Rust nor
hardware.

**Try it and report back.** If you have a CM5 set up to run Brume,
play with it and open issues for what surprised, broke, or felt
wrong. Bug reports with clear reproduction steps are some of the
most valuable contributions the project gets.

**Write or share Lua scripts.** Brume embeds Lua 5.4 with a
norns-flavored API (`brume.param`, `brume.note`, `brume.fx`,
`brume.transport`, plus a coroutine-based `clock` and a screen-drawing
API). Scripts live in `~/brume/scripts/` on the device or in the
repo under `crates/scripting/scripts/`. A useful study, a generative
sketch, or a MIDI mapping for a controller you own would all be
welcome contributions, none of which require Rust knowledge to
write.

**Improve the docs.** [`INSTALL.md`](INSTALL.md), [`DEPLOY.md`](DEPLOY.md),
[`HARDWARE.md`](HARDWARE.md), and the per-engine guides will all benefit
from people who notice what's confusing on a fresh read.

**Fix bugs or add features in the engine.** Brume is a Rust workspace.
The DSP primitives are in `crates/dsp-core/`; engine voices and the
audio thread live in `crates/engine-runtime/`; the iced + wgpu UI is
in `crates/ui-native/`; the modulation + control plane is split
across `crates/modulation/`, `crates/control-model/`, and
`crates/midi-io/`. The companion CLI is in `apps/brumectl/`. Pick an
issue tagged `good first issue` if you want a smaller starting point,
or open one to propose larger work.

**Add a control-surface driver.** Brume has a tiered control-surface
model. Tier 4 is first-class: drivers ship as Rust modules in
`crates/ui-native/src/controllers/`, with the `nanoKONTROL2` driver
as the reference. If you own a controller that isn't supported, a
driver PR is welcome; please open an issue first to align on shape.

**Hardware notes.** Brume targets the Raspberry Pi CM5 on a specific
IO board, touchscreen, and (eventually) DAC HAT combination. If
you've gotten Brume running on a different IO board, a different
display, or a different audio HAT (still on a CM5), notes in
[`HARDWARE.md`](HARDWARE.md) are valuable.

## Setting up a development environment

Brume is a CM5 instrument: the runtime that gets exercised, tested,
and supported is a Raspberry Pi Compute Module 5 running Pi OS Lite
Trixie with the audio, display, and USB-gadget configuration this
project provides. The development workflow assumes you have one.

If you have a CM5 already set up (per [`INSTALL.md`](INSTALL.md)),
the iteration loop lives in [`DEPLOY.md`](DEPLOY.md): rsync the
source over SSH, cross-build on the device, and restart
`brume.service`. Round trips are measured in seconds.

If you're contributing code without owning a CM5, the workspace
builds and its unit tests run anywhere with Rust 1.87 or later and
the build closure listed below. That path lets you validate
compilation and pass `cargo test --workspace` from your laptop, but
it is not a way to actually run Brume. The engine, UI, and audio
chain may load on a desktop Linux box, although that runtime is an
experimental compatibility surface rather than a supported user
path. Code-only contributions are welcome; expect review feedback
that exercises the change on a CM5 before merge.

`brumectl` (the Mac-side companion CLI) is the exception: it builds
and runs natively on macOS and Linux desktops, since that is the
host side of the user story.

### Prerequisites

- **Rust 1.87 or later.** Pinned via `rust-toolchain.toml`; `rustup
  show` from the repo root resolves it.
- **System libraries** (Linux): ALSA. On Debian / Ubuntu / Pi OS:
  ```
  sudo apt install -y pkg-config libasound2-dev
  ```
  On macOS, `brumectl` development needs no extra system libraries
  beyond Xcode Command Line Tools.

### Compile + test (no hardware required)

```
git clone git@github.com:aftertonesignal/brume.git
cd brume
cargo build --workspace
cargo test --workspace
```

Both should succeed end-to-end on macOS or any reasonably current
Linux distribution. CI runs the same checks on every PR. To work on
`brumectl` specifically, `cargo build -p brumectl` is enough.

### Iterate on a CM5

The full picture (sync source, cross-build on the device, restart
the service) lives in [`DEPLOY.md`](DEPLOY.md), which is the place
to go once changes are ready to be heard.

## House style

Brume tries to read like a careful, idiomatic Rust codebase. The
existing code is the best reference; the conventions below are the
ones most worth highlighting.

- **`cargo fmt --all` is enforced by CI.** Run it before opening a PR.
  The formatting is whatever stable rustfmt produces; no project-
  specific overrides.
- **Clippy is currently advisory, not gating.** The workspace has
  `clippy::pedantic = "warn"` configured, which produces a substantial
  warning set against the current codebase. Don't add new lint
  regressions; cleanup of the existing backlog is welcome but is its
  own workstream.
- **`unsafe_code = "forbid"`** at the workspace level. There is no
  unsafe code in the tree and there should not be any. A real reason
  to add some is a discussion, not a PR.
- **Atomic commits.** One logical change per commit; don't batch
  unrelated work, since the maintainer prefers reviewing five
  focused commits over one sprawling one. If you find yourself
  fixing something tangential during a feature commit, that fix
  belongs in a separate commit.
- **CHANGELOG.md `[Unreleased]`.** Every user-observable change adds
  a bullet under `Added`, `Changed`, `Fixed`, `Removed`, `Deprecated`,
  or `Security` in the `[Unreleased]` block at the top of
  [`CHANGELOG.md`](CHANGELOG.md). The release script promotes that
  block to a versioned section at cut time. Internal refactors that
  users won't notice can skip the changelog entry.
- **SPDX headers.** New `.rs` files start with:
  ```
  // SPDX-License-Identifier: GPL-3.0-only
  // Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
  ```
  Existing files already carry this header; preserve it on edits.
- **Doc comments are for the *why*, not the *what*.** Identifiers
  carry meaning; comments should explain hidden constraints, subtle
  invariants, or workarounds for specific bugs. A `//` describing
  what the next line obviously does is noise.

## Submitting changes

**Non-trivial code changes require a referenced issue and explicit
maintainer alignment before the PR.** Trivial fixes (typo, broken
link, obvious one-line bug) can skip this. Everything else: open
an issue, get a "sounds good" from the maintainer, then implement.
PRs without a referenced issue will be closed without review.

This rule is here for one reason: AI-generated PRs are cheap, and
an unfiltered review queue eats the time of a single-maintainer
project. The issue-first step is the filter; it lets review time go
to contributions whose direction is already agreed.

Once you have alignment, the flow is conventional GitHub:

1. Fork the repo (or push a branch directly if you have access).
2. Create a feature branch from `main`. Branch naming is up to you.
3. Make your changes. Keep commits atomic; write commit messages that
   explain *why*, not just *what* (`git log` is the most-read piece
   of project history).
4. Run the local equivalent of CI before pushing:
   ```
   cargo fmt --all -- --check
   cargo test --workspace --locked
   ```
5. Open a pull request against `main`, referencing the aligned issue
   in the description (e.g. "closes #42"). CI must be green for a
   merge.
6. Update `CHANGELOG.md` `[Unreleased]` if your change is user-
   observable.

There is **no DCO sign-off requirement and no CLA.** Your commits
are your authorship statement under GPL-3.0, with no further
paperwork required.

The maintainer reviews when they can; the project doesn't commit
to a review SLA. Friendly follow-ups after a couple of weeks are
fine, and a tag on the PR ("@maintainer when you have a moment")
helps surface things that fell off the queue.

## Reporting bugs

Open a [GitHub issue](https://github.com/aftertonesignal/brume/issues). The
most useful bug reports include:

- A short title describing the symptom, not the suspected cause.
- What you did, what you expected to happen, what actually happened.
- Brume version (footer status bar, SYS page, or `brumectl --version`).
- Hardware: CM5 + which IO board + which screen, or "Linux desktop"
  with distribution + kernel.
- Logs if relevant: `journalctl --user -u brume.service` on the CM5,
  or `/tmp/brume.log` if running ad-hoc.
- A screenshot or a short video for UI-shaped bugs.

A small, focused reproduction is the most useful kind of report.
"Open Brume, load patch X, turn knob Y fully clockwise, and audio
drops" is much easier to act on than "audio sometimes drops."

## Discussing design

For anything more open-ended than a bug or a small change (feature
proposals, architectural questions, "should we restructure X"
threads), open an issue and label it `discussion`. The issue tracker
is the project's design forum for now. GitHub Discussions may get
enabled later if there's enough traffic to justify a second surface.

For private correspondence (security disclosures, anything that
shouldn't be public), email `hello@aftertone.co`.

## Versioning, releases, and "when will my change ship"

Brume follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-1.0:

- `0.X.0` ships anything user-observable.
- `0.X.Y` ships bug fixes only.

The current and historical surface is in
[`CHANGELOG.md`](CHANGELOG.md), and the maintainer-side release
procedure is in [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md).
Releases happen when there's enough accumulated work to be worth
cutting one; there is no fixed cadence.

Your merged change lands in the next release. If a release is
imminent and you want to confirm whether your change is included,
ask in the PR or in an issue.

## Acknowledgements

Brume owes a lot to the modular and analog synth tradition, the
open and scriptable instrument culture that followed, and the
Rust audio crates that did the heavy lifting underneath.
