// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! User-persisted application settings for Brume.
//!
//! Lives at `~/.brume/settings.json` by default. Small, hand-editable, tolerant
//! of missing fields — every addition uses `#[serde(default)]` so old files
//! keep loading after the schema grows.
//!
//! Atomic writes via temp-file + rename (same pattern as `brume-patch-store`)
//! so a power loss mid-save can never leave a half-written file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Errors that can occur loading or saving settings.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("I/O error on {0}: {1}")]
    Io(String, #[source] std::io::Error),

    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// User-persisted settings for the Brume app.
///
/// Every field defaults so partial or outdated files load cleanly. Unknown
/// keys are accepted silently (serde default) rather than failing — a file
/// written by a newer Brume shouldn't break an older one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Selected output device. `None` = host default. The id is cpal's
    /// `Device::name()` verbatim; see `brume_audio_io::OutputDeviceInfo`.
    #[serde(default)]
    pub output_device_id: Option<String>,

    /// Audio output latency preset (SYS → AUDIO OUTPUT). Maps to the cpal
    /// buffer size on the UAC2 gadget; see [`brume_common::LatencyPreset`].
    /// Defaults to `Balanced`. `BRUME_AUDIO_PERIOD` overrides at runtime.
    #[serde(default)]
    pub audio_latency: brume_common::LatencyPreset,
}

impl Settings {
    /// Returns the default settings file path: `$HOME/.brume/settings.json`.
    ///
    /// Falls back to `./settings.json` if `$HOME` is unset.
    #[must_use]
    pub fn default_path() -> PathBuf {
        let base = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.join(".brume").join("settings.json")
    }

    /// Loads settings from `path`. Missing file returns default settings
    /// (not an error — a fresh install hasn't written anything yet).
    /// Parse errors on an existing file are surfaced so the caller can
    /// decide whether to warn and fall back, or abort startup.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::Io`] for read failures other than
    /// `NotFound`, or [`SettingsError::Parse`] if the file exists but
    /// isn't valid JSON.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(json) => serde_json::from_str(&json).map_err(SettingsError::Parse),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(SettingsError::Io(path.display().to_string(), e)),
        }
    }

    /// Loads settings from `path`, falling back to default with a warning
    /// logged to stderr on any failure. Callers that want strict error
    /// handling use `load` directly.
    #[must_use]
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "brume settings: failed to load {}: {e}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Atomically writes settings to `path`, creating parent directories
    /// if needed. Uses a temp-file + rename so a crash mid-write leaves
    /// either the old file intact or the new file complete — never
    /// a partial mix.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::Io`] for directory/file I/O failures,
    /// or [`SettingsError::Parse`] if serialization fails (shouldn't
    /// happen for well-formed `Settings`, but possible for a future
    /// field with a custom serializer).
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| SettingsError::Io(parent.display().to_string(), e))?;
            }
        }

        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| SettingsError::Io(tmp.display().to_string(), e))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| SettingsError::Io(path.display().to_string(), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "brume-settings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = tmpdir();
        let path = dir.join("settings.json");
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.output_device_id, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let dir = tmpdir();
        let path = dir.join("settings.json");
        let original = Settings {
            output_device_id: Some("plughw:CARD=UAC2Gadget,DEV=0".into()),
            ..Default::default()
        };
        original.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.output_device_id, original.output_device_id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let dir = tmpdir();
        let path = dir.join("settings.json");
        // Simulate a file written by a future Brume with extra fields.
        std::fs::write(
            &path,
            r#"{"output_device_id":"hw:X","future_field":123,"nested":{"a":1}}"#,
        )
        .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.output_device_id.as_deref(), Some("hw:X"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_fields_default() {
        let dir = tmpdir();
        let path = dir.join("settings.json");
        // Empty object — every field should default.
        std::fs::write(&path, "{}").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.output_device_id, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_or_default_returns_default_on_parse_error() {
        let dir = tmpdir();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{this is not json}").unwrap();
        let s = Settings::load_or_default(&path);
        assert_eq!(s.output_device_id, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tmpdir();
        let path = dir.join("nested").join("deep").join("settings.json");
        let s = Settings {
            output_device_id: Some("hw:Y".into()),
            ..Default::default()
        };
        s.save(&path).unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
