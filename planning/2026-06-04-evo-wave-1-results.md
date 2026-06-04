# EVO Wave 1 Results - Engine Runtime

Date: 2026-06-04
Branch: `evo-performance`
EVO run: `run_0000`

This note records the first contained EVO optimization wave after the benchmark
and gate harness landed. The work ran on the isolated Linux development machine,
not on the reference CM5, so these results are search evidence rather than final
hardware validation.

## Frontier

Current best path:

```text
exp_0002 baseline wiring check after EVO result-file fix
  -> exp_0003 use sine LUT in phase oscillator waveform helpers
  -> exp_0004 avoid rem_euclid in harmonic phase advance
```

Scores:

| Experiment | Status | Score | Delta vs parent | Result |
| --- | --- | ---: | ---: | --- |
| `exp_0002` | committed | 2301.508 | n/a | Baseline |
| `exp_0003` | committed | 2289.315 | -12.193 | Integrated as `dsp-core: use sine LUT in phase helpers` |
| `exp_0004` | committed | 2281.670 | -7.645 | Integrated as `engine: simplify harmonic phase wrap` |
| `exp_0005` | discarded | 2285.618 | +3.948 | Harmonic invariant hoist regressed after 3 attempts |
| `exp_0006` | discarded | 2292.456 | +10.786 | Granular Hann-cosine branch move regressed after 3 attempts |
| `exp_0007` | discarded | n/a | n/a | Granular fast-exp approximation regressed in preliminary scoring |

The best committed EVO score improved from `2301.508` to `2281.670`
mean ns/frame, about `0.86%` on this benchmark. Several ad hoc score runs showed
larger wins, but repeated formal EVO attempts were the deciding evidence.

## Accepted Changes

### Shared sine LUT for phase helpers

`exp_0003` replaced direct `f32::sin` calls in `PhaseOscillator::sine`,
`PhaseOscillator::sine_pm`, and the sine portions of `morph_wave` with
`brume_dsp_core::sine_normalized`.

Why it was accepted:

- It directly targets hot sine paths used by Harmonic, Granular, and Timbral
  voice rendering.
- It preserves the existing waveform-helper structure.
- It introduces no new audio-thread allocation or control-flow dependency in
  the per-sample loop.
- `cargo test -p brume-dsp-core --locked --no-fail-fast` passed before
  integration.

### Positive wrap for Harmonic phase advance

`exp_0004` replaced a per-harmonic `rem_euclid(1.0)` with a single positive
wrap in `HarmonicOscillator::process`.

Why it was accepted:

- The code skips harmonics at or above Nyquist before advancing phase.
- The resulting `phase_inc` is below half a cycle per sample, so one subtract
  is enough when the phase crosses `1.0`.
- Harmonic phases only advance forward in this oscillator.
- `cargo test -p brume-engine-runtime --locked --no-fail-fast` passed before
  integration.

## Rejected Changes

### Harmonic loop invariant hoist

`exp_0005` hoisted scan-window and sample-rate invariants out of the
per-harmonic loop. It looked promising in one ad hoc score run, but formal EVO
runs scored:

```text
2290.719
2290.697
2285.618
```

Parent score was `2281.670`, so this was discarded.

### Granular Hann-cosine branch move

`exp_0006` moved the Hann-envelope cosine into the `shape <= 0.5` branch. The
change is plausible and should be behavior-preserving for the high-shape
benchmark path, but repeated formal EVO runs scored:

```text
2286.964
2287.116
2292.456
```

Parent score was `2281.670`, so this was discarded.

### Granular fast-exp approximation

`exp_0007` replaced the granular Gaussian envelope `exp` call with the existing
`fast_exp2` approximation. It scored `2388.358` in preliminary local scoring and
also changed envelope shape, so it was discarded without spending formal EVO
attempts.

## Process Lessons

- The 1000-block score command has enough run-to-run noise to swamp tiny
  changes. Formal repeated EVO attempts should outweigh isolated ad hoc wins.
- Changes with a strong correctness argument can still fail the benchmark. Keep
  discarded hypotheses explicit so EVO and future agents do not rediscover the
  same dead ends.
- Larger future candidates should either be expected to beat roughly
  `10-20 ns/frame` of local noise or should use a longer scoring run for
  confirmation before integration.

## Next Candidates

1. Revisit granular only with changes that are larger than arithmetic hoists:
   fixed-size active-index management, envelope lookup, or a stricter
   high-shape fast path.
2. Try FM routing schedules from the table-driven algorithms; this is lower
   priority than Granular but has a clearer behavior-preservation story.
3. Improve the EVO score configuration for low-noise confirmation runs before
   accepting more marginal changes.
4. Validate the accepted best path on the CM5 before release or merge.
