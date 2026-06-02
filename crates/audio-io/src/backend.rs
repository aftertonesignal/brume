// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Audio backend trait definitions.

/// Error type for audio operations.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no output device available")]
    NoDevice,

    #[error("unsupported audio configuration: {0}")]
    UnsupportedConfig(String),

    #[error("stream error: {0}")]
    Stream(String),
}

/// Configuration for opening an audio output stream.
pub struct AudioOutputConfig {
    /// Desired sample rate in Hz. `None` = use device default.
    pub sample_rate: Option<f32>,
    /// Desired buffer size in frames. `None` = use device default.
    pub buffer_size: Option<u32>,
    /// Number of output channels (2 for stereo).
    pub channels: u16,
    /// Which device to open. `None` = default output device. `Some(id)` =
    /// match cpal `Device::name()` equal to `id`, falling back to default
    /// with a warning log if the named device isn't present.
    pub device: Option<String>,
}

impl Default for AudioOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: None,
            buffer_size: None,
            channels: 2,
            device: None,
        }
    }
}

/// One selectable audio output device (ALSA PCM on Linux, Core Audio device
/// on macOS, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDeviceInfo {
    /// Stable identifier — cpal's `Device::name()` verbatim. Persist this
    /// in settings; match against it when re-opening.
    pub id: String,
    /// Friendly display form for the UI (e.g. "Meridian (USB to DAW)").
    pub label: String,
    /// Whether this device is the host's current default output.
    pub is_default: bool,
}

/// The audio callback signature: receives an interleaved `&mut [f32]`
/// buffer plus the stream's channel count. The callback must fill the
/// buffer assuming that channel layout (e.g. `[L0, R0, L1, R1, …]` for
/// 2 channels, per-part-pair for 8 channels on the Meridian gadget).
/// Runs on the audio thread — must not block or allocate.
pub type AudioCallback = Box<dyn FnMut(&mut [f32], usize) + Send + 'static>;

/// A running audio output stream.
///
/// Audio stops when this is dropped. The stream handle is not `Send`
/// because some platform backends (e.g. `CoreAudio`) bind streams to
/// the creating thread.
pub trait AudioStream {
    /// The actual sample rate of the opened stream.
    fn sample_rate(&self) -> f32;
}

/// Backend for opening audio output streams.
pub trait AudioBackend {
    /// Returns the default output device's sample rate without opening a stream.
    ///
    /// Use this to create the engine at the correct rate before opening audio.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NoDevice`] if no output device is available.
    fn default_sample_rate(&self) -> Result<f32, AudioError>;

    /// Enumerates available output devices. Exactly one entry should have
    /// `is_default = true` under normal conditions.
    ///
    /// The listing is taken at call time; device presence can change
    /// (USB audio interfaces, DAC hats) so this is not cached. Callers
    /// re-enumerate when the user opens the SYS selector, not every frame.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NoDevice`] if the host reports no devices,
    /// or [`AudioError::Stream`] on a host-level failure.
    fn list_output_devices(&self) -> Result<Vec<OutputDeviceInfo>, AudioError>;

    /// Opens a stereo output stream.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NoDevice`] if no output device is available,
    /// [`AudioError::UnsupportedConfig`] if the requested format is not
    /// supported, or [`AudioError::Stream`] on a runtime stream error.
    fn open_output(
        &self,
        config: AudioOutputConfig,
        callback: AudioCallback,
    ) -> Result<Box<dyn AudioStream>, AudioError>;
}
