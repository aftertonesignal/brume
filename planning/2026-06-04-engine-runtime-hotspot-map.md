# Engine Runtime Hotspot Map

Date: 2026-06-04
Branch: `evo-performance`

This note follows the baseline harness in
`planning/2026-06-04-evo-performance-planning.md`. No EVO operation has run
yet. The purpose here is to turn the code read and benchmark results into a
ranked set of candidate analyses and simulations.

## Current Evidence

The benchmark harness now measures three layers:

- Direct `BrumeVoice` scenarios, isolating oscillator + voice chain cost.
- Direct `Part` scenarios, measuring six-voice part rendering, modulation, and
  headroom behavior.
- Full `BrumeEngine` scenarios, including master/send buses, FX, limiter, UI
  side state maintenance, and stereo/8-channel emission.

The latest 512-frame run on this isolated Linux machine reported:

| Group | Aggregate ns/frame |
| --- | ---: |
| Direct voice | 318.025 |
| Direct part | 1905.324 |
| Full engine | 3859.781 |
| All scenarios | 2289.435 |

Most one-part engine scenarios remain close to the corresponding direct part
scenario. That means part rendering dominates those rows; final stereo emission
and master-path overhead are not the first target.

The full-engine stereo and 8-channel scenarios remain close:

| Scenario | ns/frame |
| --- | ---: |
| `full_engine_stereo` | 7653.731 |
| `full_engine_8ch` | 7681.839 |

On this host, final channel layout is not the bottleneck. CM5 confirmation is
still required before treating this as hardware truth.

## Block Size Sweep

The external feedback specifically called out small-block processing, so the
harness was run at 8, 16, 32, 64, 256, and 512 frames. The runs used different
total frame counts to keep turnaround quick; treat these as directional rather
than statistically rigorous.

| Block size | Voice mean ns/frame | Part mean ns/frame | Engine mean ns/frame | All mean ns/frame |
| ---: | ---: | ---: | ---: | ---: |
| 8 | 314.004 | 1887.420 | 3962.371 | 2327.137 |
| 16 | 314.024 | 1908.583 | 3894.007 | 2303.891 |
| 32 | 321.527 | 1924.283 | 3900.598 | 2313.345 |
| 64 | 315.050 | 1971.433 | 3887.841 | 2319.498 |
| 256 | 316.141 | 1930.393 | 3865.045 | 2298.315 |
| 512 | 318.025 | 1905.324 | 3859.781 | 2289.435 |

Initial read:

- Per-frame cost is mostly stable across these block sizes.
- The current scalar implementation is dominated by per-sample voice work, not
  per-block setup.
- A future block-processing refactor may still create SIMD and coefficient
  caching opportunities, but it should not be the first change unless EVO or a
  profiler shows a specific block-level win.

## Hot Path Shape

### Granular oscillator

Primary file: `crates/engine-runtime/src/granular_oscillator.rs`

Benchmark signal:

- Direct voice: `voice_granular_cloud` is the slowest voice scenario.
- Direct part: `part_granular_cloud` is the slowest part scenario.
- One-part engine: `granular_cloud_stereo` stays close to direct part cost.

Hot-loop observations:

- `GranularOscillator::process` advances ten smoothers every sample.
- It updates FM oscillator frequency and computes FM sample every sample.
- It checks spawn timing every sample and may call `spawn_grain`.
- It scans all 32 grain slots every sample, branching on `active`.
- Each active grain computes an envelope with `cos` and one or two `exp` calls.
- Each active grain calls `morph_wave`, which currently routes sine-morph paths
  through `f32::sin` rather than the shared sine LUT.
- The scenario intentionally uses dense settings: density 1.0, morph 0.72,
  scatter 0.65, drift 0.85, FM depth 2.4, grain shape 0.72.

First simulation candidates:

1. Measure a shared-LUT version of `morph_wave` or a granular-local variant
   that uses `sine_normalized` for sine components.
2. Measure an active-grain index list or compact active list to avoid scanning
   all 32 slots when fewer grains are active.
3. Measure a grain-envelope approximation or lookup path, with strict RMS,
   zero-crossing, and checksum/energy drift gates.
4. Evaluate whether FM oscillator frequency and sample work can be skipped when
   `fm_depth` is effectively zero; this is a patch-dependent fast path, not the
   current dense benchmark path.

Risk notes:

- Granular texture can change with small envelope, spawn, and random-walk
  differences. Any optimization here needs stronger behavior gates than simple
  non-silence.
- A compact active list must remain allocation-free and must handle stolen
  grains without stale indices.

### Harmonic oscillator

Primary file: `crates/engine-runtime/src/harmonic_oscillator.rs`

Benchmark signal:

- Direct voice: `voice_harmonic_scan` is the second-slowest voice scenario.
- Direct part: `part_harmonic_scan` is the second-slowest part scenario.

Hot-loop observations:

- `HarmonicOscillator::process` runs an 8-harmonic loop per sample.
- It advances eight control smoothers per sample before the harmonic loop.
- Per harmonic it computes inharmonic stretch with `sqrt`.
- It branches for Nyquist rejection.
- It computes scan amplitude with Gaussian `exp` unless width is near full.
- It branches on odd/even balance and morph/FM path selection.
- When morph is active, it calls `morph_wave`, which uses `f32::sin` for the
  sine side of the morph.
- It wraps each harmonic phase with `rem_euclid`.
- The current benchmark intentionally sets scan width 0.22, morph 0.65, FM
  depth 2.75, inharmonicity 0.8, and spread 0.45, so it exercises the expensive
  path rather than the clean sine path.

First simulation candidates:

1. Replace `morph_wave` sine calls with the shared sine LUT and measure across
   Harmonic, Granular, and Timbral scenarios.
2. Precompute per-harmonic constants for harmonic number, normalized index,
   odd/even class, and base level to reduce branch and conversion work.
3. Cache scan coefficients at a small control-rate block size when scan center
   and width are stable enough; compare exact per-sample smoothing against
   8/16/32-sample coefficient updates.
4. Consider a no-morph/no-FM fast path for simple harmonic patches, but keep the
   first benchmark centered on the dense scan path.

Risk notes:

- Scan and inharmonic coefficients are part of the voice character. Block-rate
  coefficient updates need explicit drift gates, not only performance gates.
- `rem_euclid` replacement is only safe if negative phase is impossible in the
  path being changed.

### FM oscillator

Primary file: `crates/engine-runtime/src/fm_oscillator.rs`

Benchmark signal:

- Direct FM voice is cheaper than Harmonic and Granular.
- The six-voice `part_fm_dense` row is materially heavier than Timbral part
  rendering because all six voices are active and each voice runs six operators
  twice per output sample through the `Voice` oversampling path.

Hot-loop observations:

- `FmOscillator::process` refreshes six ratio smoothers and six level
  smoothers every sample.
- It evaluates operators from OP6 down to OP1.
- For each operator, it scans a six-bit modulation mask and branches per bit.
- It separately scans the carrier mask after computing all operators.
- Algorithm data is table-driven and intentionally future-friendly for Lua/user
  algorithms.

First simulation candidates:

1. Precompute per-algorithm routing schedules and carrier lists, then compare
   against the bit-mask scanner.
2. Keep the current table as the source of truth and generate fixed compact
   schedules at startup or compile time, preserving user-defined algorithm
   extensibility as a separate design question.
3. Measure whether carrier-list summing moves the needle before considering
   specialized code paths for common algorithms.

Risk notes:

- FM exact samples may change if operator order, self-feedback timing, or level
  lookup timing changes. Gates need to catch algorithm-specific drift.
- The table-driven algorithm model is part of the design. Optimization should
  not casually remove that extensibility.

### Timbral oscillator

Primary file: `crates/engine-runtime/src/timbral_oscillator.rs`

Benchmark signal:

- Timbral is currently the cheapest direct voice scenario.
- Its dense part scenario remains cheaper than FM, Harmonic, and Granular.

Hot-loop observations:

- The path advances six smoothers and runs a triangle-core shape.
- FM, feedback, sub, and fold-stage branches are active in the dense scenario.
- The stage loop is bounded to one through four iterations.

First simulation candidates:

1. Leave this lower priority until Granular/Harmonic/FM candidates are tested.
2. If needed, specialize one-to-four fold stages and compare against the loop.
3. Consider sine-LUT effects if `PhaseOscillator::sine` or `morph_wave` changes
   are already being measured globally.

## Cross-Cutting Observations

- `brume_dsp_core::sine_normalized` already exists and is used by FM and the
  non-morph Harmonic sine path.
- `PhaseOscillator::sine` and `morph_wave` still call `f32::sin`.
- `Smoother::process` has a branch for asymmetric smoothing on every call. Many
  smoothers are symmetric; this is a possible low-level target, but it is broad
  and should wait until oscillator-specific targets are measured.
- `Part::process_buffer` applies modulation once per buffer, then renders all
  voices per frame. The block-size sweep does not point to this as the first
  bottleneck.
- `BrumeEngine::process_block` uses stack scratch arrays per chunk and
  allocation-free steady-state message/UI paths, except for the documented
  periodic parameter snapshot.

## Recommended EVO Starting Targets

Use the benchmark harness as a guardrail but start with small, constrained
mutations. Ranked order:

1. `morph_wave` / `PhaseOscillator::sine` LUT substitution or local
   equivalents, because it touches expensive math used by Harmonic and
   Granular while preserving broad structure.
2. Harmonic coefficient precomputation for fixed per-harmonic constants and
   scan/morph control-rate experiments.
3. Granular active-list or envelope approximation experiments, with strict
   behavior gates.
4. FM routing schedule precomputation.
5. Only after scalar wins are measured: explicit small-block render APIs for
   8/16/32-sample control-rate updates and SIMD-friendly buffers.

## Verification Expectations

Fast local gate for any code candidate:

```bash
cargo fmt --all -- --check
cargo test -p brume-dsp-core --locked --no-fail-fast
cargo test -p brume-engine-runtime --locked --no-fail-fast
cargo test -p brume-fx-chain --locked --no-fail-fast
cargo run --quiet -p brume-engine-runtime --example render_bench --release
```

Before accepting a performance result:

- Repeat a 512-frame run and at least one 16- or 32-frame run.
- Compare per-scenario peak, RMS, zero crossings, and checksum/energy drift.
- Run clippy with the project performance/correctness gate.
- Validate on the reference CM5 once the macOS-attached board is available.
