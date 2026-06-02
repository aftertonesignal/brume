// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Patch serialization, storage, and recall for Brume.
//!
//! A patch captures the complete state of a single part's oscillator,
//! filter, and envelope (or the global FX chain's parameters, for
//! `mode: "fx"` patches). The schema is versioned so patches from older
//! firmware can be migrated forward.
//!
//! # Storage layout
//!
//! ```text
//! ~/.brume/library/
//!   fm/          warm-pad.json          ← single-mode patches
//!   harmonic/    organ-8.json
//!   timbral/     growl-bass.json
//!   granular/    cloud-01.json
//!   fx/          cathedral.json
//!   perf/        obsidian-evening.json  ← whole-instrument snapshots
//! ```
//!
//! # Future expansion
//!
//! The `Patch` struct uses `#[serde(default)]` on optional sections so
//! new capabilities (modulation scenes, richer effects state, multi-part
//! performances) can be added without breaking existing patches.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Current schema version. Bump when the patch format changes.
const SCHEMA_VERSION: u32 = 1;

/// Canonical library mode directories — one per patch kind. The first
/// slot ("fm") is also the rename target for any legacy "complex/"
/// directory found at library-open time.
const LIBRARY_MODES: &[&str] = &["fm", "harmonic", "timbral", "granular", "fx"];

/// Directory name for whole-instrument Perf snapshots. Sits alongside
/// the per-mode patch directories under `~/.brume/library/`.
const PERF_SUBDIR: &str = "perf";

/// Default install path for the factory presets shipped with Brume.
/// `brumectl install` extracts the release artifact's `factory/` tree
/// here; the runtime treats it as a read-only library layer alongside
/// the user's writable root. Override with `BRUME_FACTORY_DIR`
/// (colon-separated paths) for dev workflow on hosts that don't have
/// the system path provisioned.
const DEFAULT_FACTORY_ROOT: &str = "/usr/share/brume/factory";

/// Author tag stamped onto every factory preset in source. The cleanup
/// migration uses this — combined with a parameter-byte equality check
/// against the corresponding factory file — to identify user-dir files
/// left over from the old seed-on-first-run model and remove them.
/// Patches the user has actually edited (params differ) are kept.
const FACTORY_AUTHOR: &str = "Brume Factory";

/// Legacy marker file from the seed-on-first-run model. Still cleaned
/// up by `cleanup_seeded_factory_files` so a fresh open doesn't leave
/// it dangling under the library root.
const LEGACY_FACTORY_MARKER: &str = ".factory_seeded_1";

/// Provenance of an entry returned by `PatchLibrary::list`.
///
/// Drives the LIBRARY UI's "factory" badge and the disabled-DELETE
/// state. `User` files live under the user library root and are
/// freely writable; `Factory` files live under one of the factory
/// roots and are read-only — the runtime never writes there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSource {
    User,
    Factory,
}

/// UTC "now" as an ISO 8601 / RFC 3339 string (e.g. "2026-04-24T19:35:00Z").
///
/// Hand-rolled to keep this crate zero-dep beyond serde. Uses Howard
/// Hinnant's civil_from_days algorithm for the date portion; accurate
/// for any year within the SystemTime range we'll ever hit.
fn now_iso8601() -> String {
    let total_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (total_secs / 86_400) as i64;
    let sod = total_secs % 86_400;
    let h = sod / 3_600;
    let m = (sod % 3_600) / 60;
    let s = sod % 60;
    // civil_from_days: days-since-1970-01-01 → (Y, M, D).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year_shifted = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 {
        year_shifted + 1
    } else {
        year_shifted
    };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// A complete patch that can be saved and recalled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Schema version for forward compatibility.
    pub version: u32,

    /// Human-readable name shown in the library browser.
    pub name: String,

    /// Which oscillator mode this patch targets
    /// ("fm" | "harmonic" | "timbral" | "granular" | "fx").
    pub mode: String,

    /// Oscillator + filter + envelope parameter values (or the FX chain's
    /// parameters, for `mode: "fx"`). Keys are `ParameterId` names
    /// (e.g. "FmIndex", "FilterCutoff") or FX slot-qualified names.
    pub params: BTreeMap<String, f64>,

    /// Tags for browsing / search. The engine mode is auto-inserted on
    /// save (e.g. `["fm"]`) so tag filters work from day one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// ISO 8601 UTC timestamp — set at creation, preserved across
    /// subsequent saves. Backfilled on first save when missing
    /// (older files written before metadata fields existed).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,

    /// ISO 8601 UTC timestamp — refreshed on every save.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_at: String,

    /// The Brume version that wrote this patch (CARGO_PKG_VERSION).
    /// Provenance only; never used for migration decisions (that's `version`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub brume_version: String,

    /// Optional author attribution (free-form string, no auth meaning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Optional modulation state (future).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulation: Option<serde_json::Value>,

    /// Optional richer effects state (future — the core FX params already
    /// round-trip through `params` for `mode: "fx"` patches).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<serde_json::Value>,

    /// Optional notes from the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Patch {
    /// Creates a new patch with the given name and mode.
    ///
    /// Populates `created_at` / `modified_at` with the current UTC time,
    /// `brume_version` with this workspace's `CARGO_PKG_VERSION`, and
    /// auto-tags the patch with its mode (users can add more via the
    /// library UI or brumectl).
    #[must_use]
    pub fn new(name: String, mode: String, params: BTreeMap<String, f64>) -> Self {
        let now = now_iso8601();
        let mut tags = Vec::new();
        if !mode.is_empty() {
            tags.push(mode.clone());
        }
        Self {
            version: SCHEMA_VERSION,
            name,
            mode,
            params,
            tags,
            created_at: now.clone(),
            modified_at: now,
            brume_version: env!("CARGO_PKG_VERSION").to_string(),
            author: None,
            modulation: None,
            effects: None,
            notes: None,
        }
    }
}

// ─── Perf ──────────────────────────────────────────────────────────
// Whole-instrument snapshot — four engine patches + FX + mixer in one
// recallable unit. Symmetric with Patch on the metadata envelope
// (version/name/tags/timestamps/brume_version/author/notes) but the
// body is a structured composite rather than a flat `params` map.

/// A whole-instrument snapshot recallable as a single unit.
///
/// A Perf embeds copies of each part's current patch state rather than
/// referencing external `Patch` files by name — keeps the Perf self-
/// contained (rename/delete of a source Patch can't silently break it).
/// A non-authoritative `source_patch` breadcrumb on each slot lets the
/// UI show "this slot came from obsidian-bells" but it never drives
/// load behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Perf {
    /// Schema version for forward compatibility. Shares the namespace
    /// with `Patch::version` (v1 today); split the constant if one
    /// format ever needs to evolve independently.
    pub version: u32,

    /// Human-readable name shown in the LIBRARY PERF tab.
    pub name: String,

    /// Tags for browsing / search. `"perf"` is auto-inserted on new()
    /// so the tag filter can distinguish Perfs from Patches uniformly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// ISO 8601 UTC timestamp — set at creation, preserved across
    /// subsequent saves. Backfilled on first save when missing
    /// (older files written before metadata fields existed).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,

    /// ISO 8601 UTC timestamp — refreshed on every save.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub modified_at: String,

    /// CARGO_PKG_VERSION captured at write time. Provenance only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub brume_version: String,

    /// Optional free-form author attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Optional longer-form description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Embedded patches for the four parts. Index 0 = FM, 1 = Harmonic,
    /// 2 = Timbral, 3 = Granular. `None` = part inactive in this Perf.
    pub parts: [Option<PartSnapshot>; 4],

    /// Embedded FX chain state (saturator / chorus / delay / reverb).
    /// `None` leaves the active FX chain alone on Perf load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<FxSnapshot>,

    /// Mixer snapshot — per-part levels/mutes/sends + master volume.
    pub mixer: MixerSnapshot,

    /// Optional modulation state (future — kept opaque until a concrete
    /// modulation-snapshot schema is worth defining).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulation: Option<serde_json::Value>,

    /// Optional user notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// One part's embedded patch state within a Perf. Same parameter key
/// space as a single-mode `Patch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartSnapshot {
    /// Oscillator mode ("fm" | "harmonic" | "timbral" | "granular").
    pub mode: String,

    /// Parameter map, keyed by `ParameterId` string.
    pub params: BTreeMap<String, f64>,

    /// Optional breadcrumb: the name of the source Patch this slot was
    /// built from. Informational only; ignored by load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_patch: Option<String>,
}

/// Embedded FX chain state within a Perf. Same parameter key space as
/// an `fx`-mode `Patch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxSnapshot {
    /// Parameter map, keyed by slot-qualified FX parameter names.
    pub params: BTreeMap<String, f64>,

    /// Optional breadcrumb: source FX Patch name. Informational only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_patch: Option<String>,
}

/// Mixer state within a Perf. All arrays are indexed 0..=3 matching the
/// engine's part order (FM / Harmonic / Timbral / Granular).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixerSnapshot {
    /// Per-part output levels, 0.0..=1.0.
    pub levels: [f32; 4],

    /// Per-part mute flags.
    pub mutes: [bool; 4],

    /// Per-part delay-send levels, 0.0..=1.0.
    pub delay_sends: [f32; 4],

    /// Per-part reverb-send levels, 0.0..=1.0.
    pub reverb_sends: [f32; 4],

    /// Master output volume, 0.0..=1.0.
    pub master_volume: f32,
}

impl Default for MixerSnapshot {
    fn default() -> Self {
        Self {
            levels: [0.8; 4],
            mutes: [false; 4],
            delay_sends: [0.0; 4],
            reverb_sends: [0.0; 4],
            master_volume: 0.8,
        }
    }
}

impl Perf {
    /// Creates a new Perf with the given name and composition.
    ///
    /// Populates `created_at` / `modified_at` with the current UTC time,
    /// `brume_version` with this workspace's `CARGO_PKG_VERSION`, and
    /// auto-tags with `"perf"` so the browse-by-tag filter can
    /// distinguish it from single-mode Patches.
    #[must_use]
    pub fn new(
        name: String,
        parts: [Option<PartSnapshot>; 4],
        fx: Option<FxSnapshot>,
        mixer: MixerSnapshot,
    ) -> Self {
        let now = now_iso8601();
        Self {
            version: SCHEMA_VERSION,
            name,
            tags: vec!["perf".to_string()],
            created_at: now.clone(),
            modified_at: now,
            brume_version: env!("CARGO_PKG_VERSION").to_string(),
            author: None,
            description: None,
            parts,
            fx,
            mixer,
            modulation: None,
            notes: None,
        }
    }
}

/// Manages the patch library on disk.
///
/// A library has one writable user root (where `save` lands) and zero
/// or more read-only factory roots (where shipped presets live). All
/// `list` / `load` / `patch_source` calls walk both layers; `save`
/// always targets the user root, and `delete` refuses entries that
/// only exist under a factory root.
pub struct PatchLibrary {
    root: PathBuf,
    factory_roots: Vec<PathBuf>,
}

impl PatchLibrary {
    /// Opens or creates a library at the given user root, with the
    /// default factory root list (`/usr/share/brume/factory`, or
    /// `BRUME_FACTORY_DIR` if set — colon-separated for multiple).
    pub fn open(root: &Path) -> std::io::Result<Self> {
        Self::open_with_factory_roots(root, default_factory_roots())
    }

    /// Like `open` but with an explicit factory root list. Tests use
    /// `vec![]` to keep the factory layer out of the picture; the
    /// release build relies on the default.
    ///
    /// Ensures every canonical mode directory exists, runs the
    /// `complex/` → `fm/` migration, and cleans up any leftover
    /// files from the old seed-on-first-run model (idempotent).
    pub fn open_with_factory_roots(
        root: &Path,
        factory_roots: Vec<PathBuf>,
    ) -> std::io::Result<Self> {
        for mode in LIBRARY_MODES {
            std::fs::create_dir_all(root.join(mode))?;
        }
        std::fs::create_dir_all(root.join(PERF_SUBDIR))?;
        migrate_complex_to_fm(root)?;
        let lib = Self {
            root: root.to_path_buf(),
            factory_roots,
        };
        cleanup_seeded_factory_files(&lib);
        Ok(lib)
    }

    /// Returns the library root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the configured factory root paths.
    #[must_use]
    pub fn factory_roots(&self) -> &[PathBuf] {
        &self.factory_roots
    }

    /// Lists all patch names for a given mode, sorted alphabetically.
    /// Names from the user root and every factory root are merged;
    /// duplicates collapse to one entry (the user copy wins on
    /// `load`, but `list` doesn't distinguish — call `patch_source`
    /// per name if the UI needs to badge factory entries).
    #[must_use]
    pub fn list(&self, mode: &str) -> Vec<String> {
        if !is_safe_name(mode) {
            return Vec::new();
        }
        let mut names = std::collections::BTreeSet::new();
        for dir in std::iter::once(&self.root).chain(self.factory_roots.iter()) {
            collect_json_stems(&dir.join(mode), &mut names);
        }
        names.into_iter().collect()
    }

    /// Reports whether a patch lives in the user root or a factory
    /// root. `None` means no file with that name exists in either
    /// layer. Used by the LIBRARY UI to disable DELETE on factory
    /// entries and show a small badge.
    #[must_use]
    pub fn patch_source(&self, mode: &str, name: &str) -> Option<PatchSource> {
        let user_path = self.user_path(mode, name).ok()?;
        if user_path.is_file() {
            return Some(PatchSource::User);
        }
        if self.factory_path(mode, name).is_some() {
            return Some(PatchSource::Factory);
        }
        None
    }

    /// Loads a patch by mode and name. User root takes precedence
    /// over factory roots, so a user-edited shadow copy hides the
    /// factory original.
    ///
    /// Refuses files whose declared `version` is newer than this
    /// build supports — partial parsing of a v2-as-v1 file would
    /// silently drop fields the v1 deserializer doesn't know about,
    /// and the next save would re-emit a v1 file with that data
    /// lost.
    pub fn load(&self, mode: &str, name: &str) -> Result<Patch, PatchError> {
        let user_path = self.user_path(mode, name)?;
        let path = if user_path.is_file() {
            user_path
        } else if let Some(factory) = self.factory_path(mode, name) {
            factory
        } else {
            return Err(PatchError::Io(
                user_path.display().to_string(),
                std::io::Error::new(std::io::ErrorKind::NotFound, "patch not found"),
            ));
        };
        let json = std::fs::read_to_string(&path)
            .map_err(|e| PatchError::Io(path.display().to_string(), e))?;
        let patch: Patch = serde_json::from_str(&json).map_err(PatchError::Parse)?;
        if patch.version > SCHEMA_VERSION {
            return Err(PatchError::UnsupportedVersion {
                path: path.display().to_string(),
                found: patch.version,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(patch)
    }

    /// Saves a patch into the user root. Overwrites a pre-existing
    /// user copy; if a same-named factory copy exists in any
    /// factory root, this creates a shadow under the user root that
    /// takes precedence on `load`.
    ///
    /// Refreshes `modified_at` to the current UTC time, backfills
    /// `created_at` if missing, stamps `brume_version` if missing.
    /// Uses write-to-temp + rename for atomicity (survives power
    /// loss).
    pub fn save(&self, patch: &Patch) -> Result<(), PatchError> {
        let path = self.user_path(&patch.mode, &patch.name)?;
        let mut to_write = patch.clone();
        let now = now_iso8601();
        if to_write.created_at.is_empty() {
            to_write.created_at = now.clone();
        }
        to_write.modified_at = now;
        if to_write.brume_version.is_empty() {
            to_write.brume_version = env!("CARGO_PKG_VERSION").to_string();
        }
        let json = serde_json::to_string_pretty(&to_write).map_err(PatchError::Parse)?;

        // Atomic write: temp file + rename
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| PatchError::Io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, &path).map_err(|e| PatchError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    /// Deletes a patch from the user root. Refuses to act on
    /// factory-only entries — the runtime never writes to factory
    /// roots. The UI should already have disabled the DELETE
    /// button via `patch_source`, so an error here means a stale
    /// click or a programmatic caller; either way we'd rather fail
    /// loud than silently no-op.
    pub fn delete(&self, mode: &str, name: &str) -> Result<(), PatchError> {
        match self.patch_source(mode, name) {
            Some(PatchSource::Factory) => Err(PatchError::ReadOnlyFactory(name.to_string())),
            None => Err(PatchError::Io(
                self.user_path(mode, name)?.display().to_string(),
                std::io::Error::new(std::io::ErrorKind::NotFound, "patch not found"),
            )),
            Some(PatchSource::User) => {
                let path = self.user_path(mode, name)?;
                std::fs::remove_file(&path)
                    .map_err(|e| PatchError::Io(path.display().to_string(), e))?;
                Ok(())
            }
        }
    }

    /// Path under the user root. Used for save (always) and load
    /// (preferred over factory). Validates `mode` / `name` against
    /// the safe-name rules.
    fn user_path(&self, mode: &str, name: &str) -> Result<PathBuf, PatchError> {
        if !is_safe_name(mode) {
            return Err(PatchError::InvalidName(mode.to_string()));
        }
        if !is_safe_name(name) {
            return Err(PatchError::InvalidName(name.to_string()));
        }
        Ok(self.root.join(mode).join(format!("{name}.json")))
    }

    /// Returns the first factory root that contains a file matching
    /// `mode/name.json`, or `None` if no factory layer carries it.
    fn factory_path(&self, mode: &str, name: &str) -> Option<PathBuf> {
        if !is_safe_name(mode) || !is_safe_name(name) {
            return None;
        }
        for fac_root in &self.factory_roots {
            let p = fac_root.join(mode).join(format!("{name}.json"));
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    // ── Perf methods ──────────────────────────────────────────────
    // Mirrors the Patch API shape so higher layers can expose both as
    // uniform "library entries" while the on-disk and in-memory types
    // stay distinct.

    /// Lists all perf names, sorted alphabetically.
    #[must_use]
    pub fn list_perfs(&self) -> Vec<String> {
        let dir = self.root.join(PERF_SUBDIR);
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    if let Some(stem) = path.file_stem() {
                        names.push(stem.to_string_lossy().into_owned());
                    }
                }
            }
        }
        names.sort();
        names
    }

    /// Loads a perf by name. Same forward-compat policy as `load`:
    /// reject any file whose declared `version` is newer than
    /// `SCHEMA_VERSION`.
    pub fn load_perf(&self, name: &str) -> Result<Perf, PatchError> {
        let path = self.perf_path(name)?;
        let json = std::fs::read_to_string(&path)
            .map_err(|e| PatchError::Io(path.display().to_string(), e))?;
        let perf: Perf = serde_json::from_str(&json).map_err(PatchError::Parse)?;
        if perf.version > SCHEMA_VERSION {
            return Err(PatchError::UnsupportedVersion {
                path: path.display().to_string(),
                found: perf.version,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(perf)
    }

    /// Saves a perf. Overwrites if it already exists.
    ///
    /// Refreshes `modified_at`, backfills `created_at` / `brume_version`
    /// if the incoming Perf was deserialized from an older file. Uses
    /// write-to-temp + rename for atomicity (survives power loss).
    pub fn save_perf(&self, perf: &Perf) -> Result<(), PatchError> {
        let path = self.perf_path(&perf.name)?;
        let mut to_write = perf.clone();
        let now = now_iso8601();
        if to_write.created_at.is_empty() {
            to_write.created_at = now.clone();
        }
        to_write.modified_at = now;
        if to_write.brume_version.is_empty() {
            to_write.brume_version = env!("CARGO_PKG_VERSION").to_string();
        }
        let json = serde_json::to_string_pretty(&to_write).map_err(PatchError::Parse)?;

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| PatchError::Io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, &path).map_err(|e| PatchError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    /// Deletes a perf by name.
    pub fn delete_perf(&self, name: &str) -> Result<(), PatchError> {
        let path = self.perf_path(name)?;
        std::fs::remove_file(&path).map_err(|e| PatchError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    fn perf_path(&self, name: &str) -> Result<PathBuf, PatchError> {
        if !is_safe_name(name) {
            return Err(PatchError::InvalidName(name.to_string()));
        }
        Ok(self.root.join(PERF_SUBDIR).join(format!("{name}.json")))
    }
}

/// Returns true if a name is safe for use as a filename component.
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ')
}

/// Parse a `BRUME_FACTORY_DIR`-style colon-separated path list into
/// a `Vec<PathBuf>`, dropping empty entries.
fn parse_factory_path(input: &str) -> Vec<PathBuf> {
    input
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Resolve the default factory root list. Honours `BRUME_FACTORY_DIR`
/// (colon-separated paths) for dev workflow on hosts that don't have
/// `/usr/share/brume/factory` provisioned. The release build relies on
/// `brumectl install` to put the factory tree under the system path.
fn default_factory_roots() -> Vec<PathBuf> {
    if let Ok(env_path) = std::env::var("BRUME_FACTORY_DIR") {
        let parts = parse_factory_path(&env_path);
        if !parts.is_empty() {
            return parts;
        }
    }
    vec![PathBuf::from(DEFAULT_FACTORY_ROOT)]
}

/// Read all `*.json` file stems out of `dir` into `out`. Silently
/// skips the directory if it doesn't exist (e.g. a factory root the
/// device hasn't been provisioned with) — no error, just nothing to
/// merge. Also skips any file whose name starts with `.`, which
/// covers macOS AppleDouble forks (`._foo.json`) that tar leaks when
/// archiving HFS+/APFS sources, plus general dotfile hygiene
/// (`.DS_Store`, etc.).
fn collect_json_stems(dir: &Path, out: &mut std::collections::BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if let Some(stem) = path.file_stem() {
            out.insert(stem.to_string_lossy().into_owned());
        }
    }
}

/// Cleanup migration for the previous "seed factory presets into the
/// user library on first run" model. For each user-dir patch whose
/// name matches a same-mode factory entry, we read both files: if the
/// user file is stamped `author = "Brume Factory"` *and* its `params`
/// equal the factory file's `params`, the user file is an untouched
/// seeded copy and we remove it (the factory layer covers it now). If
/// `params` differ, the user has edited the patch and we leave their
/// shadow copy in place. Idempotent — a second pass finds nothing to
/// do. Failures here never block library open; this is a best-effort
/// cleanup for an old code path.
fn cleanup_seeded_factory_files(lib: &PatchLibrary) {
    for &mode in LIBRARY_MODES {
        let user_dir = lib.root.join(mode);
        let Ok(entries) = std::fs::read_dir(&user_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let user_path = entry.path();
            if user_path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(stem) = user_path.file_stem() else {
                continue;
            };
            let name = stem.to_string_lossy();
            let Some(factory_path) = lib.factory_path(mode, &name) else {
                continue;
            };
            let (Ok(user_json), Ok(factory_json)) = (
                std::fs::read_to_string(&user_path),
                std::fs::read_to_string(&factory_path),
            ) else {
                continue;
            };
            let (Ok(user_patch), Ok(factory_patch)) = (
                serde_json::from_str::<Patch>(&user_json),
                serde_json::from_str::<Patch>(&factory_json),
            ) else {
                continue;
            };
            if user_patch.author.as_deref() == Some(FACTORY_AUTHOR)
                && user_patch.params == factory_patch.params
            {
                let _ = std::fs::remove_file(&user_path);
            }
        }
    }
    let _ = std::fs::remove_file(lib.root.join(LEGACY_FACTORY_MARKER));
}

/// One-shot migration: legacy `complex/` → `fm/`. Walks every `*.json`
/// under `root/complex/`, rewrites its `mode` field to `"fm"`, writes
/// to `root/fm/<same-name>.json`, removes the original. If the rewrite
/// fails for any file the migration leaves that one in place and
/// continues — we'd rather have the legacy file survive than get
/// silently destroyed. Idempotent: runs on every open, no-ops once the
/// directory is empty (and then removes it).
fn migrate_complex_to_fm(root: &Path) -> std::io::Result<()> {
    let legacy = root.join("complex");
    if !legacy.is_dir() {
        return Ok(());
    }
    let fm = root.join("fm");
    std::fs::create_dir_all(&fm)?;
    for entry in std::fs::read_dir(&legacy)?.flatten() {
        let src = entry.path();
        if src.extension().is_some_and(|e| e == "json") {
            let Some(file_name) = src.file_name() else {
                continue;
            };
            let dst = fm.join(file_name);
            // Don't clobber: if a same-named patch already exists in
            // fm/ the user has already saved something post-migration;
            // leave the legacy file alone as evidence.
            if dst.exists() {
                continue;
            }
            let Ok(json) = std::fs::read_to_string(&src) else {
                continue;
            };
            // Rewrite mode field via a Value round-trip so we don't lose
            // unknown fields if a newer schema ever wrote into complex/.
            let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&json) else {
                continue;
            };
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "mode".to_string(),
                    serde_json::Value::String("fm".to_string()),
                );
                // Auto-tag rewrite: if there's a tags array containing
                // "complex", replace that entry with "fm" in place; add
                // "fm" if no mode tag was present.
                if let Some(tags) = obj.get_mut("tags").and_then(|t| t.as_array_mut()) {
                    let mut has_fm = false;
                    for t in tags.iter_mut() {
                        if t.as_str() == Some("complex") {
                            *t = serde_json::Value::String("fm".to_string());
                            has_fm = true;
                        } else if t.as_str() == Some("fm") {
                            has_fm = true;
                        }
                    }
                    if !has_fm {
                        tags.push(serde_json::Value::String("fm".to_string()));
                    }
                }
            }
            let Ok(rewritten) = serde_json::to_string_pretty(&v) else {
                continue;
            };
            // Write to fm/ then remove legacy only if write succeeded.
            let tmp = dst.with_extension("json.tmp");
            if std::fs::write(&tmp, &rewritten).is_err() {
                continue;
            }
            if std::fs::rename(&tmp, &dst).is_err() {
                let _ = std::fs::remove_file(&tmp);
                continue;
            }
            let _ = std::fs::remove_file(&src);
        }
    }
    // Remove the legacy directory if now empty.
    if let Ok(mut entries) = std::fs::read_dir(&legacy) {
        if entries.next().is_none() {
            let _ = std::fs::remove_dir(&legacy);
        }
    }
    Ok(())
}

/// Errors from patch operations.
#[derive(Debug)]
pub enum PatchError {
    Io(String, std::io::Error),
    Parse(serde_json::Error),
    InvalidName(String),
    /// On-disk file declares a schema version this build doesn't know
    /// how to read. Carries `(found, supported)`. Refuses to load
    /// silently — re-saving a partially-parsed v2-as-v1 would lose
    /// fields the v1 deserializer skipped.
    UnsupportedVersion {
        path: String,
        found: u32,
        supported: u32,
    },
    /// Caller asked to delete a patch that lives only under a factory
    /// root. The runtime never writes to factory roots, so refusing
    /// here is the right behavior; the LIBRARY UI should disable the
    /// DELETE button for factory entries via `patch_source` so this
    /// error is rare in practice.
    ReadOnlyFactory(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(path, e) => write!(f, "{path}: {e}"),
            Self::Parse(e) => write!(f, "patch parse error: {e}"),
            Self::InvalidName(name) => write!(f, "invalid patch name: {name}"),
            Self::UnsupportedVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "{path}: schema version {found} is newer than this build supports ({supported}); \
                 update brume to read this file"
            ),
            Self::ReadOnlyFactory(name) => {
                write!(f, "factory preset '{name}' is read-only")
            }
        }
    }
}

impl std::error::Error for PatchError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open a library at `root` with no factory layer attached.
    /// Tests that exercise user-side `list` / `save` / `delete`
    /// semantics use this so factory entries don't appear in their
    /// assertions; the dedicated factory tests build their own
    /// factory root explicitly.
    fn open_user_only(root: &Path) -> PatchLibrary {
        PatchLibrary::open_with_factory_roots(root, vec![]).unwrap()
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join("brume-test-patches");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let mut params = BTreeMap::new();
        params.insert("FmIndex".to_string(), 3.5);
        params.insert("FilterCutoff".to_string(), 8000.0);

        let patch = Patch::new("warm-pad".to_string(), "fm".to_string(), params);
        lib.save(&patch).unwrap();

        let loaded = lib.load("fm", "warm-pad").unwrap();
        assert_eq!(loaded.name, "warm-pad");
        assert_eq!(loaded.mode, "fm");
        assert_eq!(loaded.params.get("FmIndex"), Some(&3.5));
        assert_eq!(loaded.version, SCHEMA_VERSION);
        // New metadata fields are populated.
        assert!(
            !loaded.created_at.is_empty(),
            "created_at should be populated"
        );
        assert!(
            !loaded.modified_at.is_empty(),
            "modified_at should be populated"
        );
        assert!(
            !loaded.brume_version.is_empty(),
            "brume_version should be populated"
        );
        assert!(
            loaded.tags.contains(&"fm".to_string()),
            "mode auto-tag present"
        );

        let names = lib.list("fm");
        assert_eq!(names, vec!["warm-pad"]);

        lib.delete("fm", "warm-pad").unwrap();
        assert!(lib.list("fm").is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_sorted() {
        let dir = std::env::temp_dir().join("brume-test-sorted");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        for name in &["zebra", "alpha", "middle"] {
            let p = Patch::new(name.to_string(), "timbral".to_string(), BTreeMap::new());
            lib.save(&p).unwrap();
        }

        let names = lib.list("timbral");
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_fields_default() {
        // Minimal pre-metadata file (what a v1 save wrote before the
        // created_at/modified_at/brume_version/author fields existed).
        let json = r#"{"version":1,"name":"test","mode":"fm","params":{}}"#;
        let patch: Patch = serde_json::from_str(json).unwrap();
        assert!(patch.tags.is_empty());
        assert!(patch.created_at.is_empty());
        assert!(patch.modified_at.is_empty());
        assert!(patch.brume_version.is_empty());
        assert!(patch.author.is_none());
        assert!(patch.modulation.is_none());
        assert!(patch.effects.is_none());
        assert!(patch.notes.is_none());
    }

    #[test]
    fn granular_directory_provisioned() {
        // Pre-rename libraries didn't create ~/.brume/library/granular/,
        // so list() returned empty and save() failed silently.
        let dir = std::env::temp_dir().join("brume-test-granular");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let p = Patch::new("mist".to_string(), "granular".to_string(), BTreeMap::new());
        lib.save(&p).unwrap();
        assert_eq!(lib.list("granular"), vec!["mist"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn complex_to_fm_migration() {
        let dir = std::env::temp_dir().join("brume-test-complex-migration");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("complex")).unwrap();

        // Drop a legacy `complex/` patch with the old mode string,
        // plus an extra top-level field to verify unknown-field survival.
        std::fs::write(
            dir.join("complex/warm-pad.json"),
            r#"{
                "version": 1,
                "name": "warm-pad",
                "mode": "complex",
                "params": { "FilterCutoff": 8000.0 },
                "tags": ["complex", "user"],
                "someFutureField": { "nested": true }
            }"#,
        )
        .unwrap();

        let lib = open_user_only(&dir);

        // Legacy file moved out of complex/ and into fm/ with rewritten
        // mode; the complex/ directory is removed once empty.
        assert!(
            !dir.join("complex").exists(),
            "legacy complex/ dir should be removed"
        );
        let loaded = lib.load("fm", "warm-pad").unwrap();
        assert_eq!(loaded.mode, "fm");
        assert_eq!(loaded.params.get("FilterCutoff"), Some(&8000.0));
        // "complex" tag rewritten to "fm", other tags preserved.
        assert!(loaded.tags.contains(&"fm".to_string()));
        assert!(loaded.tags.contains(&"user".to_string()));
        assert!(!loaded.tags.contains(&"complex".to_string()));

        // Unknown field survived via the Value round-trip — read raw JSON
        // to verify (Patch struct naturally ignores it).
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("fm/warm-pad.json")).unwrap())
                .unwrap();
        assert_eq!(
            raw.get("someFutureField").and_then(|v| v.get("nested")),
            Some(&serde_json::Value::Bool(true)),
        );

        // Re-opening the library is a no-op migration (idempotent).
        let _lib2 = open_user_only(&dir);
        assert!(!dir.join("complex").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_preserves_existing_fm_on_name_collision() {
        // If both complex/foo.json and fm/foo.json exist, the legacy file
        // should be left alone (don't clobber newer data).
        let dir = std::env::temp_dir().join("brume-test-migration-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("complex")).unwrap();
        std::fs::create_dir_all(dir.join("fm")).unwrap();
        std::fs::write(
            dir.join("complex/foo.json"),
            r#"{"version":1,"name":"foo","mode":"complex","params":{"X":1.0}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("fm/foo.json"),
            r#"{"version":1,"name":"foo","mode":"fm","params":{"X":2.0}}"#,
        )
        .unwrap();

        let lib = open_user_only(&dir);
        // Newer fm/foo.json wins, legacy complex/foo.json stays in place.
        let loaded = lib.load("fm", "foo").unwrap();
        assert_eq!(loaded.params.get("X"), Some(&2.0));
        assert!(
            dir.join("complex/foo.json").exists(),
            "collision should preserve legacy"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refreshes_modified_at_and_preserves_created_at() {
        let dir = std::env::temp_dir().join("brume-test-timestamps");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let patch = Patch::new("t".to_string(), "harmonic".to_string(), BTreeMap::new());
        lib.save(&patch).unwrap();
        let first = lib.load("harmonic", "t").unwrap();

        // Save again, round-trip — modified_at refreshed, created_at stable.
        // (Sub-second resolution means the timestamps may equal the first
        // save's, which is fine — we're just asserting both exist and
        // created_at survived.)
        let mut again = first.clone();
        again.params.insert("HarmonicLevel1".to_string(), 0.8);
        lib.save(&again).unwrap();
        let reloaded = lib.load("harmonic", "t").unwrap();
        assert_eq!(
            reloaded.created_at, first.created_at,
            "created_at must survive re-save"
        );
        assert!(!reloaded.modified_at.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn iso8601_format_is_well_formed() {
        let s = now_iso8601();
        // "YYYY-MM-DDTHH:MM:SSZ" = 20 chars.
        assert_eq!(s.len(), 20, "iso8601 string should be 20 chars: got {s:?}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }

    #[test]
    fn load_rejects_future_schema_version() {
        // A v2-from-the-future patch landing in a v1 build should fail
        // loud, not silently re-save as v1 with v2-specific fields
        // dropped. The check is part of the load contract; if a future
        // build introduces v2 it adds a migration arm before bumping
        // SCHEMA_VERSION here.
        let dir = std::env::temp_dir().join("brume-test-future-version");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fm")).unwrap();
        // Hand-write a patch that claims version = 99 but otherwise
        // looks like a valid v1 file.
        let path = dir.join("fm").join("future.json");
        std::fs::write(
            &path,
            r#"{
              "version": 99,
              "name": "future",
              "mode": "fm",
              "params": {}
            }"#,
        )
        .unwrap();

        let lib = open_user_only(&dir);
        let err = lib.load("fm", "future").unwrap_err();
        match err {
            PatchError::UnsupportedVersion {
                found, supported, ..
            } => {
                assert_eq!(found, 99);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_accepts_current_schema_version() {
        // Sanity: a normal save+load through this build still works
        // after the version gate. Catches a regression where the
        // gate is too strict (e.g. accidentally requires version ==
        // SCHEMA_VERSION exactly, blocking older patches that
        // legitimately predate a future bump).
        let dir = std::env::temp_dir().join("brume-test-current-version");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);
        let mut params = BTreeMap::new();
        params.insert("FmIndex".to_string(), 1.0);
        let p = Patch::new("ok".into(), "fm".into(), params);
        lib.save(&p).unwrap();
        let loaded = lib.load("fm", "ok").unwrap();
        assert_eq!(loaded.version, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Perf tests ────────────────────────────────────────────────

    fn sample_part(mode: &str, fm_idx: f64) -> PartSnapshot {
        let mut p = BTreeMap::new();
        p.insert("FmIndex".to_string(), fm_idx);
        p.insert("FilterCutoff".to_string(), 8000.0);
        PartSnapshot {
            mode: mode.to_string(),
            params: p,
            source_patch: Some(format!("{mode}-source")),
        }
    }

    fn sample_fx() -> FxSnapshot {
        let mut p = BTreeMap::new();
        p.insert("Reverb.decay".to_string(), 4.5);
        p.insert("Chorus.rate".to_string(), 0.35);
        FxSnapshot {
            params: p,
            source_patch: None,
        }
    }

    fn sample_mixer() -> MixerSnapshot {
        MixerSnapshot {
            levels: [0.6, 0.5, 0.4, 0.3],
            mutes: [false, false, false, true],
            delay_sends: [0.2, 0.0, 0.1, 0.0],
            reverb_sends: [0.4, 0.3, 0.2, 0.1],
            master_volume: 0.7,
        }
    }

    #[test]
    fn perf_directory_provisioned() {
        let dir = std::env::temp_dir().join("brume-test-perf-dir");
        let _ = std::fs::remove_dir_all(&dir);
        let _lib = open_user_only(&dir);
        assert!(dir.join("perf").is_dir(), "perf/ should be created on open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn perf_roundtrip() {
        let dir = std::env::temp_dir().join("brume-test-perf-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let parts = [
            Some(sample_part("fm", 3.5)),
            Some(sample_part("harmonic", 0.0)),
            None,
            Some(sample_part("granular", 0.0)),
        ];
        let perf = Perf::new(
            "obsidian-evening".to_string(),
            parts,
            Some(sample_fx()),
            sample_mixer(),
        );
        lib.save_perf(&perf).unwrap();

        let loaded = lib.load_perf("obsidian-evening").unwrap();
        assert_eq!(loaded.name, "obsidian-evening");
        assert_eq!(loaded.version, SCHEMA_VERSION);
        assert!(
            loaded.tags.contains(&"perf".to_string()),
            "perf auto-tag present"
        );
        assert!(!loaded.created_at.is_empty());
        assert!(!loaded.modified_at.is_empty());
        assert!(!loaded.brume_version.is_empty());

        // Parts: slot 0 populated, slot 2 None.
        assert!(loaded.parts[0].is_some());
        assert!(loaded.parts[2].is_none());
        let fm = loaded.parts[0].as_ref().unwrap();
        assert_eq!(fm.mode, "fm");
        assert_eq!(fm.params.get("FmIndex"), Some(&3.5));
        assert_eq!(fm.source_patch.as_deref(), Some("fm-source"));

        // FX + mixer survive.
        assert_eq!(
            loaded.fx.as_ref().unwrap().params.get("Reverb.decay"),
            Some(&4.5),
        );
        assert_eq!(loaded.mixer.master_volume, 0.7);
        assert_eq!(loaded.mixer.mutes[3], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_perfs_sorted() {
        let dir = std::env::temp_dir().join("brume-test-perf-sorted");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        for name in &["zebra", "alpha", "middle"] {
            let p = Perf::new(
                name.to_string(),
                [None, None, None, None],
                None,
                MixerSnapshot::default(),
            );
            lib.save_perf(&p).unwrap();
        }
        assert_eq!(lib.list_perfs(), vec!["alpha", "middle", "zebra"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn perf_delete_removes_file() {
        let dir = std::env::temp_dir().join("brume-test-perf-delete");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let p = Perf::new(
            "tmp".to_string(),
            [None, None, None, None],
            None,
            MixerSnapshot::default(),
        );
        lib.save_perf(&p).unwrap();
        assert_eq!(lib.list_perfs(), vec!["tmp"]);

        lib.delete_perf("tmp").unwrap();
        assert!(lib.list_perfs().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn perf_save_refreshes_modified_at_and_preserves_created_at() {
        let dir = std::env::temp_dir().join("brume-test-perf-timestamps");
        let _ = std::fs::remove_dir_all(&dir);
        let lib = open_user_only(&dir);

        let p = Perf::new(
            "t".to_string(),
            [None, None, None, None],
            None,
            MixerSnapshot::default(),
        );
        lib.save_perf(&p).unwrap();
        let first = lib.load_perf("t").unwrap();

        // Mutate and re-save; created_at must survive.
        let mut again = first.clone();
        again.mixer.master_volume = 0.5;
        lib.save_perf(&again).unwrap();
        let reloaded = lib.load_perf("t").unwrap();
        assert_eq!(reloaded.created_at, first.created_at);
        assert!(!reloaded.modified_at.is_empty());
        assert_eq!(reloaded.mixer.master_volume, 0.5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn perf_minimal_json_deserializes() {
        // Forward-compat smoke test: a Perf file that only carries the
        // required fields (no metadata, no fx, no modulation) should
        // still deserialize cleanly via #[serde(default)] fallbacks.
        let json = r#"{
            "version": 1,
            "name": "bare",
            "parts": [null, null, null, null],
            "mixer": {
                "levels": [0.8, 0.8, 0.8, 0.8],
                "mutes": [false, false, false, false],
                "delay_sends": [0.0, 0.0, 0.0, 0.0],
                "reverb_sends": [0.0, 0.0, 0.0, 0.0],
                "master_volume": 0.8
            }
        }"#;
        let perf: Perf = serde_json::from_str(json).unwrap();
        assert_eq!(perf.name, "bare");
        assert!(perf.tags.is_empty());
        assert!(perf.created_at.is_empty());
        assert!(perf.fx.is_none());
        assert!(perf.parts.iter().all(Option::is_none));
    }

    // ── Factory preset tests ──────────────────────────────────────
    // Factory presets ship as JSON files under
    // `crates/patch-store/factory/`, get installed to
    // `/usr/share/brume/factory/` on a real device, and are merged
    // into `list` / `load` results from a separate read-only library
    // layer. Tests below build a synthetic factory root in tempdir
    // and pass it via `open_with_factory_roots`.

    /// Workspace path to the in-tree factory tree. Tests use this as
    /// the source for synthesizing a fake `/usr/share/brume/factory`
    /// in their tempdir, and `factory_fm_presets_parse_and_validate`
    /// reads it directly.
    fn workspace_factory_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("factory")
    }

    /// Copy the in-tree `factory/` subtree into `dest` so the test
    /// can hand `dest` to `open_with_factory_roots` as a factory
    /// root. Mirrors what `brumectl install` will do at runtime.
    fn install_factory_root(dest: &Path) {
        std::fs::create_dir_all(dest).unwrap();
        let src = workspace_factory_root();
        copy_dir_recursive(&src, dest);
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let path = entry.path();
            let target = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &target);
            } else {
                std::fs::copy(&path, &target).unwrap();
            }
        }
    }

    /// Per-engine sanity params — any patch missing one of these would
    /// load with surprising defaults baked in by whichever code path
    /// constructs the live voice. Keeps factory authors honest about
    /// shipping a *complete* configuration.
    fn required_params_for(mode: &str) -> &'static [&'static str] {
        match mode {
            "fm" => &["Algorithm", "FilterCutoff", "AmpAttack"],
            "harmonic" => &["HarmonicLevel1", "FilterCutoff", "AmpAttack"],
            "timbral" => &["Timbre", "FilterCutoff", "AmpAttack"],
            "granular" => &["GranularDensity", "FilterCutoff", "AmpAttack"],
            _ => &[],
        }
    }

    #[test]
    fn factory_presets_parse_and_validate() {
        let root = workspace_factory_root();
        let mut total = 0usize;
        for mode in ["fm", "harmonic", "timbral", "granular"] {
            let mode_dir = root.join(mode);
            let entries: Vec<_> = std::fs::read_dir(&mode_dir)
                .unwrap_or_else(|e| panic!("factory/{mode} unreadable: {e}"))
                .flatten()
                .collect();
            assert!(!entries.is_empty(), "factory/{mode} has no presets");
            for entry in entries {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                let json = std::fs::read_to_string(&path).unwrap();
                let patch: Patch = serde_json::from_str(&json)
                    .unwrap_or_else(|e| panic!("{mode}/{stem} won't parse: {e}"));
                assert_eq!(patch.mode, mode, "{mode}/{stem}: mode field mismatch");
                assert!(!patch.name.is_empty(), "{mode}/{stem}: name required");
                assert_eq!(
                    patch.name, stem,
                    "{mode}/{stem}: filename stem must match patch.name (the runtime looks up by patch.name → {{name}}.json)",
                );
                assert!(
                    is_safe_name(&patch.name),
                    "{mode}/{stem}: name '{}' has unsafe characters",
                    patch.name
                );
                assert_eq!(
                    patch.version, SCHEMA_VERSION,
                    "{mode}/{stem}: schema version"
                );
                assert_eq!(
                    patch.author.as_deref(),
                    Some(FACTORY_AUTHOR),
                    "{mode}/{stem}: must be stamped author = \"{FACTORY_AUTHOR}\"",
                );
                for required in required_params_for(mode) {
                    assert!(
                        patch.params.contains_key(*required),
                        "{mode}/{stem}: missing required param {required}",
                    );
                }
                total += 1;
            }
        }
        assert!(total > 0, "expected factory presets across all engines");
    }

    #[test]
    fn factory_list_merges_user_and_factory_roots() {
        let dir = std::env::temp_dir().join("brume-test-factory-list");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();

        // Factory entries appear in `list` even though no user file
        // has been saved.
        let names = lib.list("fm");
        assert!(
            names.contains(&"Carillon Bell".to_string()),
            "factory entry should appear in list"
        );

        // Save a user patch and confirm it merges with factory entries.
        let mut params = BTreeMap::new();
        params.insert("FmIndex".to_string(), 1.0);
        let patch = Patch::new("My Patch".into(), "fm".into(), params);
        lib.save(&patch).unwrap();
        let names = lib.list("fm");
        assert!(names.contains(&"My Patch".to_string()));
        assert!(names.contains(&"Carillon Bell".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn factory_load_falls_back_to_factory_root() {
        let dir = std::env::temp_dir().join("brume-test-factory-load");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();

        // User dir empty for "Carillon Bell" but factory carries it.
        let loaded = lib.load("fm", "Carillon Bell").unwrap();
        assert_eq!(loaded.author.as_deref(), Some(FACTORY_AUTHOR));
        assert!(loaded.params.contains_key("Algorithm"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn factory_load_user_shadow_wins_over_factory() {
        let dir = std::env::temp_dir().join("brume-test-factory-shadow");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();

        // Load factory's "Carillon Bell", tweak a param, save back.
        let mut shadow = lib.load("fm", "Carillon Bell").unwrap();
        shadow.params.insert("FmIndex".to_string(), 42.0);
        lib.save(&shadow).unwrap();

        // Load now picks up the user shadow.
        let loaded = lib.load("fm", "Carillon Bell").unwrap();
        assert_eq!(loaded.params.get("FmIndex"), Some(&42.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn factory_patch_source_distinguishes_layers() {
        let dir = std::env::temp_dir().join("brume-test-patch-source");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();

        assert_eq!(
            lib.patch_source("fm", "Carillon Bell"),
            Some(PatchSource::Factory)
        );
        assert_eq!(lib.patch_source("fm", "Nonexistent"), None);

        let mut params = BTreeMap::new();
        params.insert("FmIndex".to_string(), 0.5);
        lib.save(&Patch::new("Mine".into(), "fm".into(), params))
            .unwrap();
        assert_eq!(lib.patch_source("fm", "Mine"), Some(PatchSource::User));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn factory_delete_user_succeeds_factory_refused() {
        let dir = std::env::temp_dir().join("brume-test-factory-delete");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();

        // Factory-only entry: delete refuses.
        let err = lib.delete("fm", "Carillon Bell").unwrap_err();
        assert!(matches!(err, PatchError::ReadOnlyFactory(_)));
        // Still listed, still loadable.
        assert!(lib.list("fm").contains(&"Carillon Bell".to_string()));

        // User entry: delete works.
        let mut params = BTreeMap::new();
        params.insert("FmIndex".to_string(), 0.1);
        lib.save(&Patch::new("Disposable".into(), "fm".into(), params))
            .unwrap();
        assert!(lib.delete("fm", "Disposable").is_ok());
        assert!(!lib.list("fm").contains(&"Disposable".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_migration_removes_unmodified_seeded_copies() {
        // Simulate the state left behind by the old seed-on-first-run
        // model: the user library has copies of factory presets stamped
        // `author = "Brume Factory"`, plus a legacy marker file.
        let dir = std::env::temp_dir().join("brume-test-cleanup-migration");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        std::fs::create_dir_all(user.join("fm")).unwrap();

        // Pre-populate the user library with a "seeded" copy that
        // matches the factory file's params verbatim.
        let factory_json = std::fs::read_to_string(factory.join("fm/Carillon Bell.json")).unwrap();
        let mut seeded: Patch = serde_json::from_str(&factory_json).unwrap();
        // The save() path stamps timestamps but preserves params + author.
        seeded.created_at = "2026-04-30T12:00:00Z".into();
        seeded.modified_at = "2026-04-30T12:00:00Z".into();
        let user_path = user.join("fm").join(format!("{}.json", seeded.name));
        std::fs::write(&user_path, serde_json::to_string_pretty(&seeded).unwrap()).unwrap();
        std::fs::write(user.join(".factory_seeded_1"), b"1\n").unwrap();
        assert!(user_path.exists());

        // Open the library — cleanup should remove the seeded copy
        // and the marker.
        let _lib = PatchLibrary::open_with_factory_roots(&user, vec![factory.clone()]).unwrap();
        assert!(
            !user_path.exists(),
            "untouched factory copy should be removed"
        );
        assert!(
            !user.join(".factory_seeded_1").exists(),
            "legacy marker should be removed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_migration_preserves_user_edited_factory_copies() {
        let dir = std::env::temp_dir().join("brume-test-cleanup-keep");
        let _ = std::fs::remove_dir_all(&dir);
        let factory = dir.join("factory");
        let user = dir.join("user");
        install_factory_root(&factory);
        std::fs::create_dir_all(user.join("fm")).unwrap();

        // User seeded copy that has been EDITED — params differ from
        // factory. Cleanup must not touch it.
        let factory_json = std::fs::read_to_string(factory.join("fm/Carillon Bell.json")).unwrap();
        let mut edited: Patch = serde_json::from_str(&factory_json).unwrap();
        edited.params.insert("FmIndex".to_string(), 99.0);
        let user_path = user.join("fm").join(format!("{}.json", edited.name));
        std::fs::write(&user_path, serde_json::to_string_pretty(&edited).unwrap()).unwrap();

        let lib = PatchLibrary::open_with_factory_roots(&user, vec![factory]).unwrap();
        assert!(user_path.exists(), "user-edited copy must be preserved");
        let loaded = lib.load("fm", &edited.name).unwrap();
        assert_eq!(loaded.params.get("FmIndex"), Some(&99.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_factory_path_splits_colon_separated() {
        // The pure parsing path is testable without touching env;
        // verify it independently of `default_factory_roots`'s env
        // read so we don't need `unsafe` env mutation in tests.
        assert_eq!(
            parse_factory_path("/tmp/a:/tmp/b"),
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]
        );
        assert_eq!(parse_factory_path(""), Vec::<PathBuf>::new());
        assert_eq!(
            parse_factory_path("::/tmp/a::"),
            vec![PathBuf::from("/tmp/a")]
        );
    }
}
