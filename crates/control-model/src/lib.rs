// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MIDI control model — channel routing and CC → parameter mapping.
//!
//! Provides configurable MIDI channel → part routing and CC → parameter
//! bindings. Shared between the MIDI input thread and the UI.

use brume_common::{ParameterId, Scale};
use serde::{Deserialize, Serialize};

/// Current on-disk schema version for `ControlMatrix`. Bump when the
/// JSON shape changes in a way the previous version's deserializer
/// would mishandle.
pub const CONTROL_MATRIX_SCHEMA_VERSION: u32 = 1;

/// Serde default for `ControlMatrix::version` — used when reading an
/// older JSON file that pre-dates the field. Pre-existing files are
/// treated as v1 (the schema as it stood when the field was
/// introduced). Files with `version` explicitly set deserialize that
/// value; the load gate then rejects anything newer than this build
/// supports.
fn default_schema_version() -> u32 {
    CONTROL_MATRIX_SCHEMA_VERSION
}

/// Maps MIDI channels (1-16) to engine parts.
///
/// Fully user-configurable. Default: CH 1→Part 0, CH 2→Part 1, CH 3→Part 2.
/// `None` means the channel is unassigned (messages ignored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiChannelMap {
    /// Index 0 = MIDI CH 1, index 15 = MIDI CH 16.
    channel_to_part: [Option<u8>; 16],
}

impl MidiChannelMap {
    /// Creates the default channel map: CH 1→Part 0, CH 2→Part 1, CH 3→Part 2, CH 4→Part 3.
    #[must_use]
    pub fn new() -> Self {
        let mut map = [None; 16];
        map[0] = Some(0); // CH 1 → Part 0 (FM)
        map[1] = Some(1); // CH 2 → Part 1 (Harmonic)
        map[2] = Some(2); // CH 3 → Part 2 (Timbral)
        map[3] = Some(3); // CH 4 → Part 3 (Granular)
        Self {
            channel_to_part: map,
        }
    }

    /// Returns the part number for a MIDI channel (0-indexed: channel 0 = CH 1).
    #[must_use]
    pub fn part_for_channel(&self, channel: u8) -> Option<u8> {
        self.channel_to_part
            .get(channel as usize)
            .copied()
            .flatten()
    }

    /// Sets the part assignment for a MIDI channel.
    pub fn set_channel(&mut self, channel: u8, part: Option<u8>) {
        if let Some(slot) = self.channel_to_part.get_mut(channel as usize) {
            *slot = part;
        }
    }

    /// Resets to default mapping.
    pub fn reset_defaults(&mut self) {
        *self = Self::new();
    }
}

impl Default for MidiChannelMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn from_json_rejects_future_version() {
        // Hand-rolled JSON that asserts a version this build doesn't
        // recognize. The gate must refuse loud rather than letting the
        // file partial-parse and round-trip back to disk with v2-only
        // fields silently dropped.
        let json = r#"{
          "version": 99,
          "channel_map": { "channel_to_part": [0,1,2,3,null,null,null,null,null,null,null,null,null,null,null,null] },
          "bindings": [],
          "fx_bindings": []
        }"#;
        let err = ControlMatrix::from_json(json).unwrap_err();
        match err {
            ControlMatrixError::UnsupportedVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, CONTROL_MATRIX_SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_treats_missing_version_as_current() {
        // A pre-version-field file (today's released format, in fact)
        // must still load — the serde default fills `version` with
        // CURRENT_SCHEMA_VERSION so the gate doesn't trip on its own.
        let json = r#"{
          "channel_map": { "channel_to_part": [0,1,2,3,null,null,null,null,null,null,null,null,null,null,null,null] },
          "bindings": [],
          "fx_bindings": []
        }"#;
        let m = ControlMatrix::from_json(json).expect("missing version should default-load");
        assert_eq!(m.version, CONTROL_MATRIX_SCHEMA_VERSION);
    }

    #[test]
    fn round_trip_includes_version() {
        // Belt-and-suspenders: a Default::default() ControlMatrix
        // serializes with `version`, and from_json round-trips it.
        let m = ControlMatrix::new();
        let json = serde_json::to_string(&m).unwrap();
        let back = ControlMatrix::from_json(&json).unwrap();
        assert_eq!(back.version, CONTROL_MATRIX_SCHEMA_VERSION);
    }
}

/// A single CC → parameter binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlBinding {
    /// MIDI CC number (0-127).
    pub cc_number: u8,
    /// MIDI channel filter: `None` = any channel, `Some(ch)` = specific channel (0-indexed).
    pub channel_filter: Option<u8>,
    /// Target parameter.
    pub destination: ParameterId,
    /// Parameter value at CC 0.
    pub range_min: f32,
    /// Parameter value at CC 127.
    pub range_max: f32,
    /// How CC 0..127 distributes across `[range_min, range_max]`.
    /// Defaults to Linear for files written before this field existed.
    #[serde(default = "default_scale")]
    pub scale: Scale,
}

fn default_scale() -> Scale {
    Scale::Linear
}

impl ControlBinding {
    /// Map a CC value (0-127) to the parameter's natural unit. Log
    /// matches `KnobBinding::apply` so a CC sweep tracks a slider
    /// drag of the same parameter.
    #[must_use]
    pub fn map_value(&self, cc_value: u8) -> f32 {
        let r = f32::from(cc_value) / 127.0;
        match self.scale {
            Scale::Linear => self.range_min + r * (self.range_max - self.range_min),
            Scale::Log if self.range_min > 0.0 && self.range_max > self.range_min => {
                (self.range_min.ln() + r * (self.range_max.ln() - self.range_min.ln())).exp()
            }
            // Degenerate Log range — fall back to linear rather than NaN.
            Scale::Log => self.range_min + r * (self.range_max - self.range_min),
        }
    }
}

/// The control matrix: CC bindings + channel routing.
///
/// Shared between the MIDI input thread (reads bindings) and the UI
/// (edits bindings). Wrapped in `Arc<Mutex>` or `Arc<RwLock>` at the
/// application level.
/// A CC → FX parameter binding (slot:param format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxCcBinding {
    pub cc_number: u8,
    /// "SlotName:param_name" format
    pub destination: String,
    pub range_min: f32,
    pub range_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMatrix {
    /// Schema version for forward compatibility. See
    /// [`CONTROL_MATRIX_SCHEMA_VERSION`]. Files written before this
    /// field existed deserialize as v1 via `default_schema_version`;
    /// `from_json` rejects anything declared newer than this build
    /// supports so a forward-version file isn't silently re-saved
    /// with unknown fields lost.
    #[serde(default = "default_schema_version")]
    pub version: u32,
    pub channel_map: MidiChannelMap,
    pub bindings: Vec<ControlBinding>,
    pub fx_bindings: Vec<FxCcBinding>,
}

/// Errors from `ControlMatrix::from_json`.
#[derive(Debug)]
pub enum ControlMatrixError {
    Parse(serde_json::Error),
    /// On-disk file declares a schema version this build doesn't know
    /// how to read. Same forward-compat policy as `PatchError::UnsupportedVersion`.
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
}

impl std::fmt::Display for ControlMatrixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "control-matrix parse error: {e}"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "control-matrix schema version {found} is newer than this build supports \
                 ({supported}); update brume to read this file"
            ),
        }
    }
}

impl std::error::Error for ControlMatrixError {}

impl ControlMatrix {
    /// Creates a control matrix with default channel routing and CC bindings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: CONTROL_MATRIX_SCHEMA_VERSION,
            channel_map: MidiChannelMap::new(),
            bindings: default_bindings(),
            fx_bindings: Vec::new(),
        }
    }

    /// Parses a JSON-encoded `ControlMatrix`, rejecting any file whose
    /// declared `version` is newer than `CONTROL_MATRIX_SCHEMA_VERSION`.
    /// Older files (and pre-version-field files, which deserialize as
    /// v1) load normally.
    pub fn from_json(s: &str) -> Result<Self, ControlMatrixError> {
        let m: Self = serde_json::from_str(s).map_err(ControlMatrixError::Parse)?;
        if m.version > CONTROL_MATRIX_SCHEMA_VERSION {
            return Err(ControlMatrixError::UnsupportedVersion {
                found: m.version,
                supported: CONTROL_MATRIX_SCHEMA_VERSION,
            });
        }
        Ok(m)
    }

    /// Looks up a CC event and returns (part, parameter_id, value) if matched.
    ///
    /// Returns `None` if the CC/channel combination has no binding, or if
    /// the channel has no part assigned.
    #[must_use]
    pub fn lookup(&self, channel: u8, cc: u8, cc_value: u8) -> Option<(u8, ParameterId, f32)> {
        let part = self.channel_map.part_for_channel(channel)?;

        let binding = self.bindings.iter().find(|b| {
            b.cc_number == cc
                && match b.channel_filter {
                    None => true,              // matches any channel
                    Some(ch) => ch == channel, // matches specific channel
                }
        })?;

        let value = binding.map_value(cc_value);
        Some((part, binding.destination, value))
    }

    /// Resets the matrix to its factory defaults — channel routing
    /// (CH 1→FM, CH 2→Harmonic, CH 3→Timbral, CH 4→Granular),
    /// the per-part default CC bindings (CC 3 ch1 → FmIndex, etc.),
    /// and no FX CC bindings. Exposed for the UI's "reset all
    /// bindings" action.
    pub fn reset_to_defaults(&mut self) {
        *self = Self::new();
    }

    /// Looks up FX CC bindings for a CC number.
    /// Returns (slot_name, param_name, value) pairs.
    ///
    /// Two `String` allocations per match are unavoidable here — the
    /// `UiToEngine::SetFxParam` wire shape owns its slot/param strings
    /// and the call sites consume the returned values into messages
    /// that get sent across a channel. The previous implementation
    /// also allocated a 2-element `Vec<&str>` per iteration via
    /// `splitn(2, ':').collect()`; this version uses `split_once(':')`
    /// to do the same parse without that intermediate Vec.
    #[must_use]
    pub fn lookup_fx(&self, cc: u8, cc_value: u8) -> Vec<(String, String, f32)> {
        self.fx_bindings
            .iter()
            .filter(|b| b.cc_number == cc)
            .filter_map(|b| {
                let (slot, param) = b.destination.split_once(':')?;
                let normalized = f32::from(cc_value) / 127.0;
                let value = b.range_min + normalized * (b.range_max - b.range_min);
                Some((slot.to_string(), param.to_string(), value))
            })
            .collect()
    }
}

impl Default for ControlMatrix {
    fn default() -> Self {
        Self::new()
    }
}

/// Default CC bindings for common controllers.
///
/// All defaults are explicitly per-part (`channel_filter: Some(part)`)
/// so they show up in the UI's CC MAPPING tab for that part and the
/// user can see + remove them. An earlier iteration shipped four
/// `channel_filter: None` defaults (CC 1 → FilterCutoff, CC 2 →
/// FilterResonance, CC 7 → MasterVolume, CC 11 → FilterEnvDepth)
/// intended as "any-channel" globals, but that path collided with
/// the UI's tab-filter convention (which treats `channel_filter`
/// as a part-tag) so the defaults were invisible from the
/// touchscreen — the user couldn't see or remove them. Worse, every
/// DAW that auto-fires mod-wheel / volume / expression CCs (Bitwig
/// during track-duplicate, for one) silently choked Brume's filter
/// or master volume. Kept the part-scoped defaults below; users who
/// want mod-wheel-to-cutoff can Learn it deliberately.
fn default_bindings() -> Vec<ControlBinding> {
    vec![
        // Part-specific: CC 3 on CH 1 → FM Index, CC 3 on CH 3 → Timbre
        ControlBinding {
            cc_number: 3,
            channel_filter: Some(0),
            destination: ParameterId::FmIndex,
            range_min: 0.0,
            range_max: 10.0,
            scale: Scale::Linear,
        },
        ControlBinding {
            cc_number: 3,
            channel_filter: Some(2),
            destination: ParameterId::Timbre,
            range_min: 0.0,
            range_max: 1.0,
            scale: Scale::Linear,
        },
        ControlBinding {
            cc_number: 4,
            channel_filter: Some(0),
            destination: ParameterId::FmFeedback,
            range_min: 0.0,
            range_max: 1.0,
            scale: Scale::Linear,
        },
        ControlBinding {
            cc_number: 74,
            channel_filter: Some(0),
            destination: ParameterId::Op2Ratio,
            range_min: 0.25,
            range_max: 16.0,
            scale: Scale::Log,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_channel_map() {
        let map = MidiChannelMap::new();
        assert_eq!(map.part_for_channel(0), Some(0)); // CH 1 → Part 0
        assert_eq!(map.part_for_channel(1), Some(1)); // CH 2 → Part 1
        assert_eq!(map.part_for_channel(2), Some(2)); // CH 3 → Part 2
        assert_eq!(map.part_for_channel(3), Some(3)); // CH 4 → Part 3
        assert_eq!(map.part_for_channel(4), None); // CH 5 → unassigned
        assert_eq!(map.part_for_channel(15), None); // CH 16 → unassigned
    }

    #[test]
    fn channel_map_reassignment() {
        let mut map = MidiChannelMap::new();
        map.set_channel(9, Some(0)); // CH 10 → Part 0 (drums → FM)
        assert_eq!(map.part_for_channel(9), Some(0));
    }

    #[test]
    fn binding_value_mapping_linear() {
        let b = ControlBinding {
            cc_number: 1,
            channel_filter: None,
            destination: ParameterId::FilterCutoff,
            range_min: 20.0,
            range_max: 20000.0,
            scale: Scale::Linear,
        };
        assert!((b.map_value(0) - 20.0).abs() < 0.1);
        assert!((b.map_value(127) - 20000.0).abs() < 1.0);
        assert!((b.map_value(64) - 10070.0).abs() < 100.0); // ~midpoint
    }

    #[test]
    fn binding_value_mapping_log() {
        // Log scale: matches what KnobBinding::apply does for a
        // log-scale slider drag. CC 64 lands close to (but slightly
        // above) the geometric mean because 64/127 = 0.5039, not
        // exactly 0.5. Linear midpoint would be ~10010 Hz; log
        // midpoint should be ≈ 632 Hz — a 16× difference, which is
        // the whole point of the scale awareness.
        let b = ControlBinding {
            cc_number: 1,
            channel_filter: None,
            destination: ParameterId::FilterCutoff,
            range_min: 20.0,
            range_max: 20000.0,
            scale: Scale::Log,
        };
        assert!((b.map_value(0) - 20.0).abs() < 0.1);
        assert!((b.map_value(127) - 20000.0).abs() < 1.0);
        let mid = b.map_value(64);
        let geo = (20.0_f32 * 20000.0).sqrt(); // ≈ 632.46
        assert!(
            (mid - geo).abs() / geo < 0.05,
            "log map at cc=64 ({mid}) should be within 5% of geometric mean ({geo})"
        );
        // And critically distinct from the linear midpoint.
        let linear_mid = 20.0 + (64.0 / 127.0) * (20000.0 - 20.0);
        assert!(
            mid < linear_mid * 0.1,
            "log midpoint ({mid}) must be much lower than linear midpoint ({linear_mid})"
        );
    }

    #[test]
    fn cc_1_does_not_auto_bind_to_filter_cutoff() {
        // Lock in the post-fix behaviour: CC 1 (mod wheel) is not
        // bound by default. Earlier iterations shipped CC 1 → FilterCutoff
        // as a `channel_filter: None` global, which made any DAW's
        // standard mod-wheel automation silently choke the filter.
        let matrix = ControlMatrix::new();
        assert!(matrix.lookup(0, 1, 127).is_none());
        assert!(matrix.lookup(0, 1, 0).is_none());
        // CC 2, 7, 11 used to be globals too; they're gone too now.
        assert!(matrix.lookup(0, 2, 127).is_none());
        assert!(matrix.lookup(0, 7, 127).is_none());
        assert!(matrix.lookup(0, 11, 127).is_none());
    }

    #[test]
    fn lookup_channel_specific_binding() {
        let matrix = ControlMatrix::new();
        // CC 3 on CH 1 → FmIndex
        let result = matrix.lookup(0, 3, 64);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ParameterId::FmIndex);

        // CC 3 on CH 3 → Timbre
        let result = matrix.lookup(2, 3, 64);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ParameterId::Timbre);
    }

    #[test]
    fn lookup_unassigned_channel() {
        let matrix = ControlMatrix::new();
        // CC 1 on CH 5 — channel has no part assignment
        let result = matrix.lookup(4, 1, 64);
        assert!(result.is_none());
    }

    #[test]
    fn lookup_unknown_cc() {
        let matrix = ControlMatrix::new();
        // CC 99 — no binding exists
        let result = matrix.lookup(0, 99, 64);
        assert!(result.is_none());
    }
}
