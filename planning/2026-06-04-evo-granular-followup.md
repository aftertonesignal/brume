# EVO Granular Follow-up - Gaussian Lookup

Date: 2026-06-04
Branch: `evo-performance`
EVO run: `run_0000`

This note records a Granular follow-up attempted after the FM routing schedule
win in `planning/2026-06-04-evo-wave-2-results.md`.

## Experiment

```text
exp_0010 use granular Gaussian envelope lookup table
parent: exp_0008 precompute FM algorithm routing schedules
```

The hypothesis was that the granular oscillator's per-grain Gaussian envelope
`exp` call was hot enough to replace with a fixed-size lookup table stored on
`GranularOscillator`.

Implementation shape in the experiment:

- Added a fixed 257-entry Gaussian envelope table to `GranularOscillator`.
- Built the table in `GranularOscillator::new`, not in the audio loop.
- Used linear interpolation for the Gaussian component of the grain envelope.
- Added a unit test that compared lookup output against the exact Gaussian
  formula within `0.001`.

## Result

The experiment compiled and passed engine-runtime tests:

```text
cargo test -p brume-engine-runtime --locked --no-fail-fast
  69 passed
```

Preliminary score:

```text
2240.412 mean ns/frame
```

Parent score:

```text
2226.156 mean ns/frame
```

The lookup-table candidate was discarded without formal EVO attempts because it
did not improve the current frontier and also introduced envelope approximation
risk. Future granular work should prioritize structural changes or stronger
behavior gates before revisiting envelope approximations.
