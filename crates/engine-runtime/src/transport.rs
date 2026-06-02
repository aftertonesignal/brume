// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Audio transport — clock, tempo, beat position tracking.
//!
//! Supports both internal clock (free-running BPM) and external MIDI
//! clock (24 PPQN slave). The transport provides beat position to the
//! delay (for tempo-synced divisions), step sequencers, and future
//! arpeggiators.

/// MIDI clock sends 24 pulses per quarter note.
const PPQN: f32 = 24.0;

/// Clock source mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockMode {
    /// Internal clock at the configured BPM.
    Internal,
    /// Slave to incoming MIDI clock messages.
    External,
}

/// Number of MIDI clock ticks to accumulate before re-estimating BPM.
/// 24 ticks = one quarter note, so this yields ~2Hz update at 120 BPM.
///
/// Was 12 originally (half-beat, ~4Hz). Widened to 24 on 2026-04-18 when
/// Meridian Stage 5 added 8-channel USB isoc alongside the f_midi endpoint
/// and tick-to-sample correlation got visibly noisier — the shorter window
/// was passing scheduling jitter through to the reported BPM. One full
/// beat per estimate averages USB scheduling noise down to sub-BPM levels
/// at the cost of a slightly slower response to real tempo changes.
const EXT_BPM_WINDOW_TICKS: u32 = 24;

/// Sanity clamp on the external BPM estimate. Anything outside this
/// window is almost certainly a host bug or clock flood (the Bitwig
/// double-clock case lands at 240 — still within range; a clock storm
/// from a misbehaving USB controller could push the estimator
/// arbitrarily high). Without the clamp, one bad window of ticks
/// poisons the IIR with a giant value that takes seconds to decay,
/// during which delay sync and visible tempo are wildly off.
const MIN_EXT_BPM: f32 = 30.0;
const MAX_EXT_BPM: f32 = 300.0;

/// Audio transport with beat-accurate position tracking.
pub struct Transport {
    bpm: f32,
    beat_position: f64,
    playing: bool,
    mode: ClockMode,
    sample_rate: f32,

    // External clock state — window-based BPM estimator.
    // We accumulate samples (via advance) and ticks (via midi_clock_tick),
    // then compute BPM over the complete window. This is robust to the
    // "multiple ticks drain in one audio callback" scenario that the
    // pairwise-interval estimator handled incorrectly.
    ext_tick_count: u32,
    ext_window_samples: u32,
    ext_window_ticks: u32,
    ext_bpm_estimate: f32,
}

impl Transport {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            bpm: 120.0,
            beat_position: 0.0,
            playing: true,
            mode: ClockMode::Internal,
            sample_rate,
            ext_tick_count: 0,
            ext_window_samples: 0,
            ext_window_ticks: 0,
            ext_bpm_estimate: 120.0,
        }
    }

    /// Current BPM (internal or estimated from external clock).
    #[must_use]
    pub fn bpm(&self) -> f32 {
        match self.mode {
            ClockMode::Internal => self.bpm,
            ClockMode::External => self.ext_bpm_estimate,
        }
    }

    /// Current beat position (fractional beats from start).
    #[must_use]
    pub fn beat_position(&self) -> f64 {
        self.beat_position
    }

    /// Whether the transport is playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    #[must_use]
    pub fn mode(&self) -> ClockMode {
        self.mode
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm.clamp(20.0, 300.0);
    }

    pub fn set_mode(&mut self, mode: ClockMode) {
        self.mode = mode;
    }

    /// Converts a beat division to milliseconds at the current tempo.
    #[must_use]
    pub fn division_to_ms(&self, beats: f32) -> f32 {
        let current_bpm = self.bpm();
        if current_bpm < 1.0 {
            return 500.0;
        }
        beats * 60_000.0 / current_bpm
    }

    /// Advance the internal clock by one audio buffer.
    /// Call once per `process_block` with the number of frames.
    pub fn advance(&mut self, num_frames: usize) {
        if !self.playing {
            return;
        }

        match self.mode {
            ClockMode::Internal => {
                let beats_per_sample = f64::from(self.bpm) / (60.0 * f64::from(self.sample_rate));
                self.beat_position += beats_per_sample * num_frames as f64;
            }
            ClockMode::External => {
                // Accumulate samples into the current estimator window.
                // BPM is recomputed once per window (see midi_clock_tick).
                self.ext_window_samples = self.ext_window_samples.saturating_add(num_frames as u32);
            }
        }
    }

    /// Handle a MIDI clock tick (0xF8). Called 24 times per quarter note.
    ///
    /// BPM is re-estimated once every `EXT_BPM_WINDOW_TICKS` ticks by
    /// dividing total samples in the window by total ticks — robust to
    /// audio callbacks that drain multiple ticks at once.
    ///
    /// Skipped when transport isn't playing. DAWs (Bitwig, Logic, Ableton)
    /// continue sending MIDI clock at 24 PPQN regardless of play state per
    /// the MIDI spec, but `advance` only accumulates samples while playing.
    /// If we counted ticks during a stop the samples-per-tick denominator
    /// would stay small while the tick numerator climbed, yielding an
    /// artificial high-BPM reading (saw 219 reported from 133 real).
    /// Freezing here keeps the last estimate visible until play resumes.
    pub fn midi_clock_tick(&mut self) {
        if self.mode != ClockMode::External {
            return;
        }
        if !self.playing {
            return;
        }

        self.ext_tick_count = self.ext_tick_count.saturating_add(1);
        self.ext_window_ticks = self.ext_window_ticks.saturating_add(1);
        self.beat_position = f64::from(self.ext_tick_count) / f64::from(PPQN);

        if self.ext_window_ticks >= EXT_BPM_WINDOW_TICKS {
            // Average tick interval over the window.
            let samples_per_tick = self.ext_window_samples as f32 / self.ext_window_ticks as f32;
            let tick_seconds = samples_per_tick / self.sample_rate;
            if tick_seconds > 1.0e-6 {
                // Clamp before the IIR so a wildly-out-of-range
                // reading (clock storm → 600 BPM, dropped ticks →
                // 15 BPM) doesn't drag the smoothed estimate around
                // for seconds. The clamp on the result itself is
                // belt-and-suspenders against accumulated drift.
                let estimated = (60.0 / (tick_seconds * PPQN)).clamp(MIN_EXT_BPM, MAX_EXT_BPM);
                // Heavier IIR on top of the widened window — the window
                // averages USB scheduling jitter over one full beat; the
                // IIR further smooths residual drift. 0.8/0.2 feels right
                // empirically for Bitwig at 133 BPM alongside 8-channel
                // USB isoc. Settling on a real tempo change: ~1–2 s.
                self.ext_bpm_estimate =
                    (self.ext_bpm_estimate * 0.8 + estimated * 0.2).clamp(MIN_EXT_BPM, MAX_EXT_BPM);
            }
            self.ext_window_samples = 0;
            self.ext_window_ticks = 0;
        }
    }

    /// Handle MIDI Start (0xFA).
    pub fn midi_start(&mut self) {
        self.playing = true;
        self.beat_position = 0.0;
        self.ext_tick_count = 0;
        self.ext_window_samples = 0;
        self.ext_window_ticks = 0;
    }

    /// Handle MIDI Stop (0xFC).
    pub fn midi_stop(&mut self) {
        self.playing = false;
    }

    /// Handle MIDI Continue (0xFB).
    pub fn midi_continue(&mut self) {
        self.playing = true;
    }
}

/// Musical time divisions for tempo-synced delay.
#[derive(Debug, Clone, Copy)]
pub enum TimeDivision {
    Free,           // manual ms
    Whole,          // 4 beats
    Half,           // 2 beats
    Quarter,        // 1 beat
    Eighth,         // 0.5 beats
    Sixteenth,      // 0.25 beats
    DottedHalf,     // 3 beats
    DottedQuarter,  // 1.5 beats
    DottedEighth,   // 0.75 beats
    TripletHalf,    // 1.333 beats
    TripletQuarter, // 0.667 beats
    TripletEighth,  // 0.333 beats
}

impl TimeDivision {
    /// Returns the division length in beats, or None for free mode.
    #[must_use]
    pub fn beats(self) -> Option<f32> {
        Some(match self {
            Self::Free => return None,
            Self::Whole => 4.0,
            Self::Half => 2.0,
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
            Self::DottedHalf => 3.0,
            Self::DottedQuarter => 1.5,
            Self::DottedEighth => 0.75,
            Self::TripletHalf => 4.0 / 3.0,
            Self::TripletQuarter => 2.0 / 3.0,
            Self::TripletEighth => 1.0 / 3.0,
        })
    }

    /// Parse from a UI string.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "1/1" => Self::Whole,
            "1/2" => Self::Half,
            "1/4" => Self::Quarter,
            "1/8" => Self::Eighth,
            "1/16" => Self::Sixteenth,
            "1/2d" => Self::DottedHalf,
            "1/4d" => Self::DottedQuarter,
            "1/8d" => Self::DottedEighth,
            "1/2t" => Self::TripletHalf,
            "1/4t" => Self::TripletQuarter,
            "1/8t" => Self::TripletEighth,
            _ => Self::Free,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_clock_advances() {
        let mut t = Transport::new(48000.0);
        t.set_bpm(120.0);
        // At 120 BPM, 48000 samples = 1 second = 2 beats
        t.advance(48000);
        assert!((t.beat_position() - 2.0).abs() < 0.01);
    }

    #[test]
    fn division_to_ms() {
        let t = Transport::new(48000.0);
        // 120 BPM: quarter note = 500ms
        assert!((t.division_to_ms(1.0) - 500.0).abs() < 1.0);
        // Eighth = 250ms
        assert!((t.division_to_ms(0.5) - 250.0).abs() < 1.0);
        // Dotted quarter = 750ms
        assert!((t.division_to_ms(1.5) - 750.0).abs() < 1.0);
    }

    #[test]
    fn midi_clock_estimates_bpm() {
        let mut t = Transport::new(48000.0);
        t.set_mode(ClockMode::External);
        t.midi_start();

        // Simulate 120 BPM: 24 ticks per beat, 2 beats per second
        // Tick interval = 48000 / (24 * 2) = 1000 samples
        for _ in 0..48 {
            t.advance(1000);
            t.midi_clock_tick();
        }

        let bpm = t.bpm();
        assert!(
            (bpm - 120.0).abs() < 5.0,
            "estimated BPM should be near 120: got {bpm}"
        );
    }

    #[test]
    fn midi_clock_clamps_runaway_estimate() {
        // Simulate a clock storm: ticks come in 10× faster than they
        // should at 120 BPM. The naive estimator would land at ~1200
        // BPM; the clamp must hold it inside the [30, 300] window.
        let mut t = Transport::new(48000.0);
        t.set_mode(ClockMode::External);
        t.midi_start();

        // Tick interval at 1200 BPM is 100 samples (24 ppqn × 20 bps).
        // Drive a full window at that absurd rate.
        for _ in 0..EXT_BPM_WINDOW_TICKS * 2 {
            t.advance(100);
            t.midi_clock_tick();
        }

        let bpm = t.bpm();
        assert!(
            bpm <= MAX_EXT_BPM + 0.5,
            "BPM should be clamped to MAX ({MAX_EXT_BPM}); got {bpm}"
        );
    }

    #[test]
    fn midi_clock_clamps_starved_estimate() {
        // Inverse of the runaway case: ticks come in 10× too slowly.
        // Estimator would land at ~12 BPM; clamp must hold at MIN.
        let mut t = Transport::new(48000.0);
        t.set_mode(ClockMode::External);
        t.midi_start();

        // Tick interval at 12 BPM is 10000 samples per tick.
        for _ in 0..EXT_BPM_WINDOW_TICKS * 2 {
            t.advance(10_000);
            t.midi_clock_tick();
        }

        let bpm = t.bpm();
        assert!(
            bpm >= MIN_EXT_BPM - 0.5,
            "BPM should be clamped to MIN ({MIN_EXT_BPM}); got {bpm}"
        );
    }

    #[test]
    fn time_division_parsing() {
        assert_eq!(TimeDivision::from_name("1/4").beats(), Some(1.0));
        assert_eq!(TimeDivision::from_name("1/8").beats(), Some(0.5));
        assert_eq!(TimeDivision::from_name("1/4d").beats(), Some(1.5));
        assert!(TimeDivision::from_name("FREE").beats().is_none());
    }
}
