// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! FX-tab tables for the MIX page's EFFECTS panel. Each tab is one
//! slot in the master FX chain (SAT / DELAY / CHORUS / REVERB) plus
//! an OUT tab that hosts the master limiter and clock controls.

use brume_common::Scale;
use iced::Color;

#[derive(Debug, Clone, Copy)]
pub struct FxParamSpec {
    pub label: &'static str,
    /// Engine-side parameter key (matches the `param` field of
    /// `UiToEngine::SetFxParam`).
    pub id: &'static str,
    pub kind: FxParamKind,
    /// Engine-unit value at boot, before any echo arrives.
    pub default_value: f32,
    pub unit: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum FxParamKind {
    Slider { min: f32, max: f32, scale: Scale },
    Tap(&'static [&'static str]),
}

pub struct FxTab {
    pub title: &'static str,
    /// Engine slot name (matches the `slot` field of
    /// `UiToEngine::SetFxParam`). `None` on the OUT tab — its
    /// "params" are master clock + tempo controls handled out of band.
    pub slot: Option<&'static str>,
    /// Accent color for this tab — the active-tab pill backing on
    /// the FX sub-tab strip and the tap-enum value text.
    pub color: Color,
    pub params: &'static [FxParamSpec],
}

const fn lin(min: f32, max: f32) -> FxParamKind {
    FxParamKind::Slider {
        min,
        max,
        scale: Scale::Linear,
    }
}

pub const SAT: FxTab = FxTab {
    title: "SATURATOR",
    slot: Some("Saturator"),
    // #cc3388
    color: Color {
        r: 0.80,
        g: 0.20,
        b: 0.53,
        a: 1.0,
    },
    params: &[
        FxParamSpec {
            label: "TYPE",
            id: "type",
            kind: FxParamKind::Tap(&["Soft", "Hard", "Tape", "Tube"]),
            default_value: 0.0,
            unit: "",
        },
        FxParamSpec {
            label: "DRIVE",
            id: "drive",
            kind: lin(0.0, 1.0),
            default_value: 0.0,
            unit: "",
        },
        FxParamSpec {
            label: "MIX",
            id: "mix",
            kind: lin(0.0, 1.0),
            default_value: 1.0,
            unit: "",
        },
    ],
};

pub const DELAY: FxTab = FxTab {
    title: "DELAY",
    slot: Some("Delay"),
    // #00aacc
    color: Color {
        r: 0.0,
        g: 0.67,
        b: 0.80,
        a: 1.0,
    },
    params: &[
        FxParamSpec {
            label: "SYNC",
            id: "sync",
            kind: FxParamKind::Tap(&[
                "FREE", "MIDI", "1/1", "1/2", "1/4", "1/8", "1/16", "1/4d", "1/8d", "1/4t", "1/8t",
            ]),
            default_value: 0.0,
            unit: "",
        },
        FxParamSpec {
            label: "TIME",
            id: "time",
            kind: lin(10.0, 2000.0),
            default_value: 300.0,
            unit: "ms",
        },
        FxParamSpec {
            label: "FEEDBACK",
            id: "feedback",
            kind: lin(0.0, 0.95),
            default_value: 0.3,
            unit: "",
        },
        FxParamSpec {
            label: "DAMPING",
            id: "damping",
            kind: lin(0.0, 1.0),
            default_value: 0.3,
            unit: "",
        },
        FxParamSpec {
            label: "MIX",
            id: "mix",
            kind: lin(0.0, 1.0),
            default_value: 0.0,
            unit: "",
        },
    ],
};

pub const CHORUS: FxTab = FxTab {
    title: "CHORUS",
    slot: Some("Chorus"),
    // #00cc66
    color: Color {
        r: 0.0,
        g: 0.80,
        b: 0.40,
        a: 1.0,
    },
    params: &[
        FxParamSpec {
            label: "RATE",
            id: "rate",
            kind: lin(0.1, 5.0),
            default_value: 0.5,
            unit: "Hz",
        },
        FxParamSpec {
            label: "DEPTH",
            id: "depth",
            kind: lin(0.0, 1.0),
            default_value: 0.3,
            unit: "",
        },
        FxParamSpec {
            label: "MIX",
            id: "mix",
            kind: lin(0.0, 1.0),
            default_value: 0.0,
            unit: "",
        },
    ],
};

pub const REVERB: FxTab = FxTab {
    title: "REVERB",
    slot: Some("Reverb"),
    // #cc8800
    color: Color {
        r: 0.80,
        g: 0.53,
        b: 0.0,
        a: 1.0,
    },
    params: &[
        FxParamSpec {
            label: "TYPE",
            id: "type",
            kind: FxParamKind::Tap(&["Plate", "Room", "Hall", "Spring"]),
            default_value: 0.0,
            unit: "",
        },
        FxParamSpec {
            label: "PREDELAY",
            id: "predelay",
            kind: lin(0.0, 200.0),
            default_value: 20.0,
            unit: "ms",
        },
        FxParamSpec {
            label: "DECAY",
            id: "decay",
            kind: lin(0.1, 8.0),
            default_value: 2.0,
            unit: "",
        },
        FxParamSpec {
            label: "DAMPING",
            id: "damping",
            kind: lin(0.0, 1.0),
            default_value: 0.4,
            unit: "",
        },
        FxParamSpec {
            label: "MIX",
            id: "mix",
            kind: lin(0.0, 1.0),
            default_value: 0.0,
            unit: "",
        },
    ],
};

// The OUTPUT tab (BPM + CLOCK + LIMITER) was retired in issue #10
// phase B. BPM and CLOCK source moved to the MIDI page's CLOCK section
// where they read/write `NativeUi::transport_ui` and dispatch
// `UiToEngine::SetTempo` / `UiToEngine::SetClockMode` directly. LIMITER
// was placeholder UI not wired to the engine — re-add as a master-
// section control once `UiToEngine` grows a corresponding message.
pub const FX_TABS: &[&FxTab] = &[&SAT, &DELAY, &CHORUS, &REVERB];
