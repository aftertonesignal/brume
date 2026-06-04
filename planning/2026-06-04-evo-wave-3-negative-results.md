# EVO Wave 3 Negative Results - Post-FM Frontier

Date: 2026-06-04
Branch: `evo-performance`
EVO run: `run_0000`

This note records the experiments attempted after `exp_0008` became the current
frontier. None beat the FM routing schedule result, so no production code was
integrated in this wave.

Current frontier:

```text
exp_0008 precompute FM algorithm routing schedules
score: 2226.156 mean ns/frame
```

## Discarded Experiments

### `exp_0011` - active granular grain slots

Hypothesis: track active granular slots with a `u32` mask and iterate only set
bits instead of scanning all 32 grain slots each sample.

Result:

```text
preliminary score: 2288.522
verbose rerun:     2276.487
parent score:      2226.156
```

The granular rows were slower in the verbose rerun. In the dense benchmark,
enough grain slots are active that bit iteration costs more than the simple
fixed scan. Do not retry this exact active-mask approach for the current dense
scenario.

### `exp_0012` - symmetric smoother fast path

Hypothesis: add `Smoother::process_symmetric()` and use it in oscillator
controls that are constructed with symmetric `Smoother::new`, avoiding the
rise-vs-fall branch in `Smoother::process`.

Result:

```text
cargo test -p brume-dsp-core --locked --no-fail-fast
  63 passed

cargo test -p brume-engine-runtime --locked --no-fail-fast
  68 passed

preliminary score: 2310.450
parent score:      2226.156
```

The method split likely worsened code generation or failed to remove meaningful
bottleneck work. The broader change is not worth carrying.

### `exp_0013` - Timbral wavefolder stage unroll

Hypothesis: replace the small dynamic loop over one to four Timbral
wavefolder stages with a `match` that directly calls the fixed number of
`triangle_fold` stages.

Result:

```text
preliminary score: 2259.083
EVO check score:   2233.842
parent score:      2226.156
```

The change was behavior-preserving and passed tests, but did not clear the
frontier. Timbral remains lower priority than Granular and FM in this harness.

### `exp_0014` - granular pitch-scatter fast exp2

Hypothesis: replace spawn-time `(2.0_f32).powf(x)` with
`brume_dsp_core::utility::fast_exp2(x)` for granular pitch scatter.

Result:

```text
cargo test -p brume-engine-runtime --locked --no-fail-fast
  68 passed

preliminary score: 2258.222
parent score:      2226.156
```

Spawn-time `powf` is not enough of an aggregate bottleneck in the current
benchmark. The approximation also changes grain pitch slightly, so there is no
reason to keep it without a measurable win.

## Read

The current benchmark has now rejected several plausible micro-optimizations
after the FM routing schedule win. The next likely useful work is not another
tiny arithmetic substitution. Better next candidates:

1. Add a longer/noise-reduced confirmation score mode for marginal candidates.
2. Profile or instrument Granular to measure active grain count, envelope time,
   and spawn-time cost before choosing another structural change.
3. Explore block-level rendering only with a very narrow target, such as
   control-rate updates for Harmonic scan coefficients, and add stricter
   behavior drift checks before accepting it.
