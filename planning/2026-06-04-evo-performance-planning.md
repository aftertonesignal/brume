# EVO Performance Planning - Voice Engine Simulation

Date: 2026-06-04
Branch: `evo-performance`
Repo: `git@github.com:aftertonesignal/brume.git`

## Context

This branch exists to explore performance work without disturbing `main`.
Current development is happening on an isolated Linux machine, not on the
reference Raspberry Pi CM5 board. The reference CM5 is attached to the macOS
machine, so Linux results here should be treated as relative analysis and
simulation evidence, not final hardware validation.

No EVO discovery or optimization run has started yet. The near-term goal is to
understand the codebase, identify focused benchmark targets, and define gates
that keep generated changes from trading sound behavior for speed.

## External Feedback

Post-launch feedback pointed at the voice engines as the likely first area of
performance headroom. The key claims were:

- The voice engines are clear but closer to textbook/reference
  implementations than aggressively optimized synth code.
- Low-hanging fruit likely includes organizing control flow to reduce
  branching and organizing data references with CPU cache behavior in mind.
- Larger gains may come from small-block processing, such as 8, 16, or 32
  samples at a time, and using vector instructions where practical.

This matches the local source read: the main opportunity is not broad app
optimization. It is the audio hot path in `crates/engine-runtime`.

## Relevant Constraints

Load-bearing constraints from `AGENTS.md` still apply:

- Workspace-wide `unsafe_code = "forbid"`.
- No allocations on the audio thread.
- No syscalls on the audio thread.
- Changes touching `process_block` or callees need audio-thread review.

For this branch, performance changes also need behavior gates. EVO will optimize
the metric it sees, so any benchmark must make it difficult to "win" by muting
voices, skipping modulation, flattening spectra, or removing tails.

## Primary Target Surface

Start with `crates/engine-runtime`:

- `src/fm_oscillator.rs`
- `src/harmonic_oscillator.rs`
- `src/timbral_oscillator.rs`
- `src/granular_oscillator.rs`
- `src/voice.rs`
- `src/part.rs`
- `src/engine.rs`

These files contain the per-sample and per-block render path:

- `BrumeEngine::process_block` renders parts, builds master and send buses,
  runs FX, limits output, and writes stereo or 8-channel buffers.
- `Part::process_buffer` advances modulation, applies offsets, and mixes active
  voices per frame.
- `BrumeVoice::process` runs oscillator core, halfband decimation, filter,
  envelopes, velocity smoothing, and DC blocking.
- Each oscillator mode has branch-heavy and math-heavy per-sample logic.

## Initial Hotspot Hypotheses

### FM oscillator

The FM engine evaluates algorithm masks per sample:

- Refresh six ratio smoothers and six level smoothers.
- Walk operators OP6 to OP1.
- For each operator, branch on routing masks and scan possible modulators.
- Sum carriers by checking carrier bits.

Likely first improvements:

- Precompute per-algorithm routing schedules instead of scanning masks in the
  hot loop.
- Precompute carrier lists per algorithm.
- Separate self-feedback handling from normal modulation inputs.
- Consider specialized code paths for common algorithms if benchmarks justify
  it.

### Harmonic oscillator

The harmonic engine loops over eight harmonics per sample and performs:

- Smoother reads for multiple controls.
- Per-harmonic Nyquist branch.
- `sqrt` for inharmonic stretch.
- Gaussian scan using `exp`.
- Odd/even branching.
- Morph/FM branch selection.
- Per-harmonic phase wrap.

Likely first improvements:

- Cache per-harmonic coefficients when controls are stable or per small block.
- Move width/scan calculations out of the innermost path where possible.
- Replace some branches with precomputed amplitude masks or coefficient arrays.
- Investigate block-sized control-rate updates before SIMD.

### Timbral oscillator

The timbral engine has fewer voices of internal state but does several
parameter-dependent branches per sample:

- FM branch.
- feedback branch.
- sub-oscillator branch.
- timbre path in the wave multiplier.
- stage loop for folding.

Likely first improvements:

- Split clean and shaped paths when timbre/sub/FM/feedback are effectively off.
- Precompute stage count behavior or specialize stages 1 through 4.
- Evaluate whether branch reductions are actually helpful versus preserving a
  simple scalar path.

### Granular oscillator

The granular engine scans a 32-grain pool every sample:

- Advance many smoothers every sample.
- Spawn checks and random jitter.
- Branch per grain on `active`.
- Compute envelope per active grain.
- Normalize by active count.

Likely first improvements:

- Maintain active grain indices or a compact active list instead of scanning
  all 32 slots when grain counts are low.
- Move spawn scheduling and slow control updates toward block/control rate.
- Cache envelope-shape state where possible.
- Keep behavior gates strict because granular changes can easily alter texture.

## Benchmark Direction

Before EVO optimization, build a deterministic simulation harness in a worktree
or experiment branch. The harness should report one numeric score for EVO, but
also emit enough trace data for gates.

Recommended benchmark dimensions:

- Render time for fixed musical scenarios, measured as nanoseconds per rendered
  audio frame or rendered seconds per wall-clock second.
- Separate scenario timings for FM, Harmonic, Timbral, Granular, and full
  4-part engine.
- Stereo and 8-channel output paths.
- 256 and 512 frame blocks, with optional small-block scenarios at 8, 16, and
  32 frames for future block-processing work.

Recommended deterministic scenarios:

- Dense FM chord with high index and feedback.
- Harmonic scan/morph/FM sweep.
- Timbral high-timbre/high-symmetry patch with feedback and sub enabled.
- Granular high-density cloud with scatter, drift, FM, and morph enabled.
- Full engine with four parts active, sends enabled, and master FX in the path.
- Control-pressure trace with repeated `SetParameter`, pitch bend, mod wheel,
  and note events.

## Gates

Fast gates for every experiment:

- `cargo test -p brume-dsp-core --locked --no-fail-fast`
- `cargo test -p brume-engine-runtime --locked --no-fail-fast`
- `cargo test -p brume-fx-chain --locked --no-fail-fast`
- Benchmark output must be finite and non-silent for all non-silent scenarios.
- Peak bounds must stay within scenario-specific thresholds.
- Basic spectral proxies must not collapse, such as zero-crossing count,
  RMS range, and per-scenario energy floor.

Slower gates before accepting a result:

- `cargo fmt --all -- --check`
- `cargo test --workspace --locked --no-fail-fast`
- `cargo clippy --workspace --all-targets -- -D clippy::correctness -D clippy::suspicious -D clippy::perf -A clippy::pedantic`

Hardware gate before merge or release:

- Repeat the accepted benchmark and listening/smoke tests on the reference CM5.
- Check for xruns or audible regressions under normal UI and MIDI load.

## Proposed Sequence

1. Add a deterministic benchmark/simulation harness for `engine-runtime`.
2. Record baseline numbers on this Linux machine.
3. Use EVO only after the benchmark and gates are in place.
4. Start with control-flow/data-layout improvements inside oscillator hot paths.
5. Escalate to small-block processing after scalar improvements are measured.
6. Consider SIMD only after block-oriented data flow makes it practical.
7. Validate any accepted result on the CM5 before treating it as production
   performance work.

## Baseline Harness

Added a dependency-light explicit benchmark example:

```bash
cargo run --quiet -p brume-engine-runtime --example render_bench --release
```

Environment overrides:

- `BRUME_RENDER_BENCH_BLOCKS` (default `2000`)
- `BRUME_RENDER_BENCH_WARMUP_BLOCKS` (default `128`)
- `BRUME_RENDER_BENCH_BLOCK_SIZE` (default `512`)

The harness has three groups:

- Direct `BrumeVoice` scenarios, which isolate oscillator + voice cost.
- Direct `Part` scenarios, which measure six-voice part mixing, modulation
  advance, and part headroom without the full engine master path.
- Full `BrumeEngine` scenarios, which include part mixing, modulation advance,
  master/send buses, FX, limiter, and channel emission.

Both groups render fixed buffers and report elapsed time, nanoseconds per
rendered frame, real-time multiple, peak, RMS, first-channel zero crossings,
and checksum. They panic if a scenario produces non-finite output, falls below
a scenario-specific activity floor, or collapses below a broad zero-crossing
floor.

### Linux Baseline - 2026-06-04

Host context: isolated Linux development machine, not the reference CM5.
Numbers are useful for relative comparisons only.

Command:

```bash
cargo run --quiet -p brume-engine-runtime --example render_bench --release
```

Run settings:

- sample rate: 48000 Hz
- blocks: 2000
- warmup blocks: 128
- block size: 512 frames
- frames per scenario: 1024000

Warmed release pass after adding activity gates:

Direct voice scenarios:

| Scenario | ns/frame | realtime x | peak | RMS | zero crossings |
| --- | ---: | ---: | ---: | ---: | ---: |
| `voice_fm_dense` | 148.356 | 140.428 | 0.905357 | 0.277043 | 319782 |
| `voice_harmonic_scan` | 415.816 | 50.102 | 0.524548 | 0.073690 | 250229 |
| `voice_timbral_shaped` | 128.489 | 162.141 | 0.672628 | 0.253851 | 25020 |
| `voice_granular_cloud` | 565.398 | 36.847 | 1.102247 | 0.216131 | 139566 |

Voice aggregate `voice_mean_ns_per_frame`: `314.515`.

Direct part scenarios:

| Scenario | ns/frame | realtime x | peak | RMS | zero crossings |
| --- | ---: | ---: | ---: | ---: | ---: |
| `part_fm_dense` | 969.644 | 21.486 | 0.399785 | 0.082631 | 316669 |
| `part_harmonic_scan` | 2499.846 | 8.334 | 0.124419 | 0.021157 | 254173 |
| `part_timbral_shaped` | 765.116 | 27.229 | 0.294971 | 0.068802 | 39790 |
| `part_granular_cloud` | 3383.577 | 6.157 | 0.303217 | 0.052110 | 146691 |

Part aggregate `part_mean_ns_per_frame`: `1904.545`.

Engine scenarios:

| Scenario | ns/frame | realtime x | peak | RMS | zero crossings |
| --- | ---: | ---: | ---: | ---: | ---: |
| `fm_dense_stereo` | 1017.037 | 20.484 | 0.319828 | 0.066105 | 316669 |
| `harmonic_scan_stereo` | 2528.759 | 8.239 | 0.099535 | 0.016926 | 254173 |
| `timbral_shaped_stereo` | 812.531 | 25.640 | 0.235977 | 0.055042 | 39790 |
| `granular_cloud_stereo` | 3430.851 | 6.072 | 0.242573 | 0.041688 | 146691 |
| `full_engine_stereo` | 7622.891 | 2.733 | 0.445356 | 0.087394 | 226356 |
| `full_engine_8ch` | 7644.739 | 2.725 | 0.399785 | 0.060673 | 316669 |

Engine aggregate `engine_mean_ns_per_frame`: `3842.801`.

Combined aggregate `mean_ns_per_frame`: `2280.932` across 14 scenarios.

Initial read:

- Direct voice isolation confirms the external feedback: oscillator/voice
  implementation matters, with Granular and Harmonic the slowest voices.
- Granular direct voice cost is roughly 3.7x FM and 4.4x Timbral in this
  scenario.
- Harmonic direct voice cost is roughly 2.8x FM and 3.3x Timbral.
- The direct `Part` scenarios closely track the corresponding one-part engine
  scenarios, so the one-part engine rows are mostly part render cost plus a
  small master-path overhead.
- FM is materially slower than Timbral at the part layer despite direct voice
  cost being only modestly higher, likely because the dense FM chord keeps six
  comparatively expensive voices active while the timbral patch's waveform path
  remains cheaper.
- Full-engine stereo and 8-channel paths are close on this Linux machine,
  suggesting oscillator/FX work dominates over final channel emission here.
- The next analysis pass should inspect the Granular and Harmonic hot loops
  first, then FM part/chord behavior.

## Open Questions

- Should the benchmark harness live in the repo as a normal `bench`/example
  artifact, or only inside EVO worktrees at first?
- Do we want to add Criterion, or keep a dependency-light custom harness that
  prints JSON for EVO?
- Which scenario should be the primary score: full-engine render cost or a
  weighted aggregate over the four oscillator modes?
- How much output drift is acceptable for optimization work that preserves
  musical behavior but changes exact samples?
