// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Per-page view modules.
//!
//! Each engine + utility page lives in its own file. The dispatcher
//! (`NativeUi::body`) and the `Message` enum stay in `lib.rs`; pages
//! contribute additional `impl NativeUi` blocks that hold the page's
//! view methods (and any state-mutating helpers called by `update`
//! that are page-local).
//!
//! Pages are descendants of the crate root, so they can read
//! `NativeUi`'s private fields and crate-private helpers (constants,
//! styling functions, sibling widget modules) directly.

pub(crate) mod engine;
pub(crate) mod library;
pub(crate) mod midi;
pub(crate) mod mix;
pub(crate) mod mod_page;
pub(crate) mod script;
pub(crate) mod sys;
