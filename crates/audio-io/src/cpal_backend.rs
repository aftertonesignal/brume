// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! cpal-based audio backend implementation.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use crate::backend::{
    AudioBackend, AudioCallback, AudioError, AudioOutputConfig, AudioStream, OutputDeviceInfo,
};

/// Audio backend using cpal (cross-platform: `CoreAudio`, ALSA, WASAPI).
pub struct CpalBackend;

impl CpalBackend {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps an ALSA card id (or cpal device name) to a friendly UI label.
///
/// On Linux we parse `/proc/asound/cards` and get bare card ids
/// (e.g. `UAC2Gadget`, `vc4hdmi0`). On macOS we get cpal's CoreAudio
/// device name. Either way: substring-match known identifiers first,
/// pass through unknowns.
#[must_use]
pub fn friendly_label(name: &str) -> String {
    if name.contains("UAC2Gadget") {
        // Drop the "USB-C" prefix — Meridian works over any
        // data-capable USB port the carrier breaks out (USB_OTG on
        // the Waveshare CM5 IO board, the standard USB-A on the
        // CM4 IO board, etc.). The connector type is incidental.
        "Meridian (USB to DAW)".into()
    } else if name.contains("vc4hdmi0") || name.contains("vc4-hdmi-0") {
        "HDMI 0 (touchscreen)".into()
    } else if name.contains("vc4hdmi1") || name.contains("vc4-hdmi-1") {
        "HDMI 1".into()
    } else {
        name.into()
    }
}

/// Parses `/proc/asound/cards` and returns `(card_number, card_id)` pairs.
///
/// Each card occupies two lines; only the first has the identifier in
/// brackets. Format (from kernel):
///
/// ```text
///  0 [vc4hdmi0       ]: vc4-hdmi - vc4-hdmi-0
///                       vc4-hdmi-0
/// ```
///
/// We're strict about the ` N [id]:` prefix and ignore everything else.
#[cfg(target_os = "linux")]
fn parse_proc_cards(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let digit_len = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digit_len == 0 {
            continue;
        }
        let Ok(num) = trimmed[..digit_len].parse::<u32>() else {
            continue;
        };
        let after_num = trimmed[digit_len..].trim_start();
        if !after_num.starts_with('[') {
            continue;
        }
        let Some(close) = after_num.find(']') else {
            continue;
        };
        let id = after_num[1..close].trim().to_string();
        if id.is_empty() {
            continue;
        }
        out.push((num, id));
    }
    out
}

impl AudioBackend for CpalBackend {
    fn default_sample_rate(&self) -> Result<f32, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let config = device
            .default_output_config()
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        Ok(config.sample_rate().0 as f32)
    }

    #[cfg(target_os = "linux")]
    fn list_output_devices(&self) -> Result<Vec<OutputDeviceInfo>, AudioError> {
        // On Linux we parse /proc/asound/cards directly instead of relying on
        // cpal's output_devices() enumeration. Two reasons:
        //   (a) cpal's ALSA backend silently omits cards that are currently
        //       held exclusive by another process — including our own audio
        //       stream when it opens via "default" → UAC2Gadget. This would
        //       make Meridian disappear from its own selector.
        //   (b) cpal returns the same card under multiple PCM names
        //       (hw:CARD=X, plughw:CARD=X, sysdefault:CARD=X, etc.).
        //       Collapsing to one row per card is what we actually want.
        //
        // /proc/asound/cards is the authoritative card list and doesn't
        // require probing. We filter to cards that expose a playback PCM
        // (`/proc/asound/cardN/pcm0p/` exists — MIDI-only devices like
        // nanoKONTROL2 won't have that directory).
        let cards_text = std::fs::read_to_string("/proc/asound/cards")
            .map_err(|e| AudioError::Stream(format!("/proc/asound/cards: {e}")))?;

        let cards = parse_proc_cards(&cards_text);
        let mut out: Vec<OutputDeviceInfo> = Vec::new();
        for (num, id) in cards {
            let pcm_p = format!("/proc/asound/card{num}/pcm0p");
            if !std::path::Path::new(&pcm_p).exists() {
                continue;
            }
            // `plughw:CARD=<id>,DEV=0` is the canonical open path — the
            // `plug` plugin wraps hardware-specific conversion, so the
            // engine's 48 kHz f32 stereo works on cards that natively
            // want e.g. S16_LE 44.1 kHz.
            let device_id = format!("plughw:CARD={id},DEV=0");
            out.push(OutputDeviceInfo {
                label: friendly_label(&id),
                is_default: false, // Linux reports "active" separately from kernel default
                id: device_id,
            });
        }

        if out.is_empty() {
            return Err(AudioError::NoDevice);
        }
        Ok(out)
    }

    #[cfg(not(target_os = "linux"))]
    fn list_output_devices(&self) -> Result<Vec<OutputDeviceInfo>, AudioError> {
        // macOS / other: cpal's enumeration is fine (CoreAudio doesn't have
        // the exclusive-device-hides-itself issue).
        let host = cpal::default_host();
        let default_name = host.default_output_device().and_then(|d| d.name().ok());

        let devices = host
            .output_devices()
            .map_err(|e| AudioError::Stream(e.to_string()))?;

        let mut out: Vec<OutputDeviceInfo> = Vec::new();
        for device in devices {
            let Ok(name) = device.name() else { continue };
            let is_default = default_name.as_deref() == Some(name.as_str());
            out.push(OutputDeviceInfo {
                label: friendly_label(&name),
                is_default,
                id: name,
            });
        }

        if out.is_empty() {
            return Err(AudioError::NoDevice);
        }
        Ok(out)
    }

    fn open_output(
        &self,
        config: AudioOutputConfig,
        mut callback: AudioCallback,
    ) -> Result<Box<dyn AudioStream>, AudioError> {
        let host = cpal::default_host();

        // Pick the requested device if supplied; otherwise fall back to
        // the host default. An unmatched `device` id is logged and we
        // still fall back — callers persist IDs that may not exist today
        // (DAC hat unplugged, USB iface disconnected).
        let device = if let Some(ref wanted) = config.device {
            let mut found = None;
            if let Ok(mut iter) = host.output_devices() {
                found = iter.find(|d| d.name().ok().as_deref() == Some(wanted.as_str()));
            }
            if found.is_none() {
                eprintln!(
                    "brume audio: requested device {wanted:?} not found; falling back to default"
                );
            }
            found
                .or_else(|| host.default_output_device())
                .ok_or(AudioError::NoDevice)?
        } else {
            host.default_output_device().ok_or(AudioError::NoDevice)?
        };

        let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());

        // Try the device's default config first — most reliable path
        let stream_config = if let Ok(default_cfg) = device.default_output_config() {
            let mut cfg: StreamConfig = default_cfg.into();
            // Override sample rate if requested
            if let Some(sr) = config.sample_rate {
                cfg.sample_rate = cpal::SampleRate(sr as u32);
            }
            // Override channel count. The caller's explicit request wins over
            // whatever the device's default happens to be — plughw wrappers on
            // Linux in particular often default to 2 channels even when the
            // underlying UAC2 endpoint advertises more. Setting this lets the
            // Meridian 8-ch stems path actually pick up all 8.
            if config.channels > 0 {
                cfg.channels = config.channels;
            }
            if let Some(buf_size) = config.buffer_size {
                cfg.buffer_size = cpal::BufferSize::Fixed(buf_size);
            }
            cfg
        } else {
            // Fall back to searching supported configs for f32 stereo
            let supported = device
                .supported_output_configs()
                .map_err(|e| AudioError::Stream(e.to_string()))?
                .find(|c| c.sample_format() == SampleFormat::F32 && c.channels() == config.channels)
                .ok_or_else(|| {
                    AudioError::UnsupportedConfig(format!(
                        "no compatible config on device {device_name:?}"
                    ))
                })?;

            let sample_rate = config.sample_rate.map_or_else(
                || supported.min_sample_rate(),
                |sr| cpal::SampleRate(sr as u32),
            );
            let sample_rate = sample_rate
                .0
                .clamp(supported.min_sample_rate().0, supported.max_sample_rate().0);

            let mut cfg: StreamConfig = supported
                .with_sample_rate(cpal::SampleRate(sample_rate))
                .into();

            if let Some(buf_size) = config.buffer_size {
                cfg.buffer_size = cpal::BufferSize::Fixed(buf_size);
            }
            cfg
        };

        let actual_sample_rate = stream_config.sample_rate.0 as f32;
        let actual_channels = stream_config.channels;

        eprintln!(
            "brume: opening audio on {device_name:?} — {actual_sample_rate} Hz, {actual_channels} ch"
        );

        let hw_channels = actual_channels as usize;

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // The engine receives the actual buffer and channel
                    // count; it handles per-channel layout itself (stereo
                    // master mix at 2ch; per-part stems at 8ch; safe
                    // fallback with zero-fill for exotic counts).
                    callback(data, hw_channels);
                },
                |err| {
                    eprintln!("brume audio stream error: {err}");
                },
                None,
            )
            .map_err(|e| AudioError::Stream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioError::Stream(e.to_string()))?;

        Ok(Box::new(CpalStream {
            _stream: stream,
            sample_rate: actual_sample_rate,
        }))
    }
}

/// A running cpal output stream. Audio stops when dropped.
struct CpalStream {
    _stream: cpal::Stream,
    sample_rate: f32,
}

impl AudioStream for CpalStream {
    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_label_maps_known_devices() {
        // Bare card ids (Linux /proc/asound/cards path):
        assert_eq!(friendly_label("UAC2Gadget"), "Meridian (USB to DAW)");
        assert_eq!(friendly_label("vc4hdmi0"), "HDMI 0 (touchscreen)");
        assert_eq!(friendly_label("vc4hdmi1"), "HDMI 1");
        // Full cpal names also match via substring (macOS path or
        // unexpected Linux input):
        assert_eq!(
            friendly_label("plughw:CARD=UAC2Gadget,DEV=0"),
            "Meridian (USB to DAW)"
        );
        assert_eq!(
            friendly_label("hw:CARD=vc4hdmi0,DEV=0"),
            "HDMI 0 (touchscreen)"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_cards_extracts_ids() {
        let sample = " 0 [vc4hdmi0       ]: vc4-hdmi - vc4-hdmi-0\n                      vc4-hdmi-0\n 1 [vc4hdmi1       ]: vc4-hdmi - vc4-hdmi-1\n                      vc4-hdmi-1\n 2 [UAC2Gadget     ]: UAC2_Gadget - UAC2_Gadget\n                      UAC2_Gadget 0\n 3 [nanoKONTROL2   ]: USB-Audio - nanoKONTROL2\n";
        let cards = parse_proc_cards(sample);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0], (0, "vc4hdmi0".to_string()));
        assert_eq!(cards[1], (1, "vc4hdmi1".to_string()));
        assert_eq!(cards[2], (2, "UAC2Gadget".to_string()));
        assert_eq!(cards[3], (3, "nanoKONTROL2".to_string()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_cards_ignores_garbage_lines() {
        let sample = "random garbage line\n 0 [goodcard       ]: driver - name\n   continuation\n--- separator ---\n";
        let cards = parse_proc_cards(sample);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].1, "goodcard");
    }

    #[test]
    fn friendly_label_passes_unknown_through() {
        assert_eq!(friendly_label("unknown-device"), "unknown-device");
        assert_eq!(friendly_label(""), "");
    }

    #[test]
    fn list_output_devices_returns_at_least_default() {
        // Best-effort test: on a dev host without any audio device (CI
        // sometimes), the backend returns NoDevice — that's still a valid
        // outcome for the API shape, just skip the length assertion.
        let backend = CpalBackend::new();
        if let Ok(list) = backend.list_output_devices() {
            assert!(!list.is_empty());
            // Exactly one default under normal conditions. Test is lenient
            // in case the host reports none, but it must never report more
            // than one.
            let defaults = list.iter().filter(|d| d.is_default).count();
            assert!(
                defaults <= 1,
                "more than one default device reported: {list:?}"
            );
        }
    }

    #[test]
    fn output_config_default_has_no_device() {
        let cfg = AudioOutputConfig::default();
        assert_eq!(cfg.device, None);
        assert_eq!(cfg.channels, 2);
    }
}
