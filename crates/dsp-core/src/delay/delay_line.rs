// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
//
// Originally authored in the private `aftertone-dsp-kit` library and
// vendored here for Brume's GPL-licensed distribution with the
// copyright holder's authorization. See LICENSE for full terms.

//! Basic delay line with linear interpolation for fractional delays.

/// A delay line with linear interpolation for fractional delays.
///
/// # Example
///
/// ```
/// use brume_dsp_core::delay::DelayLine;
///
/// let mut delay = DelayLine::new(44100); // 1 second max delay
/// delay.set_delay_samples(22050.0);      // 500ms delay
/// let output = delay.process(1.0);
/// ```
pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    delay_samples: f32,
}

impl DelayLine {
    /// Creates a new delay line with the specified maximum capacity in samples.
    #[must_use]
    pub fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_pos: 0,
            delay_samples: 1.0,
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

    /// Resets the delay buffer to silence.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    /// Processes a sample through the delay line. Writes the input to
    /// the buffer and returns the delayed output with linear
    /// interpolation for fractional delays.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.buffer[self.write_pos] = input;

        let delay_int = self.delay_samples as usize;
        let delay_frac = self.delay_samples - delay_int as f32;

        let len = self.buffer.len();
        let read_pos_0 = (self.write_pos + len - delay_int) % len;
        let read_pos_1 = (read_pos_0 + len - 1) % len;

        let sample_0 = self.buffer[read_pos_0];
        let sample_1 = self.buffer[read_pos_1];
        let output = sample_0 + delay_frac * (sample_1 - sample_0);

        self.write_pos = (self.write_pos + 1) % len;

        output
    }

    /// Reads from the delay line at a specific delay time without writing.
    #[must_use]
    pub fn read(&self, delay_samples: f32) -> f32 {
        let delay_int = delay_samples as usize;
        let delay_frac = delay_samples - delay_int as f32;

        let len = self.buffer.len();
        let read_pos_0 = (self.write_pos + len - delay_int - 1) % len;
        let read_pos_1 = (read_pos_0 + len - 1) % len;

        let sample_0 = self.buffer[read_pos_0];
        let sample_1 = self.buffer[read_pos_1];
        sample_0 + delay_frac * (sample_1 - sample_0)
    }

    /// Processes a block of samples in-place. Each input sample is
    /// written to the buffer and replaced with the delayed output.
    pub fn process_block(&mut self, samples: &mut [f32]) {
        let delay_int = self.delay_samples as usize;
        let delay_frac = self.delay_samples - delay_int as f32;
        let len = self.buffer.len();

        for sample in samples.iter_mut() {
            let input = *sample;
            self.buffer[self.write_pos] = input;

            let read_pos_0 = (self.write_pos + len - delay_int) % len;
            let read_pos_1 = (read_pos_0 + len - 1) % len;

            let sample_0 = self.buffer[read_pos_0];
            let sample_1 = self.buffer[read_pos_1];
            *sample = sample_0 + delay_frac * (sample_1 - sample_0);

            self.write_pos = (self.write_pos + 1) % len;
        }
    }

    /// Processes a block from input to output buffers.
    pub fn process_block_to(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len());

        let delay_int = self.delay_samples as usize;
        let delay_frac = self.delay_samples - delay_int as f32;
        let len = self.buffer.len();

        for (inp, out) in input.iter().zip(output.iter_mut()) {
            self.buffer[self.write_pos] = *inp;

            let read_pos_0 = (self.write_pos + len - delay_int) % len;
            let read_pos_1 = (read_pos_0 + len - 1) % len;

            let sample_0 = self.buffer[read_pos_0];
            let sample_1 = self.buffer[read_pos_1];
            *out = sample_0 + delay_frac * (sample_1 - sample_0);

            self.write_pos = (self.write_pos + 1) % len;
        }
    }
}

impl Default for DelayLine {
    fn default() -> Self {
        Self::new(44100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_appears_at_set_delay() {
        let mut delay = DelayLine::new(100);
        delay.set_delay_samples(10.0);

        let mut outputs = Vec::new();
        outputs.push(delay.process(1.0));
        for _ in 0..15 {
            outputs.push(delay.process(0.0));
        }

        assert!(
            outputs[10] > 0.5,
            "expected impulse at index 10, got {}",
            outputs[10]
        );
    }

    #[test]
    fn reset_clears_buffer() {
        let mut delay = DelayLine::new(100);
        delay.process(1.0);
        delay.process(1.0);

        delay.reset();
        assert!(delay.buffer.iter().all(|&x| x == 0.0));
    }
}
