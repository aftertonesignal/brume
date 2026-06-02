// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Effects chain with pluggable FX slots.
//!
//! Each effect implements [`FxSlot`] — a trait that both built-in Rust
//! effects and future Lua-scripted effects can satisfy. The [`FxChain`]
//! processes audio through an ordered sequence of slots.
//!
//! Per-part send levels route independent amounts of each part's signal
//! to the shared effect buses (delay, reverb). Saturator and chorus run
//! as inserts on the master bus.

mod chorus;
mod dattorro;
mod saturator;
mod stereo_delay;

pub use chorus::ChorusFx;
pub use dattorro::DattorroReverb;
pub use saturator::SaturatorFx;
pub use stereo_delay::StereoDelayFx;

/// Describes a single controllable parameter on an FX slot.
#[derive(Debug, Clone)]
pub struct FxParamDef {
    /// Internal name used for IPC (e.g. "time", "feedback").
    pub name: String,
    /// Display label for the UI (e.g. "TIME", "FEEDBACK").
    pub label: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Optional unit string (e.g. "ms", "Hz", "dB").
    pub unit: Option<String>,
}

/// The interface every effect module must implement.
///
/// Built-in effects implement this directly in Rust. Future Lua effects
/// will wrap a script VM behind this same trait, so the chain and UI
/// treat all effects uniformly.
pub trait FxSlot: Send {
    /// Human-readable name for UI display.
    fn name(&self) -> &str;

    /// Introspectable parameter definitions.
    fn params(&self) -> &[FxParamDef];

    /// Set a parameter by name. Values are clamped to the param's range.
    fn set_param(&mut self, name: &str, value: f32);

    /// Get current parameter value by name.
    fn get_param(&self, name: &str) -> f32;

    /// Process a stereo buffer in-place. Called from the audio thread.
    /// `left` and `right` must be the same length.
    fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]);

    /// Clear all internal state (delay lines, filters, etc).
    fn reset(&mut self);
}

/// The four built-in FX slot names, in construction order. The engine's
/// `process_block` looks these up by name rather than by index, so adding
/// a new built-in no longer requires keeping `FxChain::new` and the
/// audio-callback indices in lockstep.
pub const SATURATOR_SLOT: &str = "Saturator";
pub const CHORUS_SLOT: &str = "Chorus";
pub const DELAY_SLOT: &str = "Delay";
pub const REVERB_SLOT: &str = "Reverb";

/// An ordered chain of effect slots.
///
/// Default chain: Saturator → Chorus → Delay → Reverb.
/// Slots can be reordered, replaced, or extended at runtime.
pub struct FxChain {
    slots: Vec<Box<dyn FxSlot>>,
}

impl FxChain {
    /// Creates the default effects chain.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let slots: Vec<Box<dyn FxSlot>> = vec![
            Box::new(SaturatorFx::new()),
            Box::new(ChorusFx::new(sample_rate)),
            Box::new(StereoDelayFx::new(sample_rate)),
            Box::new(DattorroReverb::new(sample_rate)),
        ];
        Self { slots }
    }

    /// Number of slots in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the chain has no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Appends a new slot to the end of the chain.
    /// If a slot with the same name already exists, replaces it.
    pub fn push(&mut self, slot: Box<dyn FxSlot>) {
        let name = slot.name().to_string();
        if let Some(idx) = self.slots.iter().position(|s| s.name() == name) {
            self.slots[idx] = slot;
        } else {
            self.slots.push(slot);
        }
    }

    /// Removes a slot by name. Returns true if found and removed.
    pub fn remove_by_name(&mut self, name: &str) -> bool {
        if let Some(idx) = self.slots.iter().position(|s| s.name() == name) {
            self.slots.remove(idx);
            true
        } else {
            false
        }
    }

    /// Access a slot by index.
    pub fn slot_mut(&mut self, index: usize) -> Option<&mut (dyn FxSlot + 'static)> {
        self.slots.get_mut(index).map(|s| &mut **s)
    }

    /// Access a slot by name. O(n) over the chain — negligible at chain
    /// sizes of ~8 slots and called a handful of times per audio block.
    pub fn slot_mut_by_name(&mut self, name: &str) -> Option<&mut (dyn FxSlot + 'static)> {
        self.slots
            .iter_mut()
            .find(|s| s.name() == name)
            .map(|s| &mut **s)
    }

    /// Iterates custom (non-built-in) slots — every slot whose name
    /// isn't one of the four built-ins. This is how the audio callback
    /// reaches Lua FX without encoding the built-in count as a magic
    /// number (the old `for i in 4..len` loop).
    pub fn custom_slots_mut(&mut self) -> impl Iterator<Item = &mut (dyn FxSlot + 'static)> {
        const BUILTINS: &[&str] = &[SATURATOR_SLOT, CHORUS_SLOT, DELAY_SLOT, REVERB_SLOT];
        self.slots
            .iter_mut()
            .filter(|s| !BUILTINS.contains(&s.name()))
            .map(|s| &mut **s)
    }

    /// Find a slot by name and set a parameter on it.
    pub fn set_param(&mut self, slot_name: &str, param_name: &str, value: f32) {
        for slot in &mut self.slots {
            if slot.name() == slot_name {
                slot.set_param(param_name, value);
                return;
            }
        }
    }

    /// Process stereo audio through all slots in order.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        for slot in &mut self.slots {
            slot.process_stereo(left, right);
        }
    }

    /// Reset all slots.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_creates_with_four_slots() {
        let chain = FxChain::new(48000.0);
        assert_eq!(chain.len(), 4);
    }

    #[test]
    fn chain_processes_silence() {
        let mut chain = FxChain::new(48000.0);
        let mut left = vec![0.0_f32; 512];
        let mut right = vec![0.0_f32; 512];
        chain.process_stereo(&mut left, &mut right);
        // All effects at default mix=0 should pass silence through
        assert!(left.iter().all(|&s| s.abs() < 0.001));
    }

    #[test]
    fn set_param_by_name() {
        let mut chain = FxChain::new(48000.0);
        chain.set_param("Saturator", "drive", 0.5);
        // Should not panic, param should be set
        for slot in &chain.slots {
            if slot.name() == "Saturator" {
                assert!((slot.get_param("drive") - 0.5).abs() < 0.01);
            }
        }
    }
}
