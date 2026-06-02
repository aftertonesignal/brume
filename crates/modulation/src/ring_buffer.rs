// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Fixed-size ring buffer for modulation history tracking.
//!
//! Ported from Formfactor. Const-generic, no heap allocation,
//! real-time safe. Stores the N most recent `f32` values.

/// A fixed-size ring buffer that stores the most recent N values.
///
/// All storage is inline — no heap allocations. Suitable for
/// real-time use in the audio thread.
pub struct RingBuffer<const N: usize> {
    data: [f32; N],
    write_pos: usize,
    count: usize,
}

impl<const N: usize> Default for RingBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RingBuffer<N> {
    /// Creates an empty ring buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            data: [0.0; N],
            write_pos: 0,
            count: 0,
        }
    }

    /// Pushes a value, overwriting the oldest if full.
    pub fn push(&mut self, value: f32) {
        self.data[self.write_pos] = value;
        self.write_pos = (self.write_pos + 1) % N;
        if self.count < N {
            self.count += 1;
        }
    }

    /// Number of values currently stored.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.count == N
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Returns the most recently pushed value.
    #[must_use]
    pub fn newest(&self) -> Option<f32> {
        if self.count == 0 {
            return None;
        }
        let idx = if self.write_pos == 0 {
            N - 1
        } else {
            self.write_pos - 1
        };
        Some(self.data[idx])
    }

    /// Returns the oldest value in the buffer.
    #[must_use]
    pub fn oldest(&self) -> Option<f32> {
        if self.count == 0 {
            return None;
        }
        if self.count < N {
            Some(self.data[0])
        } else {
            Some(self.data[self.write_pos])
        }
    }

    /// Sum of all values in the buffer.
    #[must_use]
    pub fn sum(&self) -> f32 {
        if self.count < N {
            self.data[..self.count].iter().sum()
        } else {
            self.data.iter().sum()
        }
    }

    /// Average of all values in the buffer.
    #[must_use]
    pub fn average(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        self.sum() / self.count as f32
    }

    /// Variance of values in the buffer.
    #[must_use]
    pub fn variance(&self) -> f32 {
        if self.count < 2 {
            return 0.0;
        }
        let mean = self.average();
        let sum_sq: f32 = self.iter().map(|v| (v - mean) * (v - mean)).sum();
        sum_sq / self.count as f32
    }

    /// Clears all values.
    pub fn clear(&mut self) {
        self.data = [0.0; N];
        self.write_pos = 0;
        self.count = 0;
    }

    /// Iterates from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        let start = if self.count < N { 0 } else { self.write_pos };
        (0..self.count).map(move |i| self.data[(start + i) % N])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_read() {
        let mut buf = RingBuffer::<4>::new();
        assert!(buf.is_empty());

        buf.push(1.0);
        buf.push(2.0);
        buf.push(3.0);
        assert_eq!(buf.len(), 3);
        assert!((buf.newest().unwrap() - 3.0).abs() < f32::EPSILON);
        assert!((buf.oldest().unwrap() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn overflow_wraps() {
        let mut buf = RingBuffer::<3>::new();
        buf.push(1.0);
        buf.push(2.0);
        buf.push(3.0);
        buf.push(4.0); // overwrites 1.0

        assert!(buf.is_full());
        assert!((buf.oldest().unwrap() - 2.0).abs() < f32::EPSILON);
        assert!((buf.newest().unwrap() - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn average_and_variance() {
        let mut buf = RingBuffer::<4>::new();
        buf.push(2.0);
        buf.push(4.0);
        buf.push(6.0);
        buf.push(8.0);

        assert!((buf.average() - 5.0).abs() < f32::EPSILON);
        assert!((buf.variance() - 5.0).abs() < 0.01);
    }

    #[test]
    fn iter_order() {
        let mut buf = RingBuffer::<3>::new();
        buf.push(10.0);
        buf.push(20.0);
        buf.push(30.0);
        buf.push(40.0); // wraps: [40, 20, 30] with oldest=20

        let vals: Vec<f32> = buf.iter().collect();
        assert_eq!(vals, vec![20.0, 30.0, 40.0]);
    }
}
