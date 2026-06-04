#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
"""Validate render_bench output against broad behavior floors."""

from __future__ import annotations

import math
import os
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Metrics:
    frames: int
    peak: float
    rms: float
    zero_crossings: int

    @property
    def zero_crossing_rate(self) -> float:
        return self.zero_crossings / self.frames


def ratio_env(name: str, default: float) -> float:
    raw = os.environ.get(name)
    if raw is None:
        return default
    try:
        value = float(raw)
    except ValueError:
        print(f"{name} must be numeric, got {raw!r}", file=sys.stderr)
        sys.exit(2)
    if not math.isfinite(value) or value <= 0.0:
        print(f"{name} must be a positive finite number, got {raw!r}", file=sys.stderr)
        sys.exit(2)
    return value


MIN_PEAK_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MIN_PEAK_RATIO", 0.35)
MAX_PEAK_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MAX_PEAK_RATIO", 2.50)
MIN_RMS_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MIN_RMS_RATIO", 0.45)
MAX_RMS_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MAX_RMS_RATIO", 2.50)
MIN_ZC_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MIN_ZERO_CROSSING_RATIO", 0.45)
MAX_ZC_RATIO = ratio_env("BRUME_EVO_BEHAVIOR_MAX_ZERO_CROSSING_RATIO", 2.50)


def parse_pairs(line: str) -> dict[str, str]:
    pairs: dict[str, str] = {}
    for token in line.split():
        if "=" in token:
            key, value = token.split("=", 1)
            pairs[key] = value
    return pairs


def parse_benchmark(text: str) -> dict[tuple[str, str], Metrics]:
    parsed: dict[tuple[str, str], Metrics] = {}
    for line in text.splitlines():
        if line.startswith("voice_scenario="):
            kind = "voice"
            name_key = "voice_scenario"
        elif line.startswith("part_scenario="):
            kind = "part"
            name_key = "part_scenario"
        elif line.startswith("scenario="):
            kind = "engine"
            name_key = "scenario"
        else:
            continue

        pairs = parse_pairs(line)
        try:
            name = pairs[name_key]
            frames = int(pairs["frames"])
            peak = float(pairs["peak"])
            rms = float(pairs["rms"])
            zero_crossings = int(pairs["zero_crossings"])
        except (KeyError, ValueError) as exc:
            print(f"failed to parse render_bench line: {line}\n{exc}", file=sys.stderr)
            sys.exit(2)

        parsed[(kind, name)] = Metrics(
            frames=frames,
            peak=peak,
            rms=rms,
            zero_crossings=zero_crossings,
        )

    return parsed


def parse_baseline(path: Path) -> dict[tuple[str, str], Metrics]:
    baseline: dict[tuple[str, str], Metrics] = {}
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue

        fields = stripped.split()
        if len(fields) != 6:
            print(f"{path}:{line_no}: expected 6 fields, got {len(fields)}", file=sys.stderr)
            sys.exit(2)

        kind, name, frames, peak, rms, zero_crossings = fields
        try:
            baseline[(kind, name)] = Metrics(
                frames=int(frames),
                peak=float(peak),
                rms=float(rms),
                zero_crossings=int(zero_crossings),
            )
        except ValueError as exc:
            print(f"{path}:{line_no}: invalid numeric field: {exc}", file=sys.stderr)
            sys.exit(2)

    return baseline


def check_ratio(
    failures: list[str],
    key: tuple[str, str],
    metric: str,
    value: float,
    baseline: float,
    min_ratio: float,
    max_ratio: float,
) -> None:
    if not math.isfinite(value):
        failures.append(f"{key}: {metric} is non-finite: {value}")
        return
    if baseline <= 0.0:
        failures.append(f"{key}: baseline {metric} is not positive: {baseline}")
        return

    ratio = value / baseline
    if ratio < min_ratio or ratio > max_ratio:
        failures.append(
            f"{key}: {metric} ratio {ratio:.3f} outside "
            f"[{min_ratio:.3f}, {max_ratio:.3f}] "
            f"(value={value:.6f}, baseline={baseline:.6f})"
        )


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    baseline_path = Path(
        os.environ.get(
            "BRUME_EVO_BEHAVIOR_BASELINE",
            script_dir / "render-bench-baseline.tsv",
        )
    )

    text = sys.stdin.read()
    if not text.strip():
        print("expected render_bench output on stdin", file=sys.stderr)
        return 2

    baseline = parse_baseline(baseline_path)
    current = parse_benchmark(text)
    failures: list[str] = []

    missing = sorted(set(baseline) - set(current))
    extra = sorted(set(current) - set(baseline))
    if missing:
        failures.append(f"missing scenarios: {missing}")
    if extra:
        failures.append(f"unexpected scenarios: {extra}")

    for key, base in sorted(baseline.items()):
        metrics = current.get(key)
        if metrics is None:
            continue

        if metrics.frames <= 0:
            failures.append(f"{key}: frame count must be positive: {metrics.frames}")
            continue

        check_ratio(failures, key, "peak", metrics.peak, base.peak, MIN_PEAK_RATIO, MAX_PEAK_RATIO)
        check_ratio(failures, key, "rms", metrics.rms, base.rms, MIN_RMS_RATIO, MAX_RMS_RATIO)
        check_ratio(
            failures,
            key,
            "zero_crossing_rate",
            metrics.zero_crossing_rate,
            base.zero_crossing_rate,
            MIN_ZC_RATIO,
            MAX_ZC_RATIO,
        )

    if failures:
        print("render_bench behavior gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"render_bench behavior gate passed: {len(baseline)} scenarios", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
