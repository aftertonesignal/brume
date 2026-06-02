// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! MIDI activity tracking for UI visualization.

/// Per-channel MIDI activity.
#[derive(Clone)]
pub struct ChannelActivity {
    /// Per-note velocity (0-127). Decays over time.
    pub note_velocities: [u8; 128],
    /// Per-CC latest value (0-127).
    pub cc_values: [u8; 128],
    /// Ticks-since-last-update per CC slot. Increments each decay; a
    /// slot counts as "active" until it has been quiet for
    /// CC_ACTIVITY_TTL_TICKS calls. Previously cc_active was a
    /// one-way sticky bool — every knob turn permanently enlarged
    /// the serialized activity payload, and the to_js_call string
    /// grew unboundedly over a session, which ate UI-drain time.
    pub cc_idle_ticks: [u8; 128],
    /// Notes currently active.
    pub note_count: u8,
}

/// Number of decay ticks after the last CC update during which the
/// slot is still serialized to the UI. At 20 Hz decay (50 ms tick)
/// this is ~1.5 s of "stickiness" before a CC slot drops off the
/// MIDI window display — long enough for the eye, short enough to
/// stop the JS payload growing forever.
const CC_ACTIVITY_TTL_TICKS: u8 = 30;

impl ChannelActivity {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            note_velocities: [0; 128],
            cc_values: [0; 128],
            cc_idle_ticks: [u8::MAX; 128],
            note_count: 0,
        }
    }

    pub fn decay(&mut self) {
        for v in &mut self.note_velocities {
            *v = v.saturating_sub(4);
        }
        for t in &mut self.cc_idle_ticks {
            *t = t.saturating_add(1);
        }
    }
}

impl Default for ChannelActivity {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks MIDI activity across all 16 channels for the UI display.
pub struct MidiActivity {
    /// Per-channel activity (index 0 = CH 1).
    pub channels: [ChannelActivity; 16],
    /// Connected device name.
    pub device_name: String,
    /// Set on any update; cleared by `mark_clean()` after the UI
    /// timer serializes the payload. Skips evaluate_script RPC
    /// entirely when nothing has changed, avoiding the 20 Hz
    /// "build a string and ship it to the webview" cost while
    /// MIDI is idle.
    dirty: bool,
}

impl MidiActivity {
    #[must_use]
    pub fn new() -> Self {
        // const fn array init
        const EMPTY: ChannelActivity = ChannelActivity::new();
        Self {
            channels: [EMPTY; 16],
            device_name: String::new(),
            dirty: false,
        }
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            if (note as usize) < 128 {
                ch.note_velocities[note as usize] = velocity;
                ch.note_count = ch.note_count.saturating_add(1);
                self.dirty = true;
            }
        }
    }

    pub fn note_off(&mut self, channel: u8, note: u8) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            if (note as usize) < 128 {
                ch.note_velocities[note as usize] = 0;
                ch.note_count = ch.note_count.saturating_sub(1);
                self.dirty = true;
            }
        }
    }

    pub fn cc(&mut self, channel: u8, cc: u8, value: u8) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            if (cc as usize) < 128 {
                ch.cc_values[cc as usize] = value;
                ch.cc_idle_ticks[cc as usize] = 0;
                self.dirty = true;
            }
        }
    }

    /// Decay all channels. Note decay also counts against dirty
    /// (a decaying note is still a visible change), but only while
    /// any note is actually still ringing.
    pub fn decay(&mut self) {
        let mut any_active = false;
        for ch in &mut self.channels {
            for v in &ch.note_velocities {
                if *v > 0 {
                    any_active = true;
                    break;
                }
            }
            if !any_active {
                for t in &ch.cc_idle_ticks {
                    if *t < CC_ACTIVITY_TTL_TICKS {
                        any_active = true;
                        break;
                    }
                }
            }
            ch.decay();
        }
        if any_active {
            self.dirty = true;
        }
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Serializes per-channel activity to a JS function call.
    /// Format: window._midiActivity({ch0_notes, ch0_ccs}, {ch1...}, ..., device)
    #[must_use]
    pub fn to_js_call(&self) -> String {
        let mut parts = String::from("[");

        // Only send channels 0-2 (our 3 parts) for now — expandable
        for ch_idx in 0..3 {
            let ch = &self.channels[ch_idx];
            if ch_idx > 0 {
                parts.push(',');
            }
            parts.push('{');

            // Notes
            parts.push_str("n:[");
            let mut first = true;
            for (i, &v) in ch.note_velocities.iter().enumerate() {
                if v > 0 {
                    if !first {
                        parts.push(',');
                    }
                    parts.push_str(&format!("[{i},{v}]"));
                    first = false;
                }
            }
            parts.push_str("],c:[");

            // CCs — only slots inside the active TTL window.
            first = true;
            for (i, &idle) in ch.cc_idle_ticks.iter().enumerate() {
                if idle < CC_ACTIVITY_TTL_TICKS {
                    if !first {
                        parts.push(',');
                    }
                    parts.push_str(&format!("[{i},{}]", ch.cc_values[i]));
                    first = false;
                }
            }
            parts.push_str("]}");
        }

        parts.push(']');

        // Escape device name for safe JS interpolation
        let escaped_name = self.device_name.replace('\\', "\\\\").replace('\'', "\\'");
        format!("if(window._midiActivity)window._midiActivity({parts},'{escaped_name}')")
    }
}

impl Default for MidiActivity {
    fn default() -> Self {
        Self::new()
    }
}
