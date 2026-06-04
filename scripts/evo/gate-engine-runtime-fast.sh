#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
#
# Fast pass/fail gate for EVO engine-runtime experiments.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

blocks="${BRUME_EVO_GATE_BLOCKS:-256}"
warmup_blocks="${BRUME_EVO_GATE_WARMUP_BLOCKS:-64}"
block_size="${BRUME_EVO_GATE_BLOCK_SIZE:-512}"

cargo fmt --all -- --check
cargo test -p brume-dsp-core --locked --no-fail-fast
cargo test -p brume-engine-runtime --locked --no-fail-fast
cargo test -p brume-fx-chain --locked --no-fail-fast
cargo clippy -p brume-engine-runtime --all-targets -- \
  -D clippy::correctness \
  -D clippy::suspicious \
  -D clippy::perf \
  -A clippy::pedantic

bench_output="$(
  BRUME_RENDER_BENCH_BLOCKS="$blocks" \
    BRUME_RENDER_BENCH_WARMUP_BLOCKS="$warmup_blocks" \
    BRUME_RENDER_BENCH_BLOCK_SIZE="$block_size" \
    cargo run --quiet -p brume-engine-runtime --example render_bench --release
)"

if [[ "${BRUME_EVO_GATE_VERBOSE:-0}" == "1" ]]; then
  printf '%s\n' "$bench_output"
fi

printf '%s\n' "$bench_output" | scripts/evo/check-render-bench-behavior.py
