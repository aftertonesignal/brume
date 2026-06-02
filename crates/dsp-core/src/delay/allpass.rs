// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Allpass delay for reverb diffusion.

/// Schroeder allpass: output magnitude equals input magnitude at all
/// frequencies, but phase is shifted — creates the "smearing" /
/// diffusion that smooths reverb tails.
///
/// # Example
///
/// ```
/// use brume_dsp_core::delay::AllpassDelay;
///
/// let mut allpass = AllpassDelay::new(2048);
/// allpass.set_delay_samples(441.0); // ~10ms at 44.1kHz
/// allpass.set_gain(0.5);
/// let output = allpass.process(1.0);
/// ```
pub struct AllpassDelay {
    buffer: Vec<f32>,
    write_pos: usize,
    delay_samples: f32,
    gain: f32,
}

impl AllpassDelay {
    /// Creates a new allpass delay with the specified maximum delay in samples.
    #[must_use]
    pub fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_pos: 0,
            delay_samples: 1.0,
            gain: 0.5,
        }
    }

    /// Sets the delay time in samples (supports fractional values).
    pub fn set_delay_samples(&mut self, samples: f32) {
        self.delay_samples = samples.max(1.0).min((self.buffer.len() - 1) as f32);
    }

    /// Sets the delay time in seconds at the given sample rate.
    pub fn set_delay_time(&mut self, seconds: f32, sample_rate: f32) {
        self.set_delay_samples(seconds * sample_rate);
    }

    /// Sets the feedback/feedforward gain coefficient. Typical 0.3–0.7;
    /// higher values create more diffusion but can resonate.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.clamp(-0.99, 0.99);
    }

    /// Returns the current gain coefficient.
    #[must_use]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Resets the delay buffer to silence.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    /// Reads from the buffer with linear interpolation.
    fn read_interpolated(&self) -> f32 {
        let delay_int = self.delay_samples as usize;
        let delay_frac = self.delay_samples - delay_int as f32;

        let len = self.buffer.len();
        let read_pos_0 = (self.write_pos + len - delay_int) % len;
        let read_pos_1 = (read_pos_0 + len - 1) % len;

        let sample_0 = self.buffer[read_pos_0];
        let sample_1 = self.buffer[read_pos_1];
        sample_0 + delay_frac * (sample_1 - sample_0)
    }

    /// Processes a single sample through the allpass filter. Schroeder
    /// structure: `output = -gain * input + delayed`,
    /// `feedback = input + gain * delayed`.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let delayed = self.read_interpolated();

        let output = -self.gain * input + delayed;
        let feedback = input + self.gain * delayed;

        self.buffer[self.write_pos] = feedback;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        output
    }
}

impl Default for AllpassDelay {
    fn default() -> Self {
        Self::new(4410) // ~100ms at 44.1kHz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allpass_unity_gain_on_ac() {
        use std::f32::consts::PI;

        let mut allpass = AllpassDelay::new(1000);
        allpass.set_delay_samples(100.0);
        allpass.set_gain(0.5);

        let freq = 1000.0;
        let sample_rate = 44100.0;

        for i in 0..500 {
            let phase = 2.0 * PI * freq * (i as f32) / sample_rate;
            allpass.process(phase.sin());
        }

        let mut sum_input_sq = 0.0_f32;
        let mut sum_output_sq = 0.0_f32;
        for i in 0..1000 {
            let phase = 2.0 * PI * freq * ((500 + i) as f32) / sample_rate;
            let input = phase.sin();
            let output = allpass.process(input);
            sum_input_sq += input * input;
            sum_output_sq += output * output;
        }

        let rms_in = (sum_input_sq / 1000.0).sqrt();
        let rms_out = (sum_output_sq / 1000.0).sqrt();
        let ratio = rms_out / rms_in;
        assert!(
            (ratio - 1.0).abs() < 0.2,
            "allpass should have ~unity gain; got {ratio}"
        );
    }

    #[test]
    fn reset_clears_buffer() {
        let mut allpass = AllpassDelay::new(100);
        for _ in 0..50 {
            allpass.process(1.0);
        }
        allpass.reset();
        assert!(allpass.buffer.iter().all(|&x| x == 0.0));
    }
}
