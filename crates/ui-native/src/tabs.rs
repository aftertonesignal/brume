// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Per-(engine, sub-tab) parameter tables. The UI reads these to
//! render the visible sub-tab's slider rows and to republish the
//! shared `KnobMapping` so nanoKONTROL2 knobs follow whatever's on
//! screen.

use brume_common::{KnobBinding, ParameterId, Scale};

/// One on-screen parameter row. Carries enough information to render
/// the slider, route a drag to the engine via `binding.apply`, and
/// position the slider visually from a `ParameterChanged` echo via
/// `binding.unapply`.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub label: &'static str,
    pub binding: KnobBinding,
    /// Engine-unit value the parameter sits at on a fresh boot, before
    /// any echo arrives.
    pub default_value: f32,
    /// `Some(["STACK", "TWIN", ...])` for tap-style enum selectors.
    /// `None` for continuous sliders. The native UI reads this to
    /// render the value field as a text label rather than a numeric
    /// readout.
    pub tap: Option<&'static [&'static str]>,
}

const fn lin(part: u8, id: ParameterId, min: f32, max: f32) -> KnobBinding {
    KnobBinding {
        part,
        id,
        min,
        max,
        scale: Scale::Linear,
    }
}
const fn log(part: u8, id: ParameterId, min: f32, max: f32) -> KnobBinding {
    KnobBinding {
        part,
        id,
        min,
        max,
        scale: Scale::Log,
    }
}

pub const FM_ALGOS: [&str; 12] = [
    "STACK", "DOUBLE", "TWIN", "TRIO", "FAN-IN", "BRANCH", "HYBRID", "ADDITIVE", "PAIRS", "TOWER",
    "FUNNEL", "CHAIN",
];

// ── FM (part 0) ───────────────────────────────────────────────────

const FM_ALGO: &[ParamSpec] = &[
    ParamSpec {
        label: "ALGO",
        binding: lin(0, ParameterId::Algorithm, 0.0, 11.0),
        default_value: 0.0,
        tap: Some(&FM_ALGOS),
    },
    ParamSpec {
        label: "FDBK",
        binding: lin(0, ParameterId::FmFeedback, 0.0, 1.0),
        default_value: 0.3,
        tap: None,
    },
];

const FM_RATIOS: &[ParamSpec] = &[
    ParamSpec {
        label: "OP1 RATIO",
        binding: log(0, ParameterId::Op1Ratio, 0.25, 16.0),
        default_value: 1.0,
        tap: None,
    },
    ParamSpec {
        label: "OP2 RATIO",
        binding: log(0, ParameterId::Op2Ratio, 0.25, 16.0),
        default_value: 2.0,
        tap: None,
    },
    ParamSpec {
        label: "OP3 RATIO",
        binding: log(0, ParameterId::Op3Ratio, 0.25, 16.0),
        default_value: 3.0,
        tap: None,
    },
    ParamSpec {
        label: "OP4 RATIO",
        binding: log(0, ParameterId::Op4Ratio, 0.25, 16.0),
        default_value: 4.0,
        tap: None,
    },
    ParamSpec {
        label: "OP5 RATIO",
        binding: log(0, ParameterId::Op5Ratio, 0.25, 16.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "OP6 RATIO",
        binding: log(0, ParameterId::Op6Ratio, 0.25, 16.0),
        default_value: 6.0,
        tap: None,
    },
];

const FM_LEVELS: &[ParamSpec] = &[
    ParamSpec {
        label: "OP1 LVL",
        binding: lin(0, ParameterId::Op1Level, 0.0, 1.0),
        default_value: 0.80,
        tap: None,
    },
    ParamSpec {
        label: "OP2 LVL",
        binding: lin(0, ParameterId::Op2Level, 0.0, 1.0),
        default_value: 0.40,
        tap: None,
    },
    ParamSpec {
        label: "OP3 LVL",
        binding: lin(0, ParameterId::Op3Level, 0.0, 1.0),
        default_value: 0.30,
        tap: None,
    },
    ParamSpec {
        label: "OP4 LVL",
        binding: lin(0, ParameterId::Op4Level, 0.0, 1.0),
        default_value: 0.25,
        tap: None,
    },
    ParamSpec {
        label: "OP5 LVL",
        binding: lin(0, ParameterId::Op5Level, 0.0, 1.0),
        default_value: 0.20,
        tap: None,
    },
    ParamSpec {
        label: "OP6 LVL",
        binding: lin(0, ParameterId::Op6Level, 0.0, 1.0),
        default_value: 0.15,
        tap: None,
    },
];

const FM_MOD: &[ParamSpec] = &[
    ParamSpec {
        label: "FM INDEX",
        binding: lin(0, ParameterId::FmIndex, 0.0, 2.0),
        default_value: 0.7,
        tap: None,
    },
    ParamSpec {
        label: "FM ENV",
        binding: lin(0, ParameterId::FmIndexEnvDepth, 0.0, 2.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "FM.ATK",
        binding: log(0, ParameterId::FmIndexEnvAttack, 1.0, 2000.0),
        default_value: 2.0,
        tap: None,
    },
    ParamSpec {
        label: "FM.DEC",
        binding: log(0, ParameterId::FmIndexEnvDecay, 1.0, 5000.0),
        default_value: 800.0,
        tap: None,
    },
    ParamSpec {
        label: "FM.SUS",
        binding: lin(0, ParameterId::FmIndexEnvSustain, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "FM.REL",
        binding: log(0, ParameterId::FmIndexEnvRelease, 1.0, 5000.0),
        default_value: 400.0,
        tap: None,
    },
];

const FM_FILTER: &[ParamSpec] = &[
    ParamSpec {
        label: "CUTOFF",
        binding: log(0, ParameterId::FilterCutoff, 200.0, 20000.0),
        default_value: 18000.0,
        tap: None,
    },
    ParamSpec {
        label: "RESO",
        binding: lin(0, ParameterId::FilterResonance, 0.0, 1.0),
        default_value: 0.15,
        tap: None,
    },
    ParamSpec {
        label: "ENV DEP",
        binding: lin(0, ParameterId::FilterEnvDepth, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "F.ATK",
        binding: log(0, ParameterId::FilterEnvAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "F.DEC",
        binding: log(0, ParameterId::FilterEnvDecay, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
    ParamSpec {
        label: "F.SUS",
        binding: lin(0, ParameterId::FilterEnvSustain, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "F.REL",
        binding: log(0, ParameterId::FilterEnvRelease, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
];

const FM_AMP: &[ParamSpec] = &[
    ParamSpec {
        label: "ATTACK",
        binding: log(0, ParameterId::AmpAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "DECAY",
        binding: log(0, ParameterId::AmpDecay, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
    ParamSpec {
        label: "SUSTAIN",
        binding: lin(0, ParameterId::AmpSustain, 0.0, 1.0),
        default_value: 0.7,
        tap: None,
    },
    ParamSpec {
        label: "RELEASE",
        binding: log(0, ParameterId::AmpRelease, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
];

pub const FM_TABS: &[(&str, &[ParamSpec])] = &[
    ("ALGO", FM_ALGO),
    ("RATIOS", FM_RATIOS),
    ("LEVELS", FM_LEVELS),
    ("MOD", FM_MOD),
    ("FILTER", FM_FILTER),
    ("AMP", FM_AMP),
];

// ── HARMONIC (part 1) ─────────────────────────────────────────────

const HARMONIC_HARMONICS: &[ParamSpec] = &[
    ParamSpec {
        label: "H1",
        binding: lin(1, ParameterId::HarmonicLevel1, 0.0, 1.0),
        default_value: 1.00,
        tap: None,
    },
    ParamSpec {
        label: "H2",
        binding: lin(1, ParameterId::HarmonicLevel2, 0.0, 1.0),
        default_value: 0.50,
        tap: None,
    },
    ParamSpec {
        label: "H3",
        binding: lin(1, ParameterId::HarmonicLevel3, 0.0, 1.0),
        default_value: 0.30,
        tap: None,
    },
    ParamSpec {
        label: "H4",
        binding: lin(1, ParameterId::HarmonicLevel4, 0.0, 1.0),
        default_value: 0.25,
        tap: None,
    },
    ParamSpec {
        label: "H5",
        binding: lin(1, ParameterId::HarmonicLevel5, 0.0, 1.0),
        default_value: 0.20,
        tap: None,
    },
    ParamSpec {
        label: "H6",
        binding: lin(1, ParameterId::HarmonicLevel6, 0.0, 1.0),
        default_value: 0.15,
        tap: None,
    },
    ParamSpec {
        label: "H7",
        binding: lin(1, ParameterId::HarmonicLevel7, 0.0, 1.0),
        default_value: 0.10,
        tap: None,
    },
    ParamSpec {
        label: "H8",
        binding: lin(1, ParameterId::HarmonicLevel8, 0.0, 1.0),
        default_value: 0.08,
        tap: None,
    },
];

const HARMONIC_SCAN: &[ParamSpec] = &[
    ParamSpec {
        label: "CENTER",
        binding: lin(1, ParameterId::ScanCenter, 0.0, 1.0),
        default_value: 0.5,
        tap: None,
    },
    ParamSpec {
        label: "WIDTH",
        binding: lin(1, ParameterId::ScanWidth, 0.0, 1.0),
        default_value: 1.0,
        tap: None,
    },
    ParamSpec {
        label: "MORPH",
        binding: lin(1, ParameterId::HarmonicMorph, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "SPREAD",
        binding: lin(1, ParameterId::HarmonicSpread, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
];

const HARMONIC_SPECTRUM: &[ParamSpec] = &[
    ParamSpec {
        label: "TILT",
        binding: lin(1, ParameterId::HarmonicTilt, -1.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "ODD/EVN",
        binding: lin(1, ParameterId::HarmonicOddEven, 0.0, 1.0),
        default_value: 0.5,
        tap: None,
    },
    ParamSpec {
        label: "INHARM",
        binding: lin(1, ParameterId::Inharmonicity, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
];

const HARMONIC_FM: &[ParamSpec] = &[
    ParamSpec {
        label: "FM DEP",
        binding: lin(1, ParameterId::HarmonicFmDepth, 0.0, 10.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "FM RAT",
        binding: lin(1, ParameterId::HarmonicFmRatio, 0.5, 16.0),
        default_value: 2.0,
        tap: None,
    },
];

const HARMONIC_FILTER: &[ParamSpec] = &[
    ParamSpec {
        label: "CUTOFF",
        binding: log(1, ParameterId::FilterCutoff, 200.0, 20000.0),
        default_value: 8000.0,
        tap: None,
    },
    ParamSpec {
        label: "RESO",
        binding: lin(1, ParameterId::FilterResonance, 0.0, 1.0),
        default_value: 0.10,
        tap: None,
    },
    ParamSpec {
        label: "ENV DEP",
        binding: lin(1, ParameterId::FilterEnvDepth, 0.0, 1.0),
        default_value: 0.30,
        tap: None,
    },
    ParamSpec {
        label: "F.ATK",
        binding: log(1, ParameterId::FilterEnvAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "F.DEC",
        binding: log(1, ParameterId::FilterEnvDecay, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
    ParamSpec {
        label: "F.SUS",
        binding: lin(1, ParameterId::FilterEnvSustain, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "F.REL",
        binding: log(1, ParameterId::FilterEnvRelease, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
];

const HARMONIC_AMP: &[ParamSpec] = &[
    ParamSpec {
        label: "ATTACK",
        binding: log(1, ParameterId::AmpAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "DECAY",
        binding: log(1, ParameterId::AmpDecay, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
    ParamSpec {
        label: "SUSTAIN",
        binding: lin(1, ParameterId::AmpSustain, 0.0, 1.0),
        default_value: 0.7,
        tap: None,
    },
    ParamSpec {
        label: "RELEASE",
        binding: log(1, ParameterId::AmpRelease, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
];

pub const HARMONIC_TABS: &[(&str, &[ParamSpec])] = &[
    ("HARMONICS", HARMONIC_HARMONICS),
    ("SCAN", HARMONIC_SCAN),
    ("SPECTRUM", HARMONIC_SPECTRUM),
    ("FM", HARMONIC_FM),
    ("FILTER", HARMONIC_FILTER),
    ("AMP", HARMONIC_AMP),
];

// ── TIMBRAL (part 2) ──────────────────────────────────────────────

const TIMBRAL_SHAPE: &[ParamSpec] = &[
    ParamSpec {
        label: "TIMBRE",
        binding: lin(2, ParameterId::Timbre, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "SYMMETRY",
        binding: lin(2, ParameterId::Symmetry, -1.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "STAGES",
        binding: lin(2, ParameterId::MultiplierStages, 1.0, 4.0),
        default_value: 1.0,
        tap: None,
    },
    ParamSpec {
        label: "FEEDBK",
        binding: lin(2, ParameterId::TimbralFeedback, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
];

const TIMBRAL_FM: &[ParamSpec] = &[
    ParamSpec {
        label: "FM DEP",
        binding: lin(2, ParameterId::TimbralFmDepth, 0.0, 10.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "FM RAT",
        binding: lin(2, ParameterId::TimbralFmRatio, 0.5, 16.0),
        default_value: 2.0,
        tap: None,
    },
];

const TIMBRAL_SUB: &[ParamSpec] = &[ParamSpec {
    label: "SUB LVL",
    binding: lin(2, ParameterId::SubLevel, 0.0, 1.0),
    default_value: 0.0,
    tap: None,
}];

const TIMBRAL_FILTER: &[ParamSpec] = &[
    ParamSpec {
        label: "CUTOFF",
        binding: log(2, ParameterId::FilterCutoff, 200.0, 20000.0),
        default_value: 8000.0,
        tap: None,
    },
    ParamSpec {
        label: "RESO",
        binding: lin(2, ParameterId::FilterResonance, 0.0, 1.0),
        default_value: 0.15,
        tap: None,
    },
    ParamSpec {
        label: "ENV DEP",
        binding: lin(2, ParameterId::FilterEnvDepth, 0.0, 1.0),
        default_value: 0.30,
        tap: None,
    },
    ParamSpec {
        label: "F.ATK",
        binding: log(2, ParameterId::FilterEnvAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "F.DEC",
        binding: log(2, ParameterId::FilterEnvDecay, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
    ParamSpec {
        label: "F.SUS",
        binding: lin(2, ParameterId::FilterEnvSustain, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "F.REL",
        binding: log(2, ParameterId::FilterEnvRelease, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
];

const TIMBRAL_AMP: &[ParamSpec] = &[
    ParamSpec {
        label: "ATTACK",
        binding: log(2, ParameterId::AmpAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "DECAY",
        binding: log(2, ParameterId::AmpDecay, 1.0, 5000.0),
        default_value: 200.0,
        tap: None,
    },
    ParamSpec {
        label: "SUSTAIN",
        binding: lin(2, ParameterId::AmpSustain, 0.0, 1.0),
        default_value: 0.7,
        tap: None,
    },
    ParamSpec {
        label: "RELEASE",
        binding: log(2, ParameterId::AmpRelease, 1.0, 5000.0),
        default_value: 300.0,
        tap: None,
    },
];

pub const TIMBRAL_TABS: &[(&str, &[ParamSpec])] = &[
    ("SHAPE", TIMBRAL_SHAPE),
    ("FM", TIMBRAL_FM),
    ("SUB", TIMBRAL_SUB),
    ("FILTER", TIMBRAL_FILTER),
    ("AMP", TIMBRAL_AMP),
];

// ── GRANULAR (part 3) ─────────────────────────────────────────────

const GRANULAR_CLOUD: &[ParamSpec] = &[
    ParamSpec {
        label: "DENSITY",
        binding: lin(3, ParameterId::GranularDensity, 0.0, 1.0),
        default_value: 0.3,
        tap: None,
    },
    ParamSpec {
        label: "SIZE",
        binding: lin(3, ParameterId::GranularGrainSize, 0.0, 1.0),
        default_value: 0.2,
        tap: None,
    },
    ParamSpec {
        label: "SCATTER",
        binding: lin(3, ParameterId::GranularScatter, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "SPREAD",
        binding: lin(3, ParameterId::GranularSpread, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "DRIFT",
        binding: lin(3, ParameterId::GranularDrift, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "POSITION",
        binding: lin(3, ParameterId::GranularPosition, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
];

const GRANULAR_SHAPE: &[ParamSpec] = &[
    ParamSpec {
        label: "MORPH",
        binding: lin(3, ParameterId::GranularMorph, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "ENVELOPE",
        binding: lin(3, ParameterId::GranularGrainShape, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
];

const GRANULAR_FM: &[ParamSpec] = &[
    ParamSpec {
        label: "FM DEP",
        binding: lin(3, ParameterId::GranularFmDepth, 0.0, 10.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "FM RAT",
        binding: lin(3, ParameterId::GranularFmRatio, 0.5, 16.0),
        default_value: 2.0,
        tap: None,
    },
];

const GRANULAR_FILTER: &[ParamSpec] = &[
    ParamSpec {
        label: "CUTOFF",
        binding: log(3, ParameterId::FilterCutoff, 200.0, 20000.0),
        default_value: 4000.0,
        tap: None,
    },
    ParamSpec {
        label: "RESO",
        binding: lin(3, ParameterId::FilterResonance, 0.0, 1.0),
        default_value: 0.15,
        tap: None,
    },
    ParamSpec {
        label: "ENV DEP",
        binding: lin(3, ParameterId::FilterEnvDepth, 0.0, 1.0),
        default_value: 0.20,
        tap: None,
    },
    ParamSpec {
        label: "F.ATK",
        binding: log(3, ParameterId::FilterEnvAttack, 1.0, 2000.0),
        default_value: 5.0,
        tap: None,
    },
    ParamSpec {
        label: "F.DEC",
        binding: log(3, ParameterId::FilterEnvDecay, 1.0, 5000.0),
        default_value: 500.0,
        tap: None,
    },
    ParamSpec {
        label: "F.SUS",
        binding: lin(3, ParameterId::FilterEnvSustain, 0.0, 1.0),
        default_value: 0.0,
        tap: None,
    },
    ParamSpec {
        label: "F.REL",
        binding: log(3, ParameterId::FilterEnvRelease, 1.0, 5000.0),
        default_value: 400.0,
        tap: None,
    },
];

const GRANULAR_AMP: &[ParamSpec] = &[
    ParamSpec {
        label: "ATTACK",
        binding: log(3, ParameterId::AmpAttack, 1.0, 2000.0),
        default_value: 20.0,
        tap: None,
    },
    ParamSpec {
        label: "DECAY",
        binding: log(3, ParameterId::AmpDecay, 1.0, 5000.0),
        default_value: 500.0,
        tap: None,
    },
    ParamSpec {
        label: "SUSTAIN",
        binding: lin(3, ParameterId::AmpSustain, 0.0, 1.0),
        default_value: 0.8,
        tap: None,
    },
    ParamSpec {
        label: "RELEASE",
        binding: log(3, ParameterId::AmpRelease, 1.0, 5000.0),
        default_value: 800.0,
        tap: None,
    },
];

pub const GRANULAR_TABS: &[(&str, &[ParamSpec])] = &[
    ("CLOUD", GRANULAR_CLOUD),
    ("SHAPE", GRANULAR_SHAPE),
    ("FM", GRANULAR_FM),
    ("FILTER", GRANULAR_FILTER),
    ("AMP", GRANULAR_AMP),
];
