// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! A multi-timbral part: independent voice pool, modulation, and parameters.
//!
//! Each part owns 6 voices locked to one oscillator mode and has its own
//! modulation router. The engine owns 4 parts (FM, Harmonic, Timbral, Granular).

use brume_common::{OscillatorMode, ParameterId};
use brume_dsp_core::Smoother;
use brume_modulation::ModulationRouter;

use crate::voice::BrumeVoice;

const VOICES_PER_PART: usize = 6;

/// Pitch-bend range applied at full wheel deflection (± semitones).
const PITCH_BEND_RANGE_SEMITONES: f32 = 2.0;
/// Vibrato LFO rate driven by the mod wheel (Hz).
const VIBRATO_RATE_HZ: f32 = 5.5;
/// Vibrato depth (± semitones) at full mod-wheel deflection.
const VIBRATO_DEPTH_SEMITONES: f32 = 0.4;

/// Mix-headroom exponent: target = 1/active^EXP. Pure `1/√n` (0.5)
/// assumes uncorrelated voices, but a chord's voices correlate and
/// their sum overshoots — pinning the stem saturator. 0.8 ducks dense
/// chords harder so they stay near the knee, at the cost of a chord
/// being a touch quieter relative to a single note.
const HEADROOM_EXPONENT: f32 = 0.8;

/// A single multi-timbral part with its own voice pool and modulation.
pub struct Part {
    voices: [BrumeVoice; VOICES_PER_PART],
    pub modulation: ModulationRouter,
    mode: OscillatorMode,
    next_voice: usize,
    pub level: Smoother,
    pub muted: bool,
    pub delay_send: Smoother,
    pub reverb_send: Smoother,
    // Mix-headroom tracker. Target = 1/sqrt(active voices), smoothed
    // asymmetrically: it ducks *fast* on a chord onset so the summed
    // attack transient can't overshoot the stem saturator (the
    // chord-stab click), but restores *slow* as voices release so the
    // step back up doesn't make long release tails sound like ghost
    // re-triggers — the bug that motivated smoothing this at all.
    headroom: Smoother,
    /// Pitch bend, normalized to [-1, 1] (0 = center). Scaled by
    /// `PITCH_BEND_RANGE_SEMITONES` and applied to every sounding voice.
    pitch_bend: f32,
    /// Mod-wheel position [0, 1], driving vibrato depth.
    mod_wheel: f32,
    /// Free-running vibrato LFO phase [0, 1).
    vibrato_phase: f32,
    /// Last pitch-mod factor pushed to the voices — lets the per-block
    /// update skip work when pitch is neutral and already settled.
    pitch_factor: f32,
    sample_rate: f32,
}

impl Part {
    /// Creates a new part with voices initialized to the given oscillator mode.
    #[must_use]
    pub fn new(mode: OscillatorMode, sample_rate: f32) -> Self {
        // Per-engine filter-cutoff default matches the UI's displayed
        // default for that engine. FM's harmonic content lives above
        // the carrier, so its default is wide-open (18 kHz); the
        // subtractive engines sit in a more traditional range.
        // Without this, the engine's hard-coded 2 kHz default would
        // mute whatever the UI says until the user touches the knob.
        let (cutoff_default, env_depth_default) = match mode {
            OscillatorMode::Fm => (18000.0, 0.0),
            OscillatorMode::Harmonic | OscillatorMode::Timbral => (8000.0, 0.3),
            OscillatorMode::Granular => (4000.0, 0.2),
        };
        let voices = std::array::from_fn(|_| {
            let mut v = BrumeVoice::new(sample_rate);
            v.set_oscillator_mode(mode);
            v.set_parameter(ParameterId::FilterCutoff, cutoff_default);
            v.set_parameter(ParameterId::FilterEnvDepth, env_depth_default);
            v
        });

        Self {
            voices,
            modulation: ModulationRouter::new(sample_rate),
            mode,
            next_voice: 0,
            level: Smoother::new(0.8, 10.0, sample_rate),
            muted: false,
            delay_send: Smoother::new(0.3, 10.0, sample_rate),
            reverb_send: Smoother::new(0.3, 10.0, sample_rate),
            // 2 ms fall ducks fast enough to catch a chord's ~5 ms
            // attack before the summed transient overshoots the stem
            // saturator (the chord-stab click); 30 ms rise restores
            // slowly so a voice ending its release tail doesn't step
            // the survivors up into an audible ghost re-trigger.
            headroom: Smoother::new_asymmetric(1.0, 30.0, 2.0, sample_rate),
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            vibrato_phase: 0.0,
            pitch_factor: 1.0,
            sample_rate,
        }
    }

    /// Sets pitch bend, normalized to [-1, 1] (0 = center). Applied to
    /// all sounding voices on the next block.
    pub fn set_pitch_bend(&mut self, bend: f32) {
        self.pitch_bend = bend.clamp(-1.0, 1.0);
    }

    /// Sets the mod-wheel position [0, 1], which drives vibrato depth.
    pub fn set_mod_wheel(&mut self, value: f32) {
        self.mod_wheel = value.clamp(0.0, 1.0);
    }

    /// Advances the vibrato LFO and pushes the combined bend + vibrato
    /// pitch factor (`2^(semitones/12)`) to every active voice. Called
    /// once per block. Skips the work — and leaves the oscillator at its
    /// note pitch — while bend and mod wheel are both at rest.
    fn apply_pitch_mod(&mut self, num_frames: usize) {
        #[allow(clippy::cast_precision_loss)]
        let dt = num_frames as f32 / self.sample_rate;
        self.vibrato_phase = (self.vibrato_phase + VIBRATO_RATE_HZ * dt).fract();

        let bend_st = self.pitch_bend * PITCH_BEND_RANGE_SEMITONES;
        let vibrato_st = if self.mod_wheel > 0.0 {
            (self.vibrato_phase * std::f32::consts::TAU).sin()
                * self.mod_wheel
                * VIBRATO_DEPTH_SEMITONES
        } else {
            0.0
        };
        let total_st = bend_st + vibrato_st;

        // Neutral and already settled → nothing to do (no per-block
        // set_frequency churn when no wheel is touched).
        if total_st.abs() < 1e-9 && (self.pitch_factor - 1.0).abs() < 1e-9 {
            return;
        }
        let factor = 2.0_f32.powf(total_st / 12.0);
        self.pitch_factor = factor;
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.set_pitch_factor(factor);
            }
        }
    }

    /// The oscillator mode this part is locked to.
    #[must_use]
    pub fn mode(&self) -> OscillatorMode {
        self.mode
    }

    /// Switches all voices to a new oscillator engine mode.
    pub fn set_mode(&mut self, mode: OscillatorMode) {
        self.mode = mode;
        for voice in &mut self.voices {
            voice.set_oscillator_mode(mode);
        }
    }

    /// Triggers a note on this part's voice pool.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.modulation.note_on();
        // Apply the part's current bend to the freshly-triggered voice so
        // a note struck while the wheel is held starts in tune with the
        // notes already sounding (the next block keeps it tracking).
        let factor = self.pitch_factor;

        // Reuse a voice already playing this note
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.is_active() && v.note() == note)
        {
            voice.note_on(note, velocity);
            voice.set_pitch_factor(factor);
            return;
        }

        // Find an inactive voice
        if let Some(voice) = self.voices.iter_mut().find(|v| !v.is_active()) {
            voice.note_on(note, velocity);
            voice.set_pitch_factor(factor);
            return;
        }

        // All voices active — steal round-robin
        self.voices[self.next_voice].note_on(note, velocity);
        self.voices[self.next_voice].set_pitch_factor(factor);
        self.next_voice = (self.next_voice + 1) % VOICES_PER_PART;
    }

    /// Releases a note on this part.
    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.is_active() && voice.note() == note {
                voice.note_off();
            }
        }
    }

    /// Sets a parameter on all voices in this part. The voice routes
    /// the write to its own state (filter/envelopes/FmIndex base) or
    /// hands it to the loaded oscillator engine; engines silently drop
    /// any parameter that doesn't belong to their architecture.
    pub fn set_parameter(&mut self, id: ParameterId, value: f32) {
        self.for_voices(|v| v.set_parameter(id, value));
    }

    /// Advances modulation and processes one buffer's worth of audio.
    ///
    /// Returns the mono sum of all voices in this part (pre-limiter).
    pub fn process_buffer(&mut self, num_frames: usize, output: &mut [f32]) {
        // Pitch bend + vibrato: rescale sounding voices before rendering.
        self.apply_pitch_mod(num_frames);

        // Advance modulation
        self.modulation.advance(num_frames, None);

        // Evaluate routing and apply offsets
        let eval = self.modulation.evaluate();
        let num_offsets = eval.len().min(32);
        let mut offset_buf = [(ParameterId::MasterVolume, 0.0_f32); 32];
        offset_buf[..num_offsets].copy_from_slice(&eval[..num_offsets]);

        for &(param_id, offset) in &offset_buf[..num_offsets] {
            self.apply_modulation_offset(param_id, offset);
        }

        // Render voices into the output buffer (additive). Headroom
        // targets 1/active^HEADROOM_EXPONENT — keeps dense chords under
        // the stem saturator without over-ducking — and is smoothed
        // (asymmetric: fast duck, slow restore) so the step change as
        // voices finish their release tail doesn't pop the survivors.
        let active = self.voices.iter().filter(|v| v.is_active()).count().max(1);
        self.headroom
            .set_target(1.0 / (active as f32).powf(HEADROOM_EXPONENT));

        for frame in output.iter_mut().take(num_frames) {
            let level = self.level.process();
            let headroom = self.headroom.process();
            let mut mix = 0.0;
            if !self.muted {
                for voice in &mut self.voices {
                    mix += voice.process();
                }
            }
            *frame = mix * level * headroom;
        }
    }

    /// Releases all active voices in this part.
    pub fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.note_off();
            }
        }
    }

    /// Number of currently active voices in this part.
    #[must_use]
    pub fn active_voice_count(&self) -> u8 {
        self.voices.iter().filter(|v| v.is_active()).count() as u8
    }

    fn for_voices(&mut self, f: impl Fn(&mut BrumeVoice)) {
        for voice in &mut self.voices {
            f(voice);
        }
    }

    fn apply_modulation_offset(&mut self, param_id: ParameterId, offset: f32) {
        // Modulation-router offsets now ride on top of the user's dialed
        // base value, set once per buffer as a separate field on each
        // voice (v.set_*_mod). Previous behavior silently replaced the
        // base (e.g. "FmIndex = 2.0 + offset * 5.0") which made a user's
        // FmIndex knob movement inaudible whenever an LFO was routed
        // to it, and produced audible step clicks at LFO phase-wrap
        // in Loop mode.
        //
        // `offset` here is the mod-router's signed output in roughly
        // [-depth, +depth]; depth itself is ∈ [-1, 1] on the
        // assignment. We translate that into the natural unit of the
        // destination parameter.
        match param_id {
            ParameterId::FilterCutoff => {
                let hz_offset = offset * 5000.0;
                self.for_voices(|v| v.set_filter_cutoff_mod(hz_offset));
            }
            ParameterId::FmIndex => {
                let idx_offset = offset * 5.0;
                self.for_voices(|v| v.set_fm_index_mod(idx_offset));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_produces_output() {
        let mut part = Part::new(OscillatorMode::Fm, 48000.0);
        part.note_on(69, 0.8);

        let mut buf = vec![0.0_f32; 512];
        part.process_buffer(512, &mut buf);

        let energy: f32 = buf.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "part should produce output");
    }

    #[test]
    fn pitch_bend_shifts_rendered_pitch() {
        // Two identical parts; one gets full up-bend. Their oscillators
        // run at different frequencies, so after a couple of blocks the
        // rendered output diverges — confirming the bend reaches the
        // voices.
        let sr = 48000.0;
        let mut plain = Part::new(OscillatorMode::Fm, sr);
        let mut bent = Part::new(OscillatorMode::Fm, sr);
        plain.note_on(60, 1.0);
        bent.note_on(60, 1.0);
        bent.set_pitch_bend(1.0);

        let mut a = vec![0.0_f32; 512];
        let mut b = vec![0.0_f32; 512];
        for _ in 0..3 {
            plain.process_buffer(512, &mut a);
            bent.process_buffer(512, &mut b);
        }

        let diff: f32 = a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum();
        assert!(
            diff > 0.1,
            "pitch bend should change the rendered output: diff={diff}"
        );
    }

    #[test]
    fn part_voice_count() {
        let mut part = Part::new(OscillatorMode::Harmonic, 48000.0);
        assert_eq!(part.active_voice_count(), 0);

        part.note_on(60, 0.8);
        assert_eq!(part.active_voice_count(), 1);

        part.note_on(64, 0.8);
        assert_eq!(part.active_voice_count(), 2);
    }

    #[test]
    fn part_note_off() {
        let mut part = Part::new(OscillatorMode::Timbral, 48000.0);
        part.note_on(69, 0.8);

        let mut buf = vec![0.0; 512];
        part.process_buffer(512, &mut buf);

        part.note_off(69);

        let mut buf = vec![0.0; 48000];
        part.process_buffer(48000, &mut buf);

        assert_eq!(part.active_voice_count(), 0);
    }

    #[test]
    fn part_mode_is_fixed() {
        let part = Part::new(OscillatorMode::Harmonic, 48000.0);
        assert_eq!(part.mode(), OscillatorMode::Harmonic);
    }

    #[test]
    fn set_parameter_routes_engine_specific_params() {
        // Drive each mode-specific parameter through Part → Voice →
        // OscillatorCore → Engine and confirm rendering stays finite.
        // This is the regression net for the dispatch refactor: a
        // typo'd match arm or a missing engine impl would either
        // silently no-op (caught here only when paired with a
        // value-dependent assertion) or panic / produce NaN.
        let cases = [
            (OscillatorMode::Fm, ParameterId::Algorithm, 0.5),
            (OscillatorMode::Fm, ParameterId::Op1Level, 0.7),
            (OscillatorMode::Harmonic, ParameterId::HarmonicLevel1, 0.9),
            (OscillatorMode::Harmonic, ParameterId::ScanCenter, 0.3),
            (OscillatorMode::Timbral, ParameterId::Timbre, 0.6),
            (OscillatorMode::Timbral, ParameterId::MultiplierStages, 2.0),
            (OscillatorMode::Granular, ParameterId::GranularDensity, 0.5),
            (
                OscillatorMode::Granular,
                ParameterId::GranularGrainSize,
                0.4,
            ),
        ];

        for (mode, id, value) in cases {
            let mut part = Part::new(mode, 48000.0);
            part.set_parameter(id, value);
            // Voice-level params should also flow through cleanly
            // regardless of the loaded engine.
            part.set_parameter(ParameterId::FilterCutoff, 4000.0);
            part.set_parameter(ParameterId::AmpAttack, 10.0);
            // Foreign params (sent to the wrong engine) must be ignored
            // silently — never panic, never NaN.
            part.set_parameter(ParameterId::GranularDensity, 0.5);
            part.set_parameter(ParameterId::Algorithm, 0.5);

            part.note_on(60, 0.8);
            let mut buf = vec![0.0_f32; 512];
            part.process_buffer(512, &mut buf);
            assert!(
                buf.iter().all(|s| s.is_finite()),
                "{mode:?}/{id:?}: non-finite sample after dispatch",
            );
        }
    }
}
