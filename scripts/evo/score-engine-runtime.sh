#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
#
# Numeric score command for EVO engine-runtime experiments.
# Prints one JSON object to stdout. Lower score is better.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

blocks="${BRUME_EVO_SCORE_BLOCKS:-1000}"
warmup_blocks="${BRUME_EVO_SCORE_WARMUP_BLOCKS:-128}"
block_size="${BRUME_EVO_SCORE_BLOCK_SIZE:-512}"

output="$(
  BRUME_RENDER_BENCH_BLOCKS="$blocks" \
    BRUME_RENDER_BENCH_WARMUP_BLOCKS="$warmup_blocks" \
    BRUME_RENDER_BENCH_BLOCK_SIZE="$block_size" \
    cargo run --quiet -p brume-engine-runtime --example render_bench --release
)"

if [[ "${BRUME_EVO_SCORE_VERBOSE:-0}" == "1" ]]; then
  printf '%s\n' "$output" >&2
fi

summary="$(printf '%s\n' "$output" | awk '/^summary / { line=$0 } END { print line }')"
score="$(printf '%s\n' "$summary" | sed -n 's/.*mean_ns_per_frame=\([0-9.][0-9.]*\).*/\1/p')"

if [[ -z "$score" ]]; then
  printf 'failed to parse mean_ns_per_frame from render_bench output\n' >&2
  printf '%s\n' "$output" >&2
  exit 1
fi

printf '{"score":%s,"metric":"mean_ns_per_frame","direction":"min","blocks":%s,"warmup_blocks":%s,"block_size":%s}\n' \
  "$score" \
  "$blocks" \
  "$warmup_blocks" \
  "$block_size"
