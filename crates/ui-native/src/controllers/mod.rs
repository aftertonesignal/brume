// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Concrete control surface drivers shipped with Brume.
//!
//! The trait + registry definitions live in `brume_midi_io`; this
//! module owns the device-specific impls and the factory that wires
//! them into a registry instance.
//!
//! `impl ControlSurfaceApi for NativeUi` lives in the parent `lib.rs`
//! next to the type definition — the impl reaches into private
//! fields and helper methods, so siting it in lib.rs keeps the
//! visibility seams tight.

pub mod launch_control_xl3;
pub mod nanokontrol2;

use brume_midi_io::ControlSurfaceRegistry;

/// The canonical "everything Brume ships first-class support for"
/// registry. Constructed once at startup and shared via `Arc`.
#[must_use]
pub fn shipped_registry() -> ControlSurfaceRegistry {
    ControlSurfaceRegistry::new(vec![
        Box::new(nanokontrol2::NanoKontrol2),
        Box::new(launch_control_xl3::LaunchControlXl3),
    ])
}
