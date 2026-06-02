# Changelog

All notable changes to Brume are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until
`1.0.0`, the `0.MINOR.0` slot moves on anything user-observable (new
engine features, Lua API additions, control surface support, meaningful
UI reshuffles) and `0.MINOR.PATCH` is reserved for bug fixes.

## [Unreleased]

## [0.3.0] - 2026-06-01

### Added
- Pitch-bend wheel and mod wheel now work out of the box for any
  keyboard already sending notes. Pitch bend maps to a ±2-semitone
  per-part pitch offset; the mod wheel (CC1) drives vibrato — a ~5.5 Hz
  pitch LFO whose depth follows the wheel, neutral at zero so a DAW
  auto-firing CC1 can't disrupt the sound. CC1 stays MIDI-Learn-
  overridable.
- SYS → AUDIO OUTPUT gains a LATENCY selector — LOW / BALANCED / SAFE /
  RELAXED (256 / 512 / 1024 / 2048-frame buffer) — so audio output
  latency is tunable from the touchscreen, not only the
  `BRUME_AUDIO_PERIOD` env var. The choice persists in `settings.json`
  and rebuilds the audio stream on change (a brief dropout, like a
  device switch); `BRUME_AUDIO_PERIOD` still overrides it.
- Factory presets now ship with installable releases. The release
  bundles the preset library as `brume-<version>-factory.tar.gz`, and
  `brumectl install` stages it to `/usr/share/brume/factory` on both a
  fresh install and `--update`. Previously only the binary and configs
  were installed, so a `brumectl`-installed device booted with an empty
  LIBRARY.

### Changed
- The factory preset library is rebuilt from scratch — 100 presets, 25
  per engine, replacing the previous 45. Each preset is gain-staged on
  the loudness-calibrated engine, and the set spans every engine's
  character deliberately: FM for metallic, glassy and percussive tones;
  Harmonic for additive organs, choirs and icy pads; Timbral for analog
  wavefolding; Granular for grain clouds, atmospheres and textures. The
  granular voices vary grain timbre — waveform morph and per-grain FM —
  rather than only cloud density, so they no longer collapse toward a
  single sound.
- The four oscillator engines are now loudness-calibrated to each
  other. They normalized very differently — Harmonic ran ~8 dB below FM,
  Timbral ~2.5 dB above — so switching presets across engines jumped in
  volume. A per-mode makeup gain brings them to a consistent level
  (measured against a common reference tone), compressing the spread
  from ~10 dB to ~3 dB.
- USB Meridian audio output now opens at a low-latency buffer by
  default (512 frames, ~6–7 ms) instead of the gadget's deep device
  default (~5120 frames, ~107 ms), cutting keyboard-to-sound latency by
  roughly 80 ms with no measured CPU or stability cost on the reference
  CM5. The new `BRUME_AUDIO_PERIOD` env var overrides the buffer size
  for tuning, and any size the device rejects falls back to the device
  default so audio always opens.
- The SCOPE OUT meter is now a ballistic dB level meter. It animates
  at the UI tick rate with instant attack and a smooth ~220 ms release
  instead of snapping to each raw ~40 Hz frame, carries a peak-hold
  tick, and is scaled in dBFS with graduated green / amber / red zones.
  A clip LED at the right edge latches when the part reaches 0 dBFS,
  which also surfaces a per-part dry stem running hot enough to drive
  the Meridian stem soft-saturator.

### Fixed
- Dense chords no longer overdrive the per-part saturator. The
  voice-count headroom target was `1/√n`, which assumes uncorrelated
  voices — but a chord's voices correlate and sum faster than that, so
  the mix overshot into the saturator. Tightened to `1/n^0.8` so dense
  chords stay near the knee (a chord is now a touch quieter relative to
  a single note, rather than saturated).
- Rapidly repeated notes no longer click at the attack — overlapping
  1/16-note retriggers and fast arpeggios where a voice is restruck
  while its release tail is still sounding. Per-voice velocity is now
  smoothed, which covers the only discontinuity a retrigger
  introduces (oscillator phase and amp envelope already carry
  through); the previous output cross-fade, which froze a sample and
  faded it as a DC pulse downstream of the DC blocker, is removed.
- Dense, high-velocity chords no longer overshoot into audible
  distortion on the per-part outputs. The mix-headroom tracker
  (target `1/√active-voices`) now ducks asymmetrically — fast (2 ms)
  as voices stack, slow (30 ms) as they release — so a chord's summed
  attack transient is bounded before it reaches the stem
  soft-saturator, while long release tails keep the slow restore that
  avoids ghost-retrigger steps.
- `brumectl install` fetches the brume binary under its real
  release-asset name (`brume-<version>-aarch64-unknown-linux-gnu`). It
  had requested a versionless name no release publishes, so the
  auto-fetch 404'd for any user without a local workspace build — the
  primary onboarding path. Masked for maintainers, whose local build is
  resolved first.

[Unreleased]: https://github.com/aftertonesignal/brume/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/aftertonesignal/brume/releases/tag/v0.3.0
