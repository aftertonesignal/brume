# EVO Runbook - Engine Runtime Performance

Date: 2026-06-04
Branch: `evo-performance`

This runbook prepares Brume for EVO without starting an EVO run. It keeps all
EVO-facing glue in newly added files so the work can be redacted by removing
`scripts/evo/`, `crates/engine-runtime/examples/render_bench.rs`, and the
`planning/` notes.

## Local Tool State

On this Linux development machine:

- `evo` is available at `/home/bran/.local/bin/evo`.
- `uv` is available at `/home/bran/.local/bin/uv`.
- `evo doctor codex` passes:
  - `plugin_hooks = true` in `/home/bran/.codex/config.toml`
  - `[plugins."evo@evo-hq"]` is registered
  - marketplace cache exists under `/home/bran/.codex/.tmp/marketplaces/evo-hq`
  - `evo-hook-drain` is present and executable for EVO `0.4.5`

The reference CM5 is not attached to this machine. EVO can use these local
Linux results for relative search, but accepted candidates still need CM5
validation from the macOS-attached board.

## EVO Inputs

EVO needs a benchmark command and metric direction. For this branch:

```bash
./scripts/evo/score-engine-runtime.sh
```

Metric direction: minimize.

The score command prints a single JSON object to stdout:

- `mean_ns_per_frame` from the benchmark summary.
- Lower is better.
- When EVO sets `EVO_RESULT_PATH`, the same JSON payload is also written there;
  EVO `0.4.5` check/run paths require this file.
- Default score run settings:
  - `BRUME_EVO_SCORE_BLOCKS=1000`
  - `BRUME_EVO_SCORE_WARMUP_BLOCKS=128`
  - `BRUME_EVO_SCORE_BLOCK_SIZE=512`

Overrides are intentionally separate from the benchmark's own environment
variables so EVO-facing commands can be tuned without changing the harness:

```bash
BRUME_EVO_SCORE_BLOCKS=2000 ./scripts/evo/score-engine-runtime.sh
BRUME_EVO_SCORE_VERBOSE=1 ./scripts/evo/score-engine-runtime.sh
```

## Gate Command

Use this as the initial pass/fail gate:

```bash
./scripts/evo/gate-engine-runtime-fast.sh
```

It runs:

- `cargo fmt --all -- --check`
- `cargo test -p brume-dsp-core --locked --no-fail-fast`
- `cargo test -p brume-engine-runtime --locked --no-fail-fast`
- `cargo test -p brume-fx-chain --locked --no-fail-fast`
- targeted engine-runtime clippy correctness/suspicious/perf denies
- a shorter release `render_bench` pass to enforce activity floors and compare
  behavior metrics against `scripts/evo/render-bench-baseline.tsv`

Default gate benchmark settings:

- `BRUME_EVO_GATE_BLOCKS=256`
- `BRUME_EVO_GATE_WARMUP_BLOCKS=64`
- `BRUME_EVO_GATE_BLOCK_SIZE=512`

The benchmark itself panics on non-finite samples, silence/activity collapse,
or zero-crossing collapse. The EVO behavior checker adds broader baseline
comparison for every scenario:

- peak must remain within the configured peak ratio envelope
- RMS must remain within the configured RMS ratio envelope
- zero-crossing rate must remain within the configured spectral-activity
  envelope
- all expected voice, part, and engine scenarios must still be present

The default envelopes are intentionally broad enough for small waveform-level
changes but narrow enough to reject obvious cheating:

- `BRUME_EVO_BEHAVIOR_MIN_PEAK_RATIO=0.35`
- `BRUME_EVO_BEHAVIOR_MAX_PEAK_RATIO=2.50`
- `BRUME_EVO_BEHAVIOR_MIN_RMS_RATIO=0.45`
- `BRUME_EVO_BEHAVIOR_MAX_RMS_RATIO=2.50`
- `BRUME_EVO_BEHAVIOR_MIN_ZERO_CROSSING_RATIO=0.45`
- `BRUME_EVO_BEHAVIOR_MAX_ZERO_CROSSING_RATIO=2.50`

## Suggested Discovery Prompt

When ready, seed EVO discovery with a narrow target instead of asking it to
rediscover the whole application:

```text
Optimize Brume's engine-runtime voice engine performance on the evo-performance
branch. Focus on crates/engine-runtime and brume-dsp-core helpers used by the
voice hot path. Do not change UI, persistence, MIDI, deploy scripts, or
packaging. The benchmark command is ./scripts/evo/score-engine-runtime.sh and
the metric should be minimized. Use ./scripts/evo/gate-engine-runtime-fast.sh
as the gate. Preserve audio-thread safety: no unsafe, no audio-thread
allocation, no syscalls in process_block or callees, no panics, and no
denormal regressions. Start with small scalar changes around morph_wave,
PhaseOscillator::sine, HarmonicOscillator, GranularOscillator, and FM routing
schedules before attempting block or SIMD refactors.
```

Host syntax from EVO's README differs by agent. For Codex, EVO's README says
to use `$evo`; for Claude Code it uses `/evo:`.

## Before Running Optimize

1. Run `evo doctor codex`.
2. Confirm Codex hook trust if the doctor reports hook issues.
3. Run the score and gate commands once from this branch.
4. Commit or intentionally keep the pre-EVO harness files uncommitted,
   depending on whether EVO should inherit them through git history or direct
   working-tree state.
5. Keep the first EVO run constrained to worktree/local execution. Remote or
   CM5-backed runs can come later.

## Validation Snapshot

Validated on 2026-06-04:

```bash
./scripts/evo/score-engine-runtime.sh
```

Result:

```json
{"score":2285.854,"metric":"mean_ns_per_frame","direction":"min","blocks":1000,"warmup_blocks":128,"block_size":512}
```

```bash
./scripts/evo/gate-engine-runtime-fast.sh
```

Result: passed, including `render_bench behavior gate passed: 14 scenarios`.

```bash
evo doctor codex
```

Result: passed.

## Acceptance Rules

An EVO candidate is not accepted solely because the score improves. Require:

- Gate command passes.
- Full 512-frame score run is repeated at least twice.
- One 16- or 32-frame run is repeated to catch small-block regressions.
- Output metrics remain plausible: peak, RMS, zero crossings, and checksum or
  energy drift do not indicate silence, skipped work, or collapsed spectra.
- The diff is explainable in audio-thread terms.
- CM5 validation happens before merge or release.
