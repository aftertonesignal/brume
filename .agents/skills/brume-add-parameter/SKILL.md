---
name: brume-add-parameter
description: >
  End-to-end procedure for adding a new ParameterId to Brume — the
  enum, the per-engine dispatch, the Voice/Part default, the UI
  slider exposure, the persistence path, and the smoothing
  decision. The single most repeated extension pattern in the
  codebase. Triggers on "add a parameter", "add a new parameter",
  "new ParameterId", "expose a control", "add a control", "wire up
  a parameter", "make this controllable", "expose this in the UI",
  "add a knob", "new knob", "add a slider", "controllable from
  MIDI".
---

# Adding a new parameter

Every controllable value in Brume — filter cutoff, FM index,
modulation depth, scan center, reverb size — flows through the
same end-to-end pattern: a `ParameterId` enum variant, a per-engine
`set_parameter` arm, a default in the relevant `Voice` or `Part`,
a UI slider, and a persistence-aware path through the engine
state map. This skill is the procedure for adding one without
missing a step.

## State machine

```
1. Decide the parameter's home (which engine? which subsystem?)
   ↓
2. Add the ParameterId variant
   ↓
3. Wire set_parameter dispatch on the owner
   ↓
4. Add a default in the Voice/Part initialization
   ↓
5. Decide on smoothing
   ↓
6. Expose in the UI (slider + tab page)
   ↓
7. Verify the engine→UI echo + the snapshot path
   ↓
8. Add a test asserting the dispatch lands
```

## Step 1 — Decide the home

Brume's parameters are scoped to one of:

- **A specific oscillator engine** (FmIndex, HarmonicLevel1,
  Symmetry, GrainSize). These live on the relevant `Oscillator`
  struct and only apply when that engine is the part's active
  mode.
- **The shared filter / amp section** (FilterCutoff, FilterResonance,
  AttackTime, MasterVolume). These apply across all four engines.
- **An FX slot** (DelayTime, ReverbSize, ChorusRate). These flow
  through `UiToEngine::SetFxParam` instead of `SetParameter` and
  are out of scope for this skill — see the FX chain crate.

If you're adding a parameter that conceptually belongs to multiple
engines but with different ranges or meanings, prefer separate
`ParameterId` variants per engine over one ambiguous variant.
Brume already does this (e.g., `FmIndex` for the FM engine,
`HarmonicFmDepth` for the Harmonic engine's internal modulator).

## Step 2 — Add the ParameterId variant

File: `crates/common/src/lib.rs`.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterId {
    // ... existing ...
    YourNewParameter,
}
```

`ParameterId` derives include `Hash` and `Eq` (for `param_state`
HashMap keying) and `Serialize`/`Deserialize` (for patch
round-tripping). Don't drop any.

If the enum has a `FromStr` / `Display` impl (it does today —
search for `impl std::str::FromStr for ParameterId`), add an arm
for the new variant. The Lua scripting layer parses
`ParameterId` from a string at the API boundary; an unmatched
arm makes the parameter invisible to scripts.

## Step 3 — Wire set_parameter dispatch on the owner

The owner is whichever `Oscillator`, `Filter`, or `Voice` field
will hold the value.

For an oscillator-scoped parameter, edit the relevant
`set_parameter` impl:

```rust
// crates/engine-runtime/src/fm_oscillator.rs (or harmonic_oscillator.rs, etc.)
pub fn set_parameter(&mut self, id: ParameterId, value: f32) -> bool {
    match id {
        // ... existing ...
        ParameterId::YourNewParameter => {
            self.your_field = value.clamp(MIN, MAX);
            true
        }
        _ => false,
    }
}
```

The `bool` return is load-bearing: `false` means "not my parameter,"
and `Voice::set_parameter` uses it to fall through to the next
candidate. Returning `true` from a parameter that didn't actually
apply will silently corrupt the engine state map.

For a Voice-scoped parameter (filter, amp), edit
`crates/engine-runtime/src/voice.rs::set_parameter` directly.

For a Part-scoped parameter that affects the engine's choice of
oscillator mode or polyphony, edit
`crates/engine-runtime/src/part.rs`.

## Step 4 — Add a default in Voice/Part initialization

The engine's authoritative state map (`BrumeEngine::param_state`)
only fills as `set_parameter` is called. The UI hydrates its
`param_values` HashMap with per-tab defaults at startup
(`crates/ui-native/src/lib.rs::run`) — both sides need to agree on
the default.

For a parameter that should have a non-zero default:

1. Initialize the field in the relevant `new()` constructor
   (e.g., `FmOscillator::new`, `Voice::new`).
2. Add an entry in the per-tab default table the UI reads at
   startup. Search for `tab.default_values` or grep
   `param_values.insert` near the `run` function in
   `crates/ui-native/src/lib.rs`.

## Step 5 — Decide on smoothing

Continuous-value parameters that the user sweeps live (filter
cutoff, FM index, master volume) should run through a `Smoother`
to avoid zipper noise. Discrete parameters (algorithm selection,
oscillator mode) should not — they're step changes that the
ear hears as immediate transitions, and a smoother makes them
sound mushy.

Smoothing pattern:

```rust
// On the struct:
pub struct FmOscillator {
    fm_index: Smoother,
    // ...
}

// In set_parameter:
ParameterId::FmIndex => {
    self.fm_index.set_target(value.clamp(0.0, 10.0));
    true
}

// In the per-sample inner loop:
let idx = self.fm_index.process();
```

`Smoother::new(initial, smoothing_ms, sample_rate)`. Typical
smoothing time is 5-20 ms. See `dsp-core::Smoother` for the API.

## Step 6 — Expose in the UI

The UI lives in `crates/ui-native/`. The four engines have
dedicated tabs in `crates/ui-native/src/pages/engine.rs`. Each
tab carries a list of `ParamSpec` entries describing how to
render its sliders.

Add a `ParamSpec` for your new parameter to the relevant tab.
Search for the existing parameter that's structurally closest
(same engine, similar range) and use it as a template. The spec
includes:

- The `ParameterId`.
- Display name + units.
- Range (min, max).
- Default for slider initial position.
- Optional formatter for the current-value readout.

After adding the spec, the UI's existing slider rendering picks
it up automatically. No new `Message` variant is needed — the
existing `Message::SliderChanged { binding, value }` flow handles
all `ParamSpec`-driven controls.

## Step 7 — Verify the echo + snapshot path

The engine echoes every accepted `set_parameter` back to the UI
via `EngineToUi::ParameterChanged`. The UI inserts into its
`param_values` HashMap on receipt. If your new parameter doesn't
appear in the UI when you twist the slider, walk these:

1. Confirm `set_parameter` returns `true` for your variant (a
   silent `false` skips the echo).
2. Confirm the per-tab default table includes the new
   `ParameterId` so the slider has a starting position.
3. Confirm the `ParamSpec` is in the right tab's spec list.

The engine also emits `EngineToUi::ParameterSnapshot` ~1 Hz.
Your parameter, once `SetParameter` has fired at least once,
will appear in the snapshot. If it doesn't, the dispatch is
silently rejecting it — likely a missing match arm.

## Step 8 — Add a test

A simple round-trip test pinning the dispatch belongs in the
oscillator's tests module:

```rust
#[test]
fn your_new_parameter_round_trips() {
    let mut osc = FmOscillator::new(48000.0);
    assert!(osc.set_parameter(ParameterId::YourNewParameter, 3.5));
    // assert state changed in some observable way
}
```

For UI-side coverage, the existing `nanokontrol2` controller tests
in `crates/ui-native/src/controllers/nanokontrol2.rs` are a good
template if your parameter is supposed to be reachable from a
control surface.

## Anti-patterns

- **Don't** add a `ParameterId` variant without a matching
  `set_parameter` arm somewhere. The dispatch is exhaustive —
  silent fall-through means the parameter is dead on arrival.
- **Don't** add UI-side state that mirrors `param_values` for the
  same parameter. The HashMap is the single source of truth on
  the UI side; `param_values.insert((part, id), value)` is the
  one write path.
- **Don't** skip the smoothing decision. Values that wobble
  across the audio band (filter cutoff at audio rate) without
  smoothing produce zipper noise that's hard to debug after the
  fact.
- **Don't** persist the new parameter via a side channel. The
  engine state map + the snapshot path covers it; adding a
  separate save site for one parameter is a divergence trap.

## When to skip parts of this procedure

The whole flow assumes a parameter that's controllable from the
UI and persisted across restarts — the common case. For an
internal-only value (e.g., a debug threshold or a temporary
diagnostic), you can skip the UI exposure and the persistence
hookup. Add the variant, dispatch, and default; don't list it
in any tab spec.
