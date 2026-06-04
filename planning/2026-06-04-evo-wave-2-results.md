# EVO Wave 2 Results - FM Routing

Date: 2026-06-04
Branch: `evo-performance`
EVO run: `run_0000`

This wave followed the accepted Harmonic and shared-LUT changes from
`planning/2026-06-04-evo-wave-1-results.md`. It focused on FM routing because
the oscillator still scanned algorithm bitmasks inside the per-sample operator
loop, and that matched the external feedback about branch-heavy control flow.

## Frontier

Current best path:

```text
exp_0002 baseline wiring check after EVO result-file fix
  -> exp_0003 use sine LUT in phase oscillator waveform helpers
  -> exp_0004 avoid rem_euclid in harmonic phase advance
  -> exp_0008 precompute FM algorithm routing schedules
```

Scores:

| Experiment | Status | Score | Delta vs parent | Result |
| --- | --- | ---: | ---: | --- |
| `exp_0004` | committed | 2281.670 | n/a | Prior frontier |
| `exp_0008` | committed | 2226.156 | -55.514 | Integrated as `engine: precompute FM routing schedules` |
| `exp_0009` | discarded | n/a | n/a | Operator-level cache did not beat `exp_0008` |

`exp_0008` is the largest accepted improvement so far in `run_0000`.

## Accepted Change

### Compile-time FM routing schedules

`exp_0008` keeps the existing `ALGORITHMS` table as the source of truth, but
builds a fixed `ALGORITHM_SCHEDULES` table at compile time. The audio-rate path
now reads compact per-operator modulator lists and carrier lists instead of
rescanning six-bit masks and branching on every bit for every operator.

Why it was accepted:

- It preserves the data-driven algorithm model.
- It does not allocate in the audio path; schedules are fixed arrays.
- It removes branch-heavy mask scanning from the hot path.
- It adds a unit test that reconstructs every schedule back into the original
  `Algorithm` masks, so future table edits cannot silently desynchronize the
  schedule.
- `cargo test -p brume-engine-runtime --locked --no-fail-fast` passed before
  integration.
- `evo run --check exp_0008` passed with score `2228.849`.
- `evo run exp_0008` committed with score `2226.156`.

## Rejected Change

### Per-sample operator-level cache

`exp_0009` cached the six smoothed operator levels into a stack array once per
sample, then used that array during modulation and carrier summing. It was
behavior-preserving, but did not beat the `exp_0008` parent:

```text
preliminary score: 2242.663
EVO check score:   2233.754
parent score:      2226.156
```

The likely reason is that the extra stack array initialization does not pay for
itself once the routing schedule already removes most mask-scan overhead.

## Integrated Branch Validation

After integrating `exp_0008` onto `evo-performance`:

```text
./scripts/evo/gate-engine-runtime-fast.sh
  passed, including render_bench behavior gate for 14 scenarios

./scripts/evo/score-engine-runtime.sh
  2265.091 mean ns/frame
  2239.181 mean ns/frame

BRUME_EVO_SCORE_BLOCK_SIZE=32 \
BRUME_EVO_SCORE_BLOCKS=16000 \
BRUME_EVO_SCORE_WARMUP_BLOCKS=2048 \
./scripts/evo/score-engine-runtime.sh
  2276.115 mean ns/frame
```

The two default score attempts show the same local benchmark noise seen in wave
1. The accepted EVO score and branch-local rerun remain comfortably below the
pre-FM frontier, but CM5 validation is still required before release or merge.

## Next Candidates

1. Re-run a longer confirmation score when comparing future marginal changes,
   especially if the expected win is below roughly `20 ns/frame`.
2. Return to Granular with a larger structural candidate rather than tiny math
   hoists: fixed-size active-grain index management or a shape-specific envelope
   lookup are the next plausible experiments.
3. Consider a Harmonic scan-coefficient strategy only if its behavior drift can
   be measured with a stricter checksum or spectral gate.
