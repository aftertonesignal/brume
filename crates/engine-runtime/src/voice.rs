// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Individual voice: oscillator core → filter → envelopes → DC blocker.
//!
//! The oscillator core runs at 2× the output rate and is decimated
//! back via one 11-tap halfband. Hot FM patches at high carrier
//! pitch with high modulator ratios will alias somewhat — moving to
//! 4× nearly tripled per-voice CPU and pegged the audio thread on
//! the CM5 with both FM and Timbral parts active, so the trade-off
//! reverts to "alias slightly, run at all." Revisit with dynamic
//! oversampling (only when fm-index demands it) or a release-tail
//! early-out when CPU headroom comes back.
//!
//! Filter, envelopes, and the DC blocker stay at the output rate.

use brume_common::ParameterId;
use brume_dsp_core::{
    Adsr, AdsrStage, DcBlocker, HalfbandDecimator, Smoother, StateVariableFilter, SvfMode,
};

use crate::oscillator_core::{Engine, OscillatorCore};

/// Oversampling factor applied to each voice's oscillator path.
const OVERSAMPLE: f32 = 2.0;

/// Amp-envelope value below which a voice in the Release segment
/// stops running its OSC + filter + DC chain and emits zero. The
/// chain's output is multiplied by `amp` on the way out, so once
/// `amp` is well below audible (-60 dB ≈ 0.001), the chain's work
/// is wasted CPU. Conservative enough that even six voices
/// summing at this level (-44 dB total) stay clearly inaudible
/// against any program material.
const RELEASE_TAIL_THRESHOLD: f32 = 0.001;

/// A single Brume voice.
///
/// Contains an oscillator core (FM / Harmonic / Timbral / Granular),
/// a state-variable filter with envelope modulation, amp envelope
/// with velocity scaling, and a DC blocker.
pub struct BrumeVoice {
    osc_core: OscillatorCore,
    /// 2× → output-rate halfband decimator.
    osc_decimator: HalfbandDecimator,
    filter: StateVariableFilter,
    amp_env: Adsr,
    filter_env: Adsr,
    dc_blocker: DcBlocker,

    filter_cutoff: Smoother,
    filter_env_depth: Smoother,

    /// Static FmIndex floor — what the user sets via the MOD tab's FmIndex.
    /// The envelope below (fm_index_env × fm_index_env_depth) is added on
    /// top per sample; the sum is what the oscillator actually sees.
    fm_index_base: Smoother,
    /// Per-voice FM-index envelope. Gate-on on note_on, gate-off on
    /// note_off. Lets each voice's FM depth decay independently from
    /// other sustaining voices — required for DX7-style tine character.
    fm_index_env: Adsr,
    /// How much of the envelope adds onto the static FmIndex (0..10).
    /// Zero means envelope is off; the oscillator sees only the floor.
    fm_index_env_depth: Smoother,
    /// Modulation-router contribution to FmIndex. Updated per buffer by
    /// the part-level mod router (LFOs / Seqs routed to FmIndex). Kept
    /// as a separate field rather than folded into fm_index_base so
    /// mod routing never silently overwrites the user's dialed-in base.
    fm_index_mod: f32,
    /// Modulation-router contribution to FilterCutoff (Hz, signed).
    filter_cutoff_mod: f32,

    /// Per-note velocity, smoothed so a retrigger's velocity step
    /// glides rather than stepping the already-sounding output. This
    /// is the declick: on an active retrigger the oscillator phase and
    /// amp envelope are already continuous, so velocity is the only
    /// discontinuity — gliding it is enough, with no output-domain
    /// cross-fade (which injected a DC transient of its own).
    velocity: Smoother,
    active: bool,
    note: u8,
    /// Unbent frequency of the current note (Hz). Stored so the part's
    /// pitch modulation (bend + vibrato) can rescale the oscillator
    /// frequency each block without re-deriving it from the note.
    base_freq: f32,
    /// Per-oscillator-mode output makeup gain — calibrates the four
    /// engines to a consistent loudness (they normalize very
    /// differently: Harmonic ran ~8 dB below FM, Timbral ~2.5 dB above).
    /// Set in `set_oscillator_mode`; applied to the oscillator output.
    mode_gain: f32,
    sample_rate: f32,
}

impl BrumeVoice {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut filter = StateVariableFilter::new(sample_rate);
        filter.set_mode(SvfMode::Lowpass);
        filter.set_cutoff(2000.0);
        filter.set_resonance(0.15);

        let mut amp_env = Adsr::new(sample_rate);
        amp_env.set_attack_ms(5.0);
        amp_env.set_decay_ms(200.0);
        amp_env.set_sustain(0.7);
        amp_env.set_release_ms(300.0);

        // Filter envelope defaults: plucky shape — opens on attack, closes
        let mut filter_env = Adsr::new(sample_rate);
        filter_env.set_attack_ms(5.0);
        filter_env.set_decay_ms(300.0);
        filter_env.set_sustain(0.0);
        filter_env.set_release_ms(200.0);

        // FM-index envelope defaults: classic DX7-style tine decay.
        // Fast attack, medium decay, zero sustain, moderate release.
        // EnvDepth defaults to 0 so the envelope has no audible effect
        // until the user dials it up — preserves existing-patch behavior.
        let mut fm_index_env = Adsr::new(sample_rate);
        fm_index_env.set_attack_ms(2.0);
        fm_index_env.set_decay_ms(800.0);
        fm_index_env.set_sustain(0.0);
        fm_index_env.set_release_ms(400.0);

        Self {
            // Oscillator core runs at 2× the output rate; the
            // halfband decimator collapses each pair of oversampled
            // samples back to one output sample.
            osc_core: OscillatorCore::new_fm(sample_rate * OVERSAMPLE),
            osc_decimator: HalfbandDecimator::new(),
            filter,
            amp_env,
            filter_env,
            dc_blocker: DcBlocker::new(sample_rate),
            filter_cutoff: Smoother::new(2000.0, 5.0, sample_rate),
            filter_env_depth: Smoother::new(0.5, 5.0, sample_rate),
            fm_index_base: Smoother::new(0.0, 5.0, sample_rate),
            fm_index_env,
            fm_index_env_depth: Smoother::new(0.0, 5.0, sample_rate),
            fm_index_mod: 0.0,
            filter_cutoff_mod: 0.0,
            velocity: Smoother::new(0.0, 5.0, sample_rate),
            active: false,
            note: 0,
            base_freq: 0.0,
            mode_gain: 1.0,
            sample_rate,
        }
    }

    /// Triggers the voice with a MIDI note and velocity.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        // Hard reset only on a fresh activation. Active retriggers let
        // the oscillator, filter, DC blocker, decimators, and envelopes
        // carry their state through — `set_frequency` updates the phase
        // increment but phase stays continuous, and the amp envelope
        // continues from its current value (it does not reset to zero),
        // so the waveform flows through the trigger sample with no hard
        // cut. The one genuine discontinuity a retrigger introduces is
        // the velocity step, which the smoothed `velocity` below glides
        // over — so no output-domain cross-fade is needed (and the old
        // one injected a DC transient that read as an attack click on
        // rapid same-note repeats).
        let fresh = !self.active;
        if fresh {
            self.osc_core.reset();
            if self.osc_core.needs_hard_reset() {
                self.filter.reset();
                self.dc_blocker.reset();
            }
            self.osc_decimator.reset();
        }

        self.note = note;
        self.active = true;
        // A fresh voice starts silent (amp ramps from 0), so snap
        // velocity straight to its target; an active retrigger glides
        // so the change can't step the already-sounding output.
        if fresh {
            self.velocity.set_immediate(velocity);
        } else {
            self.velocity.set_target(velocity);
        }

        let freq = midi_to_freq(note);
        self.base_freq = freq;
        self.osc_core.set_frequency(freq);
        self.amp_env.gate_on();
        self.filter_env.gate_on();
        self.fm_index_env.gate_on();
    }

    /// Rescales the oscillator to the current pitch-modulation factor
    /// (`2^(semitones/12)`), applied per block by the owning part for
    /// pitch bend + vibrato. `1.0` restores the note's unbent pitch.
    /// Phase stays continuous — `set_frequency` only changes the
    /// increment — so a bend glides rather than clicks.
    pub fn set_pitch_factor(&mut self, factor: f32) {
        self.osc_core.set_frequency(self.base_freq * factor);
    }

    /// Releases the voice.
    pub fn note_off(&mut self) {
        self.amp_env.gate_off();
        self.filter_env.gate_off();
        self.fm_index_env.gate_off();
    }

    /// Processes one sample through the voice chain.
    #[inline]
    pub fn process(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let amp = self.amp_env.process();
        let filt_env = self.filter_env.process();

        if !self.amp_env.is_active() {
            self.active = false;
            return 0.0;
        }

        // Release-tail early-out. Once the amp envelope drops below
        // an audible noise floor (-60 dB ≈ 0.001) and we're already
        // in the release segment, the rest of the chain produces
        // signal that gets multiplied by `amp` ≈ 0 — pure CPU spent
        // rendering content that won't be heard. Skip the OSC,
        // decimator, filter, and DC blocker; the envelopes still
        // advance so the voice naturally hits Idle at release end
        // and goes inactive on a future iteration. The OSC's phase
        // freezes here — that's fine because the active-retrigger
        // path doesn't reset oscillator state, so a re-trigger
        // resumes from the held phase, with continuous amp and a
        // glided velocity keeping the resumption click-free.
        if amp < RELEASE_TAIL_THRESHOLD && self.amp_env.stage() == AdsrStage::Release {
            return 0.0;
        }

        // Filter cutoff = user base + filter-env contribution + mod-router contribution.
        // filter_cutoff_mod is a signed Hz offset pushed in by the part-level
        // modulation router each buffer; it rides on top of the base so the
        // user's dialed FilterCutoff isn't silently replaced by routed LFOs.
        let base_cutoff = self.filter_cutoff.process();
        let env_depth = self.filter_env_depth.process();
        let max_cutoff = self.sample_rate * 0.49;
        let effective_cutoff = (base_cutoff
            + env_depth * filt_env * (max_cutoff - base_cutoff)
            + self.filter_cutoff_mod)
            .clamp(20.0, max_cutoff);
        self.filter.set_cutoff(effective_cutoff);

        // FM index = static floor + envelope contribution + mod-
        // router offset, pushed per-sample so the envelope shape
        // actually reaches the oscillator. With env_depth = 0 the
        // envelope drops out and the oscillator sees only the floor.
        let fm_base = self.fm_index_base.process();
        let fm_env_val = self.fm_index_env.process();
        let fm_env_depth = self.fm_index_env_depth.process();
        let fm_effective =
            (fm_base + fm_env_val * fm_env_depth + self.fm_index_mod).clamp(0.0, 10.0);
        self.osc_core.set_fm_index(fm_effective);

        // 2× oscillator → halfband decimator → filter → amp · velocity →
        // DC blocker. The halfband strips would-be imaging before the
        // 2× → 1× downsample. `velocity` is smoothed, so a retrigger's
        // velocity change glides in over a few ms instead of stepping
        // the gain — the declick that lets us skip an output cross-fade.
        let osc_a = self.osc_core.process();
        let osc_b = self.osc_core.process();
        let osc_out = self.osc_decimator.process(osc_a, osc_b) * self.mode_gain;
        let filtered = self.filter.process(osc_out);
        let shaped = filtered * amp * self.velocity.process();
        self.dc_blocker.process(shaped)
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[must_use]
    pub fn note(&self) -> u8 {
        self.note
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.note = 0;
        self.velocity.set_immediate(0.0);
        self.osc_core.reset();
        self.osc_decimator.reset();
        self.filter.reset();
        self.amp_env.reset();
        self.filter_env.reset();
        self.fm_index_env.reset();
        self.fm_index_mod = 0.0;
        self.filter_cutoff_mod = 0.0;
        self.dc_blocker.reset();
    }

    /// Sets the modulation-router contribution to FmIndex. Called once
    /// per audio buffer by the part-level mod router when an LFO or
    /// Seq is routed to FmIndex. The value is added on top of the
    /// user's static FmIndex base inside `process()`; it does NOT
    /// overwrite the base. Signed, in the same units as FmIndex (0..10).
    pub fn set_fm_index_mod(&mut self, offset: f32) {
        self.fm_index_mod = offset;
    }

    /// Sets the modulation-router contribution to FilterCutoff. Signed
    /// Hz offset, added to the base cutoff in `process()`.
    pub fn set_filter_cutoff_mod(&mut self, hz: f32) {
        self.filter_cutoff_mod = hz;
    }

    // --- Parameter dispatch ---

    /// Apply a control-message parameter write to this voice. Voice-level
    /// concerns (filter, envelopes, FmIndex base smoother) are handled
    /// inline; everything else is forwarded to the oscillator core,
    /// which silently drops parameters that don't belong to its engine.
    pub fn set_parameter(&mut self, id: ParameterId, value: f32) {
        match id {
            // Filter — voice owns the SVF
            ParameterId::FilterCutoff => {
                let v = value.clamp(20.0, 20000.0);
                // When inactive, jump to the target so note_on doesn't start
                // with a 5 ms cutoff ramp from the Smoother's stale value —
                // audibly reads as a "filter sweep attack" when the user
                // didn't ask for one.
                if self.active {
                    self.filter_cutoff.set_target(v);
                } else {
                    self.filter_cutoff.set_immediate(v);
                }
            }
            ParameterId::FilterResonance => self.filter.set_resonance(value.clamp(0.0, 1.0)),
            ParameterId::FilterEnvDepth => self.filter_env_depth.set_target(value.clamp(0.0, 1.0)),

            // Filter envelope
            ParameterId::FilterEnvAttack => self.filter_env.set_attack_ms(value),
            ParameterId::FilterEnvDecay => self.filter_env.set_decay_ms(value),
            ParameterId::FilterEnvSustain => self.filter_env.set_sustain(value),
            ParameterId::FilterEnvRelease => self.filter_env.set_release_ms(value),

            // Amp envelope
            ParameterId::AmpAttack => self.amp_env.set_attack_ms(value),
            ParameterId::AmpDecay => self.amp_env.set_decay_ms(value),
            ParameterId::AmpSustain => self.amp_env.set_sustain(value),
            ParameterId::AmpRelease => self.amp_env.set_release_ms(value),

            // Static FmIndex floor — the oscillator sees floor + env + mod
            // each sample. When inactive, jump the smoother directly so
            // note_on doesn't ramp from a stale 0 and audibly change the
            // FM character at onset.
            ParameterId::FmIndex => {
                if self.active {
                    self.fm_index_base.set_target(value);
                } else {
                    self.fm_index_base.set_immediate(value);
                }
            }

            // Per-voice FM-index envelope
            ParameterId::FmIndexEnvDepth => {
                self.fm_index_env_depth.set_target(value.clamp(0.0, 10.0))
            }
            ParameterId::FmIndexEnvAttack => self.fm_index_env.set_attack_ms(value),
            ParameterId::FmIndexEnvDecay => self.fm_index_env.set_decay_ms(value),
            ParameterId::FmIndexEnvSustain => self.fm_index_env.set_sustain(value),
            ParameterId::FmIndexEnvRelease => self.fm_index_env.set_release_ms(value),

            // Engine-specific parameters — let the loaded engine handle
            // them. Any ID that doesn't match this engine's architecture
            // is silently ignored inside the engine's set_parameter.
            _ => self.osc_core.set_parameter(id, value),
        }
    }

    /// Switches the oscillator mode, replacing the osc core. The new
    /// core runs at 2× the output rate (see OVERSAMPLE above) so the
    /// halfband decimator downstream sees a consistent 2:1 input:output
    /// ratio regardless of engine type.
    pub fn set_oscillator_mode(&mut self, mode: brume_common::OscillatorMode) {
        use brume_common::OscillatorMode;
        let osc_rate = self.sample_rate * OVERSAMPLE;
        self.osc_core = match mode {
            OscillatorMode::Fm => OscillatorCore::new_fm(osc_rate),
            OscillatorMode::Harmonic => OscillatorCore::new_harmonic(osc_rate),
            OscillatorMode::Timbral => OscillatorCore::new_timbral(osc_rate),
            OscillatorMode::Granular => OscillatorCore::new_granular(osc_rate),
        };
        // Cross-engine loudness calibration (measured via the preset
        // audition harness against a common reference tone). FM is the
        // 1.0 reference; the others are brought toward it, capped so
        // peaks stay below the stem saturator's 0.8 knee.
        self.mode_gain = match mode {
            OscillatorMode::Fm => 1.0,
            OscillatorMode::Harmonic => 2.0,
            OscillatorMode::Timbral => 0.75,
            OscillatorMode::Granular => 1.2,
        };
        // Fresh decimator state on mode change — otherwise a held
        // tail from the previous engine would leak into the new one.
        self.osc_decimator.reset();
    }
}

/// Converts a MIDI note number to frequency in Hz (A4 = 440 Hz).
fn midi_to_freq(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_produces_output_when_triggered() {
        let mut voice = BrumeVoice::new(48000.0);
        voice.note_on(69, 0.8);

        let mut has_output = false;
        for _ in 0..480 {
            if voice.process().abs() > 0.001 {
                has_output = true;
                break;
            }
        }
        assert!(has_output, "triggered voice should produce non-zero output");
    }

    #[test]
    fn active_retrigger_injects_no_transient() {
        // Pin a patch where a same-note/same-velocity retrigger is a
        // true no-op: sustain = 1.0 (amp stays at 1.0 across the
        // re-attack) and no filter-envelope motion. Two identical
        // voices run in lockstep; one is retriggered mid-sustain, the
        // other isn't. With the declick correct they stay sample-
        // identical, because oscillator phase and amp are continuous
        // and velocity doesn't move. The old output cross-fade instead
        // froze a sample and faded it as DC — a ~0.19 divergence here,
        // which is the attack click on overlapping 1/16-note repeats.
        let mk = || {
            let mut v = BrumeVoice::new(48000.0);
            v.set_parameter(ParameterId::FilterEnvDepth, 0.0);
            v.set_parameter(ParameterId::AmpSustain, 1.0);
            v.note_on(60, 0.9);
            v
        };
        let mut retriggered = mk();
        let mut reference = mk();
        for _ in 0..2000 {
            retriggered.process();
            reference.process(); // both reach the amp plateau
        }

        retriggered.note_on(60, 0.9); // no-op retrigger on one voice only

        let mut max_diff = 0.0_f32;
        let mut peak = 0.0_f32;
        for _ in 0..480 {
            let a = retriggered.process();
            let b = reference.process();
            max_diff = max_diff.max((a - b).abs());
            peak = peak.max(a.abs());
        }
        assert!(
            peak > 0.05,
            "precondition: voice should be sounding ({peak})"
        );
        assert!(
            max_diff < 1e-3,
            "no-op retrigger diverged from a steady voice (attack click): max_diff={max_diff}"
        );
    }

    #[test]
    fn voice_silences_after_release() {
        let mut voice = BrumeVoice::new(48000.0);
        voice.note_on(69, 0.8);

        for _ in 0..4800 {
            voice.process();
        }

        voice.note_off();

        for _ in 0..48000 {
            voice.process();
        }

        assert!(!voice.is_active(), "voice should be inactive after release");
        assert!(
            voice.process().abs() < 0.0001,
            "voice should be silent after release"
        );
    }

    #[test]
    fn release_tail_returns_zero_below_threshold() {
        // The threshold→idle band is narrow (linear release passes
        // through the last 0.001 of the curve in a fraction of a
        // ms), so use a long release to widen it: a 2 s release
        // from sustain 0.5 spends roughly 4 ms below the
        // threshold while still active — comfortable window for
        // the assertion. Voice should return exactly 0.0 within
        // that window even though `is_active()` is still true
        // (envelopes are still advancing toward Idle).
        let mut voice = BrumeVoice::new(48000.0);
        voice.set_parameter(ParameterId::AmpSustain, 0.5);
        voice.set_parameter(ParameterId::AmpDecay, 50.0);
        voice.set_parameter(ParameterId::AmpRelease, 2000.0);
        voice.note_on(69, 0.8);

        // Park at sustain.
        for _ in 0..4800 {
            voice.process();
        }
        voice.note_off();

        // Run the full release window plus a margin; we should
        // observe at least one sample where the early-out fires.
        let mut hit_skip = false;
        for _ in 0..120_000 {
            let active_before = voice.is_active();
            let out = voice.process();
            if out == 0.0 && active_before {
                hit_skip = true;
                break;
            }
        }
        assert!(
            hit_skip,
            "expected the release-tail early-out to fire while voice is still active"
        );
    }

    #[test]
    fn release_tail_skip_does_not_block_natural_inactivation() {
        // Same shape as above; after the full release window the
        // voice must transition to inactive on its own — the skip
        // path can't prevent the eventual amp.is_active()-driven
        // shutoff.
        let mut voice = BrumeVoice::new(48000.0);
        voice.set_parameter(ParameterId::AmpSustain, 0.5);
        voice.set_parameter(ParameterId::AmpDecay, 50.0);
        voice.set_parameter(ParameterId::AmpRelease, 2000.0);
        voice.note_on(69, 0.8);
        for _ in 0..4800 {
            voice.process();
        }
        voice.note_off();
        for _ in 0..120_000 {
            voice.process();
        }
        assert!(
            !voice.is_active(),
            "voice should naturally finish releasing"
        );
    }

    #[test]
    fn velocity_scales_amplitude() {
        let mut loud = BrumeVoice::new(48000.0);
        let mut soft = BrumeVoice::new(48000.0);

        loud.note_on(69, 1.0);
        soft.note_on(69, 0.2);

        // Let attack settle
        let mut loud_peak = 0.0_f32;
        let mut soft_peak = 0.0_f32;
        for _ in 0..4800 {
            loud_peak = loud_peak.max(loud.process().abs());
            soft_peak = soft_peak.max(soft.process().abs());
        }

        assert!(
            loud_peak > soft_peak * 1.5,
            "loud ({loud_peak}) should be significantly louder than soft ({soft_peak})"
        );
    }

    #[test]
    fn midi_to_freq_a4() {
        let freq = midi_to_freq(69);
        assert!((freq - 440.0).abs() < 0.01);
    }
}
