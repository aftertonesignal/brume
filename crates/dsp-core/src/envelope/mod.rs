// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Envelope generators used by the Brume engine.
//!
//! - [`Adsr`] — standard four-stage envelope with retrigger
//!
//! The kit's `EnvelopeFollower` is intentionally not vendored — nothing
//! in the Brume code path consumes it.

mod adsr;

pub use adsr::{Adsr, AdsrStage};
