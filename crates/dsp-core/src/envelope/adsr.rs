// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! ADSR envelope generator.

/// Current stage of the ADSR envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdsrStage {
    /// Envelope is idle (not triggered).
    Idle,
    /// Attack phase — rising from 0 to 1.
    Attack,
    /// Decay phase — falling from 1 to sustain level.
    Decay,
    /// Sustain phase — holding at sustain level.
    Sustain,
    /// Release phase — falling from current level to 0.
    Release,
}

/// ADSR (Attack-Decay-Sustain-Release) envelope generator. Standard
/// four-stage synth envelope: attack time to rise 0→1, decay time to
/// fall 1→sustain, sustain level held while gate is on, release time
/// to fall from current value to 0 after gate-off.
///
/// # Example
///
/// ```
/// use brume_dsp_core::envelope::Adsr;
///
/// let mut env = Adsr::new(44100.0);
/// env.set_attack_ms(10.0);
/// env.set_decay_ms(50.0);
/// env.set_sustain(0.7);
/// env.set_release_ms(200.0);
///
/// env.gate_on();
/// for _ in 0..1000 {
///     let _ = env.process();
/// }
/// env.gate_off();
/// ```
pub struct Adsr {
    sample_rate: f32,
    stage: AdsrStage,
    value: f32,
    attack_ms: f32,
    decay_ms: f32,
    sustain: f32,
    release_ms: f32,
    attack_inc: f32,
    decay_inc: f32,
    release_inc: f32,
}

impl Adsr {
    /// Creates a new ADSR envelope. Defaults: 10ms attack, 50ms decay,
    /// 0.7 sustain, 200ms release.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let mut env = Self {
            sample_rate,
            stage: AdsrStage::Idle,
            value: 0.0,
            attack_ms: 10.0,
            decay_ms: 50.0,
            sustain: 0.7,
            release_ms: 200.0,
            attack_inc: 0.0,
            decay_inc: 0.0,
            release_inc: 0.0,
        };
        env.update_increments();
        env
    }

    /// Sets the sample rate and recalculates increments.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update_increments();
    }

    /// Sets the attack time in milliseconds.
    pub fn set_attack_ms(&mut self, ms: f32) {
        self.attack_ms = ms.max(0.1);
        self.update_attack_increment();
    }

    /// Sets the decay time in milliseconds.
    pub fn set_decay_ms(&mut self, ms: f32) {
        self.decay_ms = ms.max(0.1);
        self.update_decay_increment();
    }

    /// Sets the sustain level (0.0–1.0).
    pub fn set_sustain(&mut self, level: f32) {
        self.sustain = level.clamp(0.0, 1.0);
        self.update_decay_increment();
    }

    /// Sets the release time in milliseconds.
    pub fn set_release_ms(&mut self, ms: f32) {
        self.release_ms = ms.max(0.1);
        self.update_release_increment();
    }

    /// Returns the current envelope value.
    #[must_use]
    pub fn current(&self) -> f32 {
        self.value
    }

    /// Returns the current stage.
    #[must_use]
    pub fn stage(&self) -> AdsrStage {
        self.stage
    }

    /// Returns true if the envelope is active (not idle).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.stage != AdsrStage::Idle
    }

    /// Triggers the envelope. Value isn't reset so retriggering from
    /// mid-attack or mid-release continues smoothly from current value.
    pub fn gate_on(&mut self) {
        self.stage = AdsrStage::Attack;
    }

    /// Releases the envelope. Recalculates release increment from the
    /// current value so the release time is honoured regardless of the
    /// stage the gate-off happened in.
    pub fn gate_off(&mut self) {
        if self.stage != AdsrStage::Idle {
            self.stage = AdsrStage::Release;
            self.update_release_increment();
        }
    }

    /// Resets the envelope to idle state.
    pub fn reset(&mut self) {
        self.stage = AdsrStage::Idle;
        self.value = 0.0;
    }

    /// Processes one sample and returns the envelope value.
    #[inline]
    pub fn process(&mut self) -> f32 {
        match self.stage {
            AdsrStage::Idle => {}
            AdsrStage::Attack => {
                self.value += self.attack_inc;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                self.value -= self.decay_inc;
                if self.value <= self.sustain {
                    self.value = self.sustain;
                    self.stage = AdsrStage::Sustain;
                }
            }
            AdsrStage::Sustain => {
                self.value = self.sustain;
            }
            AdsrStage::Release => {
                self.value -= self.release_inc;
                if self.value <= 0.0 {
                    self.value = 0.0;
                    self.stage = AdsrStage::Idle;
                }
            }
        }
        self.value
    }

    fn update_increments(&mut self) {
        self.update_attack_increment();
        self.update_decay_increment();
        self.update_release_increment();
    }

    fn update_attack_increment(&mut self) {
        let samples = self.attack_ms * self.sample_rate / 1000.0;
        self.attack_inc = if samples > 0.0 { 1.0 / samples } else { 1.0 };
    }

    fn update_decay_increment(&mut self) {
        let samples = self.decay_ms * self.sample_rate / 1000.0;
        let decay_range = 1.0 - self.sustain;
        self.decay_inc = if samples > 0.0 {
            decay_range / samples
        } else {
            decay_range
        };
    }

    fn update_release_increment(&mut self) {
        let samples = self.release_ms * self.sample_rate / 1000.0;
        let release_from = if self.stage == AdsrStage::Release {
            self.value
        } else {
            self.sustain
        };
        self.release_inc = if samples > 0.0 {
            release_from / samples
        } else {
            release_from
        };
    }
}

impl Default for Adsr {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_reaches_unity() {
        let mut env = Adsr::new(44100.0);
        env.set_attack_ms(10.0);
        env.gate_on();
        assert_eq!(env.stage(), AdsrStage::Attack);
        for _ in 0..500 {
            env.process();
        }
        assert!(env.current() > 0.99, "expected ~1.0, got {}", env.current());
    }

    #[test]
    fn decay_settles_at_sustain() {
        let mut env = Adsr::new(44100.0);
        env.set_attack_ms(1.0);
        env.set_decay_ms(10.0);
        env.set_sustain(0.5);
        env.gate_on();
        for _ in 0..100 {
            env.process();
        }
        assert!(env.current() > 0.9);
        for _ in 0..1000 {
            env.process();
        }
        assert!((env.current() - 0.5).abs() < 0.01, "got {}", env.current());
        assert_eq!(env.stage(), AdsrStage::Sustain);
    }

    #[test]
    fn release_returns_to_zero() {
        let mut env = Adsr::new(44100.0);
        env.set_attack_ms(1.0);
        env.set_decay_ms(1.0);
        env.set_sustain(0.5);
        env.set_release_ms(20.0);
        env.gate_on();
        for _ in 0..200 {
            env.process();
        }
        env.gate_off();
        assert_eq!(env.stage(), AdsrStage::Release);
        for _ in 0..2000 {
            env.process();
        }
        assert!(env.current() < 0.01, "got {}", env.current());
        assert_eq!(env.stage(), AdsrStage::Idle);
    }

    #[test]
    fn is_active_tracks_stage() {
        let mut env = Adsr::new(44100.0);
        assert!(!env.is_active());
        env.gate_on();
        assert!(env.is_active());
        env.reset();
        assert!(!env.is_active());
    }

    #[test]
    fn retrigger_does_not_jump() {
        let mut env = Adsr::new(44100.0);
        env.set_attack_ms(10.0);
        env.gate_on();
        for _ in 0..100 {
            env.process();
        }
        let before = env.current();
        env.gate_on();
        let after = env.current();
        assert_eq!(before, after);
    }
}
