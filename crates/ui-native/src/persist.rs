// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Debounced persistence for two pieces of UI state:
//!
//! * `ControlMatrix` (CC / MIDI Learn bindings) — `~/.brume/cc-bindings.json`
//! * `TransportSnapshot` (BPM + clock source) — `~/.brume/transport.json`
//!
//! Both use the same trailing-edge debounce pattern: edits arrive at the
//! worker, the worker waits for quiescence, then writes once. A 1.5 s
//! window catches MIDI Learn flurries and BPM-slider drags without
//! hammering eMMC, while still flushing within human reaction time so
//! the most recent state survives an abrupt power-off.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use brume_control_model::ControlMatrix;
use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use serde::{Deserialize, Serialize};

/// Snapshot of the user-authoritative transport settings persisted to
/// `~/.brume/transport.json`. Mirror of `NativeUi::TransportUi` minus
/// the Rust-only details — keeps the JSON schema small and forward-
/// compatible.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TransportSnapshot {
    pub bpm: f32,
    pub clock_mode_idx: usize,
}

pub struct PersistHandle {
    // No call site yet on the iced UI — the wave 7b plumbing wires
    // the worker through; later waves (binding add/remove/reset)
    // call mark_dirty.
    #[allow(dead_code)]
    trigger: Sender<PersistCmd>,
}

#[allow(dead_code)]
enum PersistCmd {
    MarkDirty,
}

const DEBOUNCE: Duration = Duration::from_millis(1500);

impl PersistHandle {
    #[must_use]
    pub fn spawn(matrix: Arc<RwLock<ControlMatrix>>, path: PathBuf) -> Self {
        let (trigger, rx) = bounded::<PersistCmd>(8);
        thread::Builder::new()
            .name("brume-persist".into())
            .spawn(move || worker(rx, matrix, path))
            .ok();
        Self { trigger }
    }

    #[allow(dead_code)]
    pub fn mark_dirty(&self) {
        let _ = self.trigger.try_send(PersistCmd::MarkDirty);
    }
}

impl Drop for PersistHandle {
    fn drop(&mut self) {
        // Sender drop closes the channel; the worker observes Disconnected,
        // flushes any pending dirty, and exits. No explicit cleanup needed.
    }
}

fn worker(
    rx: crossbeam_channel::Receiver<PersistCmd>,
    matrix: Arc<RwLock<ControlMatrix>>,
    path: PathBuf,
) {
    let mut dirty = false;
    loop {
        let evt = if dirty {
            rx.recv_timeout(DEBOUNCE)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };

        match evt {
            Ok(PersistCmd::MarkDirty) => dirty = true,
            Err(RecvTimeoutError::Timeout) => {
                write_matrix(&matrix, &path);
                dirty = false;
            }
            Err(RecvTimeoutError::Disconnected) => {
                if dirty {
                    write_matrix(&matrix, &path);
                }
                return;
            }
        }
    }
}

/// Atomic write: temp file + rename. A crash mid-write leaves the prior
/// file intact. Persistence is best-effort — we log to stderr (captured
/// by journalctl on the CM5) on every failure mode so silent disk
/// errors (ENOSPC, EROFS, EACCES) surface during diagnosis instead of
/// disappearing into a "saved" UX with nothing on disk.
fn write_matrix(matrix: &Arc<RwLock<ControlMatrix>>, path: &Path) {
    let Ok(guard) = matrix.read() else {
        eprintln!("brume persist: control-matrix lock poisoned, skip write");
        return;
    };
    let json = match serde_json::to_string_pretty(&*guard) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("brume persist: control-matrix serialize failed: {e}");
            return;
        }
    };
    drop(guard);

    write_atomic(path, &json, "control-matrix");
}

// ── Transport persistence ─────────────────────────────────────────
//
// Same pattern as the ControlMatrix worker but operates on a
// snapshot value rather than a shared lock — TransportUi is small
// enough (two scalars) that the UI sends a fresh copy on each
// mark_dirty, and the worker holds only the latest pending one.

/// Background-persisted handle for `TransportSnapshot`. UI calls
/// `save` on each transport edit; the worker debounces and writes
/// JSON to disk.
pub struct TransportPersistHandle {
    trigger: Sender<TransportSnapshot>,
}

impl TransportPersistHandle {
    #[must_use]
    pub fn spawn(path: PathBuf) -> Self {
        let (trigger, rx) = bounded::<TransportSnapshot>(8);
        thread::Builder::new()
            .name("brume-persist-transport".into())
            .spawn(move || transport_worker(rx, path))
            .ok();
        Self { trigger }
    }

    /// Send a snapshot to the worker. The worker holds only the most
    /// recent value, so rapid edits (slider drag, segment taps in
    /// quick succession) coalesce into one disk write.
    pub fn save(&self, snapshot: TransportSnapshot) {
        let _ = self.trigger.try_send(snapshot);
    }
}

impl Drop for TransportPersistHandle {
    fn drop(&mut self) {
        // Sender drop closes the channel; the worker observes
        // Disconnected, flushes any pending snapshot, and exits.
    }
}

fn transport_worker(rx: crossbeam_channel::Receiver<TransportSnapshot>, path: PathBuf) {
    let mut pending: Option<TransportSnapshot> = None;
    loop {
        let evt = if pending.is_some() {
            rx.recv_timeout(DEBOUNCE)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match evt {
            Ok(snapshot) => {
                pending = Some(snapshot);
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(s) = pending.take() {
                    write_transport(&s, &path);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(s) = pending.take() {
                    write_transport(&s, &path);
                }
                return;
            }
        }
    }
}

fn write_transport(snapshot: &TransportSnapshot, path: &Path) {
    let json = match serde_json::to_string_pretty(snapshot) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("brume persist: transport serialize failed: {e}");
            return;
        }
    };
    write_atomic(path, &json, "transport");
}

/// Shared atomic-write helper for the persist workers. Logs every
/// failure mode (mkdir, write, rename) to stderr with the destination
/// path so journalctl makes the on-disk state debuggable. Silent
/// success on the happy path — the workers run on every dirty edit
/// and we don't want one log line per BPM-slider tick.
fn write_atomic(path: &Path, contents: &str, kind: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "brume persist: {kind} mkdir {} failed: {e}",
                parent.display()
            );
            return;
        }
    }
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, contents) {
        eprintln!("brume persist: {kind} write {} failed: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!(
            "brume persist: {kind} rename {} → {} failed: {e}",
            tmp.display(),
            path.display()
        );
    }
}

/// Synchronous read of a previously-persisted snapshot. Returns
/// `None` on missing file, read error, or JSON parse error — every
/// failure mode degrades to "no persisted state, use defaults",
/// since a malformed transport.json should never block startup.
#[must_use]
pub fn load_transport(path: &Path) -> Option<TransportSnapshot> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}
