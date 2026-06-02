// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Reusable view-layer helpers that aren't whole pages.
//!
//! `styles` holds the closures and free functions that paint Brume's
//! chrome — pick_list field + popover, segmented-pill buttons, the
//! universal `panel` wrapper, and the optional phosphor-glow shadow.
//! Page modules and the chassis both reach into this module rather
//! than redefining the visual tokens locally.

pub(crate) mod styles;
