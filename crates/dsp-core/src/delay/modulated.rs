// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Modulated delay line for chorus / flanger effects.

use std::f32::consts::TAU;

/// A delay line with a built-in sine LFO modulating the delay time,
/// producing the pitch-shifting characteristic of chorus and flanger.
///
/// # Example
///
/// ```
/// use brume_dsp_core::delay::ModulatedDelay;
///
/// let mut delay = ModulatedDelay::new(4410, 44100.0);
/// delay.set_base_delay_ms(10.0);
/// delay.set_rate_hz(0.5);
/// delay.set_depth_ms(3.0);
/// let output = delay.process(1.0);
/// ```
pub struct ModulatedDelay {
    buffer: Vec<f32>,
    write_pos: usize,
    sample_rate: f32,
    base_delay_samples: f32,
    depth_samples: f32,
    lfo_phase: f32,
    lfo_rate: f32,
    feedback: f32,
    last_output: f32,
}

impl ModulatedDelay {
    /// Creates a new modulated delay.
    #[must_use]
    pub fn new(max_samples: usize, sample_rate: f32) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_pos: 0,
            sample_rate,
            base_delay_samples: sample_rate * 0.01,
            depth_samples: sample_rate * 0.002,
            lfo_phase: 0.0,
            lfo_rate: 0.5,
            feedback: 0.0,
            last_output: 0.0,
        }
    }

    /// Sets the sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// Sets the base delay time in milliseconds. Chorus: 10–30ms.
    /// Flanger: 1–10ms.
    pub fn set_base_delay_ms(&mut self, ms: f32) {
        let samples = ms * self.sample_rate / 1000.0;
        let max = (self.buffer.len() - 1) as f32;
        self.base_delay_samples = samples.clamp(1.0, max);
    }

    /// Sets the modulation depth in milliseconds — the delay sweeps
    /// ±this amount around the base.
    pub fn set_depth_ms(&mut self, ms: f32) {
        self.depth_samples = ms * self.sample_rate / 1000.0;
    }

    /// Sets the LFO rate in Hz. Typical 0.1–5 Hz.
    pub fn set_rate_hz(&mut self, hz: f32) {
        self.lfo_rate = hz.max(0.001);
    }

    /// Sets the feedback amount (0.0–0.99). Values near 1.0 can
    /// self-oscillate.
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.99);
    }

    /// Resets the delay buffer and LFO phase.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.lfo_phase = 0.0;
        self.last_output = 0.0;
    }

    fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let delay_int = delay_samples as usize;
        let delay_frac = delay_samples - delay_int as f32;

        let len = self.buffer.len();
        let read_pos_0 = (self.write_pos + len - delay_int) % len;
        let read_pos_1 = (read_pos_0 + len - 1) % len;

        let sample_0 = self.buffer[read_pos_0];
        let sample_1 = self.buffer[read_pos_1];
        sample_0 + delay_frac * (sample_1 - sample_0)
    }

    /// Processes a single sample.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let lfo_value = (self.lfo_phase * TAU).sin();
        let modulated_delay = self.base_delay_samples + lfo_value * self.depth_samples;
        let clamped_delay = modulated_delay.clamp(1.0, (self.buffer.len() - 1) as f32);

        let delayed = self.read_interpolated(clamped_delay);

        let write_sample = input + self.feedback * self.last_output;
        self.buffer[self.write_pos] = write_sample;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        self.lfo_phase += self.lfo_rate / self.sample_rate;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }

        self.last_output = delayed;
        delayed
    }

    /// Processes a sample and returns both wet and dry signals.
    #[inline]
    pub fn process_wet_dry(&mut self, input: f32) -> (f32, f32) {
        let wet = self.process(input);
        (wet, input)
    }
}

impl Default for ModulatedDelay {
    fn default() -> Self {
        Self::new(4410, 44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulation_varies_delay() {
        let mut delay = ModulatedDelay::new(4410, 44100.0);
        delay.set_base_delay_ms(10.0);
        delay.set_depth_ms(5.0);
        delay.set_rate_hz(10.0);

        delay.process(1.0);

        let samples_per_cycle = (44100.0 / 10.0) as usize;
        let mut outputs = Vec::with_capacity(samples_per_cycle);
        for _ in 0..samples_per_cycle {
            outputs.push(delay.process(0.0));
        }

        let max_out = outputs.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        let min_out = outputs.iter().fold(1.0_f32, |a, &b| a.min(b.abs()));
        assert!(max_out > min_out, "modulation should vary the delay time");
    }

    #[test]
    fn reset_clears_state() {
        let mut delay = ModulatedDelay::new(1000, 44100.0);
        for _ in 0..100 {
            delay.process(1.0);
        }
        delay.reset();
        assert!(delay.buffer.iter().all(|&x| x == 0.0));
        assert_eq!(delay.lfo_phase, 0.0);
    }

    #[test]
    fn feedback_extends_tail() {
        let mut no_fb = ModulatedDelay::new(1000, 44100.0);
        let mut with_fb = ModulatedDelay::new(1000, 44100.0);

        no_fb.set_base_delay_ms(10.0);
        no_fb.set_depth_ms(0.0);
        no_fb.set_feedback(0.0);

        with_fb.set_base_delay_ms(10.0);
        with_fb.set_depth_ms(0.0);
        with_fb.set_feedback(0.8);

        no_fb.process(1.0);
        with_fb.process(1.0);

        let mut count_no_fb = 0;
        let mut count_with_fb = 0;
        for _ in 0..1000 {
            if no_fb.process(0.0).abs() > 0.01 {
                count_no_fb += 1;
            }
            if with_fb.process(0.0).abs() > 0.01 {
                count_with_fb += 1;
            }
        }

        assert!(
            count_with_fb > count_no_fb,
            "feedback should extend the tail"
        );
    }
}
