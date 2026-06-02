// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Core types used across Brume crate boundaries.

use std::str::FromStr;

use serde::de::IntoDeserializer;
use serde::de::value::{Error as SerdeDeError, StrDeserializer};
use serde::{Deserialize, Serialize};

/// Oscillator architecture mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OscillatorMode {
    /// 6-operator FM — DX7/Digitone-style data-driven algorithm routing.
    Fm,
    /// Additive synthesis with per-harmonic level control (Verbos inspired).
    Harmonic,
    /// Triangle core + nonlinear waveshaper (Serge NTO inspired).
    Timbral,
    /// Granular synthesis — clouds of micro-oscillator grains (Xenakis / Roads inspired).
    Granular,
}

/// Audio output latency preset, exposed in SYS → AUDIO OUTPUT. Each maps
/// to a cpal `BufferSize::Fixed` value — the total ALSA buffer in frames,
/// with the period taken at frames/4. Lower = less latency but less slack
/// for USB-consumption jitter and CPU spikes (underrun risk). Persisted in
/// `settings.json`; `BRUME_AUDIO_PERIOD` overrides it for headless tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LatencyPreset {
    /// 256-frame buffer (period 64). Lowest latency, tightest margin.
    Low,
    /// 512-frame buffer (period 128). Validated default on the reference CM5.
    #[default]
    Balanced,
    /// 1024-frame buffer (period 256). Extra margin for busier hosts.
    Safe,
    /// 2048-frame buffer (period 512). Maximum stability.
    Relaxed,
}

impl LatencyPreset {
    /// Total ALSA buffer size in frames (cpal `BufferSize::Fixed`).
    #[must_use]
    pub fn frames(self) -> u32 {
        match self {
            Self::Low => 256,
            Self::Balanced => 512,
            Self::Safe => 1024,
            Self::Relaxed => 2048,
        }
    }

    /// Short uppercase label for the SYS selector.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "LOW",
            Self::Balanced => "BALANCED",
            Self::Safe => "SAFE",
            Self::Relaxed => "RELAXED",
        }
    }

    /// All presets in latency order, for rendering the selector.
    #[must_use]
    pub fn all() -> [Self; 4] {
        [Self::Low, Self::Balanced, Self::Safe, Self::Relaxed]
    }
}

/// Canonical parameter identifier.
///
/// Every controllable parameter in Brume has a unique ID used for
/// messaging between UI, control, and engine layers.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterId {
    MasterVolume,

    // FM oscillator (part 0). 6 sine operators plus an algorithm
    // selector and a global feedback amount. Per-op ratio and level
    // expose the full DX-style palette; algorithm routing is
    // data-driven — see crates/engine-runtime/src/fm_oscillator.rs.
    /// Global FM depth — scales modulator contributions uniformly (0.0 - 10.0).
    FmIndex,
    /// Algorithm selector. UI sends a normalized 0..1 tap value which the
    /// engine scales to one of the topologies in
    /// `engine-runtime::fm_oscillator::ALGORITHMS`.
    Algorithm,
    /// Feedback amount for operators with self-loop in the algorithm (0.0-1.0).
    FmFeedback,

    /// Per-operator frequency ratio (0.25-16.0).
    Op1Ratio,
    Op2Ratio,
    Op3Ratio,
    Op4Ratio,
    Op5Ratio,
    Op6Ratio,

    /// Per-operator output/modulation level (0.0-1.0).
    Op1Level,
    Op2Level,
    Op3Level,
    Op4Level,
    Op5Level,
    Op6Level,

    // FM-index envelope (per-voice). Scales global FM index over
    // time — lets each voice's FM depth decay independently, so
    // setting FmIndex mid-chord doesn't color every sustaining note.
    // EnvDepth = 0 means envelope is off (static FmIndex only).
    /// Envelope contribution above the static FmIndex (0.0-10.0).
    FmIndexEnvDepth,
    /// FM-index envelope attack time (ms).
    FmIndexEnvAttack,
    /// FM-index envelope decay time (ms).
    FmIndexEnvDecay,
    /// FM-index envelope sustain level (0.0-1.0).
    FmIndexEnvSustain,
    /// FM-index envelope release time (ms).
    FmIndexEnvRelease,

    // Harmonic oscillator
    /// Per-harmonic level (0.0-1.0). Harmonics 1-8.
    HarmonicLevel1,
    HarmonicLevel2,
    HarmonicLevel3,
    HarmonicLevel4,
    HarmonicLevel5,
    HarmonicLevel6,
    HarmonicLevel7,
    HarmonicLevel8,
    /// Spectral tilt: -1.0 = low emphasis, 1.0 = high emphasis.
    HarmonicTilt,
    /// Odd/even balance: 0.0 = odd only, 0.5 = all, 1.0 = even only.
    HarmonicOddEven,
    /// Harmonic stretch for bell/metallic character (0.0-1.0).
    Inharmonicity,
    /// Scan window center position (0.0-1.0, sweeps across harmonics).
    ScanCenter,
    /// Scan window width (0.0 = single harmonic, 1.0 = all pass).
    ScanWidth,
    /// Per-harmonic waveform morph: 0=sine, 0.33=tri, 0.66=saw, 1.0=square.
    HarmonicMorph,
    /// FM depth on the fundamental (0.0-10.0).
    HarmonicFmDepth,
    /// FM modulator ratio to fundamental (0.5-16.0).
    HarmonicFmRatio,
    /// Phase spread: 0=locked, 1=random phases between harmonics.
    HarmonicSpread,

    // Timbral oscillator
    /// NTO waveshaper drive: 0.0 = pure triangle, 1.0 = max shaping.
    Timbre,
    /// Asymmetric shaping for even harmonics (-1.0 to 1.0).
    Symmetry,
    /// Linear FM depth on the triangle core (0.0-10.0).
    TimbralFmDepth,
    /// Linear FM modulator ratio to fundamental (0.5-16.0).
    TimbralFmRatio,
    /// Sub-oscillator mix level (0.0-1.0).
    SubLevel,
    /// Self-modulation feedback depth (0.0-1.0).
    TimbralFeedback,
    /// Wave multiplier fold stages (1.0-4.0, cast to integer).
    MultiplierStages,

    // Granular oscillator
    /// Grain spawn rate: 0.0-1.0 maps to 1-200 grains/sec.
    GranularDensity,
    /// Grain duration: 0.0-1.0 maps to 1-500ms.
    GranularGrainSize,
    /// Grain waveform morph: 0=sine, 0.33=tri, 0.66=saw, 1.0=square.
    GranularMorph,
    /// Pitch randomization per grain: 0=unison, 1=±2 octaves.
    GranularScatter,
    /// Pan randomization per grain: 0=center, 1=full width.
    GranularSpread,
    /// Slow pitch random walk: 0=stable, 1=wandering.
    GranularDrift,
    /// Per-grain FM depth (0.0-10.0).
    GranularFmDepth,
    /// Shared FM modulator ratio (0.5-16.0).
    GranularFmRatio,
    /// Grain start phase offset (0.0-1.0).
    GranularPosition,
    /// Grain envelope shape: 0=Hann, 0.5=Gaussian, 1=Trapezoid.
    GranularGrainShape,

    // Filter
    FilterCutoff,
    FilterResonance,
    /// How far the filter envelope opens the cutoff (0.0 - 1.0).
    FilterEnvDepth,

    // Filter envelope
    FilterEnvAttack,
    FilterEnvDecay,
    FilterEnvSustain,
    FilterEnvRelease,

    // Amp envelope
    AmpAttack,
    AmpDecay,
    AmpSustain,
    AmpRelease,
}

/// Scaling rule for translating a normalized 0..1 controller value
/// into the natural unit of a parameter. The UI owns scale choice
/// (matched against the param's slider behavior) and ships it to
/// midi-io as part of a `KnobBinding` so the controller path can
/// produce engine-ready values without bouncing through JavaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// `out = min + ratio * (max - min)`
    Linear,
    /// `out = exp(log(min) + ratio * (log(max) - log(min)))` —
    /// requires `min > 0`. Filter cutoffs, op ratios, envelope times.
    Log,
}

/// One knob's mapping: which (part, parameter) it drives, with the
/// range and scale needed to translate a normalized 0..1 controller
/// value into engine units. For tap-style enum params (Algorithm,
/// MultiplierStages) keep `Scale::Linear` with `min: 0, max: 1` and
/// let the engine quantize internally.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KnobBinding {
    pub part: u8,
    pub id: ParameterId,
    pub min: f32,
    pub max: f32,
    pub scale: Scale,
}

impl KnobBinding {
    /// Translate a 0..1 controller ratio into the parameter's natural unit.
    #[must_use]
    pub fn apply(&self, ratio: f32) -> f32 {
        let r = ratio.clamp(0.0, 1.0);
        match self.scale {
            Scale::Linear => self.min + r * (self.max - self.min),
            Scale::Log if self.min > 0.0 && self.max > self.min => {
                (self.min.ln() + r * (self.max.ln() - self.min.ln())).exp()
            }
            // Falls back to linear when the range can't be log-scaled
            // (e.g. min <= 0). Better than NaN.
            Scale::Log => self.min + r * (self.max - self.min),
        }
    }

    /// Inverse of `apply`: translate a natural-unit value back to a
    /// 0..1 ratio. Used by the UI to position a slider when the engine
    /// echoes a `ParameterChanged` (Lua, MIDI Learn, automation, etc.).
    /// Out-of-range values clamp to [0, 1] rather than producing NaN.
    #[must_use]
    pub fn unapply(&self, value: f32) -> f32 {
        match self.scale {
            Scale::Linear if (self.max - self.min).abs() > f32::EPSILON => {
                ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
            }
            Scale::Log if self.min > 0.0 && self.max > self.min && value > 0.0 => {
                ((value.ln() - self.min.ln()) / (self.max.ln() - self.min.ln())).clamp(0.0, 1.0)
            }
            // Degenerate range or non-positive log value — return 0
            // rather than NaN. Slider parks at the left edge.
            _ => 0.0,
        }
    }
}

/// Eight-slot routing table from a control surface's audio-priority
/// knobs to engine parameters. The UI owns the mapping (it reflects
/// the active engine + sub-tab) and ships updates to midi-io on
/// every selection change; midi-io reads it on each incoming CC and,
/// when the surface driver claims that CC via `rt_knob_slot`, routes
/// straight to `SetParameter` without a UI round-trip.
pub type KnobMapping = [Option<KnobBinding>; 8];

/// Error returned by `ParameterId`'s `FromStr` impl when the input does
/// not match any known variant. Carries the offending string so callers
/// can surface it in a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseParameterIdError(pub String);

impl std::fmt::Display for ParseParameterIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown ParameterId: {:?}", self.0)
    }
}

impl std::error::Error for ParseParameterIdError {}

impl FromStr for ParameterId {
    type Err = ParseParameterIdError;

    /// Parses a `ParameterId` from its variant name. Uses serde's
    /// `Deserialize` impl as the single source of truth — adding a new
    /// variant to the enum automatically extends `FromStr`'s coverage.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let de: StrDeserializer<'_, SerdeDeError> = s.into_deserializer();
        Self::deserialize(de).map_err(|_| ParseParameterIdError(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_round_trips_every_variant() {
        // Every variant Serialize emits must round-trip through FromStr.
        // This test is the contract that stops the three-way drift that
        // silently dropped mod assignments before this unification.
        let all = [
            ParameterId::MasterVolume,
            ParameterId::FmIndex,
            ParameterId::Algorithm,
            ParameterId::FmFeedback,
            ParameterId::Op1Ratio,
            ParameterId::Op2Ratio,
            ParameterId::Op3Ratio,
            ParameterId::Op4Ratio,
            ParameterId::Op5Ratio,
            ParameterId::Op6Ratio,
            ParameterId::Op1Level,
            ParameterId::Op2Level,
            ParameterId::Op3Level,
            ParameterId::Op4Level,
            ParameterId::Op5Level,
            ParameterId::Op6Level,
            ParameterId::FmIndexEnvDepth,
            ParameterId::FmIndexEnvAttack,
            ParameterId::FmIndexEnvDecay,
            ParameterId::FmIndexEnvSustain,
            ParameterId::FmIndexEnvRelease,
            ParameterId::HarmonicLevel1,
            ParameterId::HarmonicLevel8,
            ParameterId::HarmonicTilt,
            ParameterId::HarmonicOddEven,
            ParameterId::Inharmonicity,
            ParameterId::ScanCenter,
            ParameterId::ScanWidth,
            ParameterId::HarmonicMorph,
            ParameterId::HarmonicFmDepth,
            ParameterId::HarmonicFmRatio,
            ParameterId::HarmonicSpread,
            ParameterId::Timbre,
            ParameterId::Symmetry,
            ParameterId::TimbralFmDepth,
            ParameterId::TimbralFmRatio,
            ParameterId::SubLevel,
            ParameterId::TimbralFeedback,
            ParameterId::MultiplierStages,
            ParameterId::GranularDensity,
            ParameterId::GranularGrainSize,
            ParameterId::GranularMorph,
            ParameterId::GranularScatter,
            ParameterId::GranularSpread,
            ParameterId::GranularDrift,
            ParameterId::GranularFmDepth,
            ParameterId::GranularFmRatio,
            ParameterId::GranularPosition,
            ParameterId::GranularGrainShape,
            ParameterId::FilterCutoff,
            ParameterId::FilterResonance,
            ParameterId::FilterEnvDepth,
            ParameterId::FilterEnvAttack,
            ParameterId::FilterEnvDecay,
            ParameterId::FilterEnvSustain,
            ParameterId::FilterEnvRelease,
            ParameterId::AmpAttack,
            ParameterId::AmpDecay,
            ParameterId::AmpSustain,
            ParameterId::AmpRelease,
        ];
        for id in all {
            let name = serde_json::to_string(&id).unwrap();
            // serde_json emits a quoted string: strip quotes before FromStr.
            let unquoted = name.trim_matches('"');
            let parsed: ParameterId = unquoted.parse().unwrap_or_else(|e| {
                panic!("{id:?} serialises as {unquoted:?} but FromStr failed: {e}")
            });
            assert_eq!(parsed, id);
        }
    }

    #[test]
    fn from_str_rejects_unknown_variant() {
        let err = "NotAParameter".parse::<ParameterId>().unwrap_err();
        assert_eq!(err.0, "NotAParameter");
    }
}
