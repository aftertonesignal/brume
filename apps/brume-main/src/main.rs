// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Brume — standalone multi-timbral instrument application.
//!
//! 4-part multi-timbral: FM (part 0), Harmonic (part 1), Timbral (part 2),
//! Granular (part 3). Each part has 6 voices with independent parameters
//! and modulation.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;

use brume_app_protocol::{EngineToUi, UiToEngine};
use brume_audio_io::{AudioBackend, AudioOutputConfig, AudioStream, CpalBackend};
use brume_common::{LatencyPreset, ParameterId};
use brume_control_model::ControlMatrix;
use brume_engine_runtime::BrumeEngine;
use brume_midi_io::BrumeMidiInput;

/// Brume's native sample rate. Matches the CM5's vc4-hdmi native rate, the
/// UAC2 Meridian gadget's configured rate, and the DAW convention for
/// Aggregate Devices clocked off a 48 kHz interface (e.g. MOTU 828).
/// Opening audio below this forces ALSA's `plug` layer to resample, which
/// introduces drift against the aggregate clock master.
const SAMPLE_RATE: f32 = 48_000.0;

/// Part index for the Timbral engine. Used by the startup chime.
const PART_TIMBRAL: u8 = 2;

/// Plays a soft chime on startup using Part 2 (Timbral) — warm
/// triangle-based tone with gentle waveshaping. Lifted verbatim from
/// the pre-iced WebKit-era main.rs (commit 21cd7a2~1) when the splash
/// screen was retired alongside the WebKit UI; restored here as part
/// of the splash-restoration arc.
///
/// Two-phrase welcome melody:
///   * Phrase 1: ascending Cmaj7 arpeggio (C-E-G-B-C), 1.2 s ring.
///   * Phrase 2: resolving Fmaj9 (F-A-C-E), 1.5 s ring.
///
/// Total wall-clock ≈ 5.5 s including the initial 100 ms parameter
/// settling and the post-tail defaults restore. The splash widget's
/// fade-out timing is tuned to dismiss around 4 s so the visual gets
/// out of the way while phrase 2 is still ringing.
fn startup_chime(tx: &crossbeam_channel::Sender<UiToEngine>) {
    let set = |part: u8, id: ParameterId, val: f32| {
        tx.send(UiToEngine::SetParameter {
            part,
            id,
            value: val,
        })
        .ok();
    };

    thread::sleep(Duration::from_millis(300));

    // Configure Part 2 (Timbral) for the chime
    set(PART_TIMBRAL, ParameterId::Timbre, 0.25);
    set(PART_TIMBRAL, ParameterId::Symmetry, 0.1);
    set(PART_TIMBRAL, ParameterId::AmpAttack, 15.0);
    set(PART_TIMBRAL, ParameterId::AmpDecay, 2000.0);
    set(PART_TIMBRAL, ParameterId::AmpSustain, 0.0);
    set(PART_TIMBRAL, ParameterId::AmpRelease, 1500.0);
    set(PART_TIMBRAL, ParameterId::FilterCutoff, 6000.0);
    set(PART_TIMBRAL, ParameterId::FilterResonance, 0.05);
    set(PART_TIMBRAL, ParameterId::FilterEnvDepth, 0.3);
    set(PART_TIMBRAL, ParameterId::FilterEnvAttack, 10.0);
    set(PART_TIMBRAL, ParameterId::FilterEnvDecay, 1500.0);
    set(PART_TIMBRAL, ParameterId::FilterEnvSustain, 0.0);
    set(PART_TIMBRAL, ParameterId::FilterEnvRelease, 1000.0);

    thread::sleep(Duration::from_millis(100));

    // Two-phrase welcome melody on Part 2 (Timbral)
    let note_on = |n: u8, v: f32| {
        tx.send(UiToEngine::NoteOn {
            part: PART_TIMBRAL,
            note: n,
            velocity: v,
        })
        .ok();
    };
    let note_off = |n: u8| {
        tx.send(UiToEngine::NoteOff {
            part: PART_TIMBRAL,
            note: n,
        })
        .ok();
    };

    // Phrase 1: ascending Cmaj7 arpeggio
    note_on(60, 0.30); // C4
    thread::sleep(Duration::from_millis(120));
    note_on(64, 0.28); // E4
    thread::sleep(Duration::from_millis(120));
    note_on(67, 0.25); // G4
    thread::sleep(Duration::from_millis(120));
    note_on(71, 0.22); // B4
    thread::sleep(Duration::from_millis(120));
    note_on(72, 0.18); // C5

    // Let phrase 1 ring
    thread::sleep(Duration::from_millis(1200));

    // Release phrase 1
    for &n in &[60, 64, 67, 71, 72] {
        note_off(n);
    }
    thread::sleep(Duration::from_millis(200));

    // Phrase 2: resolving Fmaj9
    note_on(65, 0.25); // F4
    thread::sleep(Duration::from_millis(100));
    note_on(69, 0.22); // A4
    thread::sleep(Duration::from_millis(100));
    note_on(72, 0.20); // C5
    thread::sleep(Duration::from_millis(100));
    note_on(76, 0.15); // E5

    thread::sleep(Duration::from_millis(1500));

    for &n in &[65, 69, 72, 76] {
        note_off(n);
    }

    // Wait for tails, then restore Part 2 defaults so the user's first
    // touch on the keyboard plays with the engine's normal envelope
    // shape, not the long-attack chime envelope.
    thread::sleep(Duration::from_millis(1500));
    set(PART_TIMBRAL, ParameterId::AmpAttack, 5.0);
    set(PART_TIMBRAL, ParameterId::AmpDecay, 200.0);
    set(PART_TIMBRAL, ParameterId::AmpSustain, 0.7);
    set(PART_TIMBRAL, ParameterId::AmpRelease, 300.0);
}

/// Opens a cpal output stream whose callback drains `rx` into the shared
/// engine and renders the block. Used both for the initial startup stream
/// and for rebuilding the stream when the user switches output device.
///
/// The callback takes `engine.try_lock()` each buffer. Under normal
/// operation the main thread never holds the lock, so this is effectively
/// lock-free. During a device switch the main thread briefly holds the
/// lock; contended buffers zero-fill (brief silence, much shorter than
/// the underlying stream teardown/reopen already causes).
fn open_audio_stream(
    backend: &CpalBackend,
    engine: Arc<Mutex<BrumeEngine>>,
    rx: crossbeam_channel::Receiver<UiToEngine>,
    device_id: Option<String>,
    latency: LatencyPreset,
) -> Result<Box<dyn AudioStream>, brume_audio_io::AudioError> {
    // Meridian (the USB UAC2 gadget) advertises 8 channels for per-part
    // stems — ask cpal for all of them so the engine's 8-ch branch actually
    // fires. Any other device stays stereo; the engine renders the master
    // mix to L/R with master FX applied.
    let is_gadget = device_id
        .as_deref()
        .is_some_and(|d| d.contains("UAC2Gadget"));
    let channels: u16 = if is_gadget { 8 } else { 2 };

    // ALSA buffer size (frames). The UAC2 gadget's device default is a
    // deep buffer (≈5120 / period 1024 ≈ 107 ms) that dominates the
    // keyboard-to-sound latency, so default it to the user's latency
    // preset (Balanced = 512 ≈ 6–7 ms). Non-gadget devices keep their own
    // default. A smaller buffer trades slack (underrun risk) and callback
    // overhead (CPU) for latency; BRUME_AUDIO_PERIOD overrides for
    // headless tuning, and if the device rejects the size we fall back to
    // its default below so audio always opens.
    let env_period = std::env::var("BRUME_AUDIO_PERIOD")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&p| p > 0);
    let requested_period = env_period.or(is_gadget.then(|| latency.frames()));

    // Built per attempt so a fallback retry gets fresh clones of the moved
    // captures (the audio callback owns `engine` / `rx` / its own `Once`).
    let open =
        |buffer_size: Option<u32>| -> Result<Box<dyn AudioStream>, brume_audio_io::AudioError> {
            let config = AudioOutputConfig {
                sample_rate: Some(SAMPLE_RATE),
                device: device_id.clone(),
                channels,
                buffer_size,
                ..AudioOutputConfig::default()
            };
            let engine = engine.clone();
            let rx = rx.clone();
            #[cfg(target_os = "linux")]
            let rt_init = std::sync::Once::new();
            backend.open_output(
                config,
                Box::new(move |buffer: &mut [f32], channels: usize| {
                    // Pin the audio thread to SCHED_FIFO on the first
                    // invocation. Without this the cpal_alsa_out worker
                    // runs at default SCHED_OTHER and gets preempted by
                    // iced's main-thread work during sustained MIDI
                    // playback — produces audible underruns.
                    #[cfg(target_os = "linux")]
                    rt_init.call_once(elevate_audio_thread);

                    if let Some(mut eng) = engine.try_lock() {
                        while let Ok(msg) = rx.try_recv() {
                            eng.handle_message(msg);
                        }
                        eng.process_block(buffer, channels);
                    } else {
                        // Main thread holds the lock — typically during a device
                        // swap. Fill silence; audible as a brief gap.
                        buffer.iter_mut().for_each(|s| *s = 0.0);
                    }
                }),
            )
        };

    match requested_period {
        Some(period) => match open(Some(period)) {
            Ok(stream) => {
                if env_period.is_some() {
                    eprintln!(
                        "brume audio: fixed ALSA buffer {period} frames (BRUME_AUDIO_PERIOD)"
                    );
                } else {
                    eprintln!(
                        "brume audio: fixed ALSA buffer {period} frames (preset {})",
                        latency.label()
                    );
                }
                Ok(stream)
            }
            Err(e) => {
                eprintln!(
                    "brume audio: period {period} rejected ({e}); falling back to device default"
                );
                open(None)
            }
        },
        None => open(None),
    }
}

/// Elevate the calling thread (the cpal audio worker) to SCHED_FIFO
/// at rtprio 80. If the call fails — usually due to RLIMIT_RTPRIO or
/// missing CAP_SYS_NICE — we log and carry on; audio still plays,
/// just more vulnerable to xruns under load.
#[cfg(target_os = "linux")]
fn elevate_audio_thread() {
    use thread_priority::{
        RealtimeThreadSchedulePolicy, ThreadPriority, ThreadPriorityValue, ThreadSchedulePolicy,
        set_thread_priority_and_policy, thread_native_id,
    };
    let prio =
        ThreadPriorityValue::try_from(80u8).expect("80 is in the valid SCHED_FIFO priority range");
    // Skip the convenience `set_current_thread_priority` path — its
    // Crossplatform mapping translates to a SCHED_OTHER nice value
    // and silently "succeeds" without actually switching policy.
    // We need explicit SCHED_FIFO.
    let result = set_thread_priority_and_policy(
        thread_native_id(),
        ThreadPriority::Crossplatform(prio),
        ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo),
    );
    match result {
        Ok(()) => eprintln!("brume audio: cpal worker pinned to SCHED_FIFO rtprio 80"),
        Err(e) => {
            eprintln!("brume audio: failed to set SCHED_FIFO ({e:?}); audio may xrun under load")
        }
    }
}

/// Enumerates current output devices and pushes the list to the UI via
/// the engine→UI channel. Called at startup and after every device
/// switch. Silent on enumeration failure — the SYS page just shows the
/// last-known state until the next poll succeeds.
fn push_output_device_list(
    backend: &CpalBackend,
    tx: &crossbeam_channel::Sender<EngineToUi>,
    current: &Option<String>,
) {
    let Ok(list) = backend.list_output_devices() else {
        return;
    };
    // Resolve "current" — if we don't have an explicit choice, report
    // whichever device the host marks default, so the UI shows something.
    let current_id = current
        .clone()
        .or_else(|| list.iter().find(|d| d.is_default).map(|d| d.id.clone()))
        .unwrap_or_default();
    let entries: Vec<brume_app_protocol::OutputDeviceEntry> = list
        .into_iter()
        .map(|d| brume_app_protocol::OutputDeviceEntry {
            id: d.id,
            label: d.label,
            is_default: d.is_default,
        })
        .collect();
    let _ = tx.try_send(EngineToUi::OutputDeviceList {
        current: current_id,
        devices: entries,
    });
}

/// Owns the cpal audio stream for the iced/native UI path. Handles
/// app-level messages (`SetOutputDevice`, `RequestOutputDeviceList`)
/// off the iced main thread so:
///
///   1. iced never has to touch a non-Send `AudioStream`.
///   2. A device swap blocks only the audio worker, not the UI tick.
///
/// Exits when `app_rx` disconnects (UI window closes), which drops
/// the held stream and stops audio.
fn audio_control_loop(
    backend: CpalBackend,
    engine: Arc<Mutex<BrumeEngine>>,
    rx: crossbeam_channel::Receiver<UiToEngine>,
    engine_ui_tx: crossbeam_channel::Sender<EngineToUi>,
    app_rx: crossbeam_channel::Receiver<UiToEngine>,
    initial_device: Option<String>,
    initial_latency: LatencyPreset,
    settings_path: std::path::PathBuf,
) {
    let mut current_device = initial_device.clone();
    let mut current_latency = initial_latency;
    // Build a Settings snapshot from the two tracked fields. Used on every
    // save so changing one (device or latency) preserves the other.
    let save_settings = |device: &Option<String>, latency: LatencyPreset| {
        let s = brume_settings::Settings {
            output_device_id: device.clone(),
            audio_latency: latency,
        };
        if let Err(e) = s.save(&settings_path) {
            eprintln!("brume audio: failed to save settings: {e}");
        }
    };
    let mut current_stream: Option<Box<dyn AudioStream>> = open_audio_stream(
        &backend,
        engine.clone(),
        rx.clone(),
        current_device.clone(),
        current_latency,
    )
    .map_err(|e| {
        eprintln!("brume audio: initial open failed: {e}");
        e
    })
    .ok();

    // Initial enumeration + latency so SYS renders both on first paint.
    push_output_device_list(&backend, &engine_ui_tx, &current_device);
    let _ = engine_ui_tx.try_send(EngineToUi::AudioLatency {
        preset: current_latency,
    });

    while let Ok(msg) = app_rx.recv() {
        match msg {
            UiToEngine::SetOutputDevice { device_id } => {
                eprintln!("brume audio: switching output to {device_id:?}");
                // Drop the old stream first — cpal serialises Drop
                // against the worker callback, so we know no thread
                // is reading `rx` once this returns.
                drop(current_stream.take());
                let opened = open_audio_stream(
                    &backend,
                    engine.clone(),
                    rx.clone(),
                    Some(device_id.clone()),
                    current_latency,
                );
                match opened {
                    Ok(s) => {
                        current_stream = Some(s);
                        current_device = Some(device_id);
                        // Persist the user's choice. Non-fatal — a failed
                        // save just means the next launch reopens whatever
                        // was saved before. Preserves the latency setting.
                        save_settings(&current_device, current_latency);
                    }
                    Err(e) => {
                        eprintln!("brume audio: failed to open new device: {e}");
                        // Fall back to whatever was working before so
                        // we don't leave audio dead. If even that
                        // fails, current_stream stays None and the
                        // next swap attempt will retry.
                        if let Ok(s) = open_audio_stream(
                            &backend,
                            engine.clone(),
                            rx.clone(),
                            current_device.clone(),
                            current_latency,
                        ) {
                            current_stream = Some(s);
                        }
                    }
                }
                push_output_device_list(&backend, &engine_ui_tx, &current_device);
            }
            UiToEngine::SetAudioLatency { preset } => {
                eprintln!("brume audio: latency preset → {}", preset.label());
                current_latency = preset;
                // Rebuild the stream on the current device at the new
                // buffer. Drop first (same Drop-serialisation reasoning as
                // the device swap), then reopen; persist either way.
                drop(current_stream.take());
                match open_audio_stream(
                    &backend,
                    engine.clone(),
                    rx.clone(),
                    current_device.clone(),
                    current_latency,
                ) {
                    Ok(s) => current_stream = Some(s),
                    Err(e) => eprintln!("brume audio: reopen at new latency failed: {e}"),
                }
                save_settings(&current_device, current_latency);
                let _ = engine_ui_tx.try_send(EngineToUi::AudioLatency {
                    preset: current_latency,
                });
            }
            UiToEngine::RequestOutputDeviceList => {
                push_output_device_list(&backend, &engine_ui_tx, &current_device);
                let _ = engine_ui_tx.try_send(EngineToUi::AudioLatency {
                    preset: current_latency,
                });
            }
            // The UI only sends app-level variants on this channel;
            // anything else is a programming error. Log and keep
            // running — the channel stays drained either way.
            other => eprintln!("brume audio: unexpected app-channel message: {other:?}"),
        }
    }

    // Channel disconnected — UI closed. Dropping `current_stream`
    // here stops audio; the cpal worker thread joins in Drop.
}

/// Drains `rx` into the engine independently of the audio callback,
/// so MIDI input keeps reaching the engine even when the cpal worker
/// thread is blocked in `poll()` waiting for write-readiness from a
/// host that has stopped consuming (the canonical Meridian wedge
/// described in #6: route audio to UAC2Gadget with no DAW pulling
/// → cpal worker sleeps in `poll(POLLOUT)` indefinitely → audio
/// callback never fires → `rx` never drains → MIDI piles up at the
/// 256-deep bounded-channel limit and starts dropping CC events
/// within seconds).
///
/// crossbeam channels are MPMC, so this thread sharing `rx` with
/// the audio callback is safe; under healthy conditions both
/// consumers race for messages, with the audio callback usually
/// winning since it drains in batches per buffer (~10 ms cadence).
/// The drainer mostly finds an empty channel and blocks in
/// `recv()`. Under wedge conditions, the audio callback isn't
/// running, so the drainer is the sole consumer and MIDI input
/// keeps reaching the engine.
///
/// Per-message lock acquisitions are deliberate: they keep the
/// engine lock held for tens of nanoseconds at a time, so the
/// audio callback's `try_lock` rarely contends. On rare contention
/// the audio callback zero-fills one buffer (existing behavior).
///
/// This is a workaround for the structural coupling between the
/// engine event loop and the audio render loop. The proper fix is
/// a fully decoupled engine event thread with lock-free parameter
/// snapshots; that's its own project and not in scope here.
/// Resolves the user's home directory for engine-internal persistence
/// paths (`~/.brume/...`) and the user-script tree (`~/brume/scripts`).
/// Falls back to the current working directory if `$HOME` is unset or
/// empty, with a single visible warning — without it, a misconfigured
/// environment writes settings under whatever directory brume was
/// launched from and the "saved" UX silently evaporates on next boot.
fn resolve_home_dir() -> std::path::PathBuf {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
        _ => {
            eprintln!(
                "brume: $HOME is unset; persisting under CWD instead — \
                 settings may not survive across launches"
            );
            std::path::PathBuf::from(".")
        }
    }
}

fn midi_drainer_loop(engine: Arc<Mutex<BrumeEngine>>, rx: crossbeam_channel::Receiver<UiToEngine>) {
    while let Ok(msg) = rx.recv() {
        // Batch any messages already queued behind this one under a
        // single lock acquisition. Matches the audio callback's batching
        // shape so coalescing inside the engine (one
        // `ParameterChanged` echo per (part, id) per drain) still
        // works — without batching here, a knob sweep producing many
        // CCs would emit one echo per CC, which would hammer the
        // engine→UI channel under wedge conditions.
        let mut eng = engine.lock();
        eng.handle_message(msg);
        while let Ok(extra) = rx.try_recv() {
            eng.handle_message(extra);
        }
        // Flush echoes so the UI sees the parameter changes — without
        // this, `pending_echoes` accumulates until `process_block`
        // runs, which is exactly what doesn't happen under the
        // Meridian wedge.
        eng.flush_parameter_echoes();
    }
    // recv() returned Err — all senders have been dropped, i.e. the
    // engine is shutting down. Exit the thread.
}

/// Brume's entry point. Brings up the engine, audio, and MIDI, then
/// hands control to iced's main-thread event loop. Cross-platform:
/// same code runs on the CM5 reference target and on macOS dev
/// machines.
#[allow(clippy::too_many_lines)]
fn main() {
    println!("brume: starting (4-part multi-timbral)");

    let (tx, rx) = crossbeam_channel::bounded::<UiToEngine>(256);
    let (engine_ui_tx, engine_ui_rx) = crossbeam_channel::bounded::<EngineToUi>(1024);

    // Lua FX slot delivery channel. The UI constructs `LuaFxSlot`
    // instances when the user picks a script in the MIX page's LUA
    // tab; the audio thread drains this rx in `process_block` and
    // pushes the slot into `FxChain` (replace-by-name semantics, so
    // a single Lua FX slot sits at the end of the chain regardless
    // of how many times the user swaps scripts). Capacity 4 — the
    // UI never has more than one or two outstanding loads.
    let (fx_slot_tx, fx_slot_rx) = crossbeam_channel::bounded::<Box<dyn brume_fx_chain::FxSlot>>(4);

    // Engine. Audio callback shares ownership via Arc<Mutex<...>>.
    let engine = Arc::new(Mutex::new(BrumeEngine::new(SAMPLE_RATE)));
    {
        let mut eng = engine.lock();
        eng.set_ui_tx(engine_ui_tx.clone());
        eng.set_fx_slot_receiver(fx_slot_rx);
    }

    // Audio. Pick the user's stored device choice; if none, fall back
    // to the host default. Don't preferentially open UAC2Gadget
    // (Meridian) — that device only drains when a Mac is connected
    // over USB-C, so opening it on a standalone CM5 wedges
    // snd_pcm_writei after the first buffer and the cpal_alsa_out
    // thread sleeps in do_sys forever (no callbacks fire, the
    // UI→engine channel never drains, knob input dies).
    let backend = CpalBackend::new();
    let settings_path = brume_settings::Settings::default_path();
    let settings = brume_settings::Settings::load_or_default(&settings_path);
    let initial_device: Option<String> = match backend.list_output_devices() {
        Ok(list) => {
            let saved = settings.output_device_id.as_deref();
            saved
                .and_then(|id| list.iter().find(|d| d.id == id))
                .or_else(|| list.iter().find(|d| d.is_default))
                .or_else(|| list.first())
                .map(|d| d.id.clone())
        }
        Err(_) => None,
    };
    // App-level channel for messages that don't belong to the
    // audio thread (SetOutputDevice tears down + reopens the cpal
    // stream; RequestOutputDeviceList enumerates and pushes back).
    // The audio control thread below owns the stream and is the
    // single point of swap, so iced's main thread never has to
    // touch a non-Send AudioStream.
    let (app_tx, app_rx) = crossbeam_channel::bounded::<UiToEngine>(32);

    // Audio control thread. Owns the active cpal stream end-to-end
    // (open + hold + drop), which keeps the !Send AudioStream on a
    // single thread the whole time. Pushes the initial device list
    // so the SYS page renders without a "scanning" placeholder, then
    // loops on app_rx to handle live swaps.
    let engine_for_audio = engine.clone();
    let engine_ui_tx_for_audio = engine_ui_tx.clone();
    let rx_for_drainer = rx.clone();
    std::thread::Builder::new()
        .name("brume-audio-control".into())
        .spawn(move || {
            audio_control_loop(
                backend,
                engine_for_audio,
                rx,
                engine_ui_tx_for_audio,
                app_rx,
                initial_device,
                settings.audio_latency,
                settings_path,
            );
        })
        .expect("failed to spawn audio control thread");

    // MIDI drainer thread. Second consumer of `rx`, runs unconditionally
    // so MIDI keeps reaching the engine even when the cpal worker is
    // blocked in poll() during the Meridian wedge. See the doc comment
    // on `midi_drainer_loop` for the full rationale.
    let engine_for_drainer = engine.clone();
    std::thread::Builder::new()
        .name("brume-midi-drainer".into())
        .spawn(move || {
            midi_drainer_loop(engine_for_drainer, rx_for_drainer);
        })
        .expect("failed to spawn MIDI drainer thread");

    // Shared KnobMapping. ui-native::run() pre-populates this with
    // the FM tab bindings before iced starts. Later phases will
    // republish per active sub-tab via a dedicated message.
    let knob_mapping: Arc<std::sync::RwLock<brume_common::KnobMapping>> =
        Arc::new(std::sync::RwLock::new([None; 8]));

    // Resolve $HOME once for every persisted-state path below. If the
    // var is missing we fall back to CWD with a single visible warning
    // — without the warning, a misconfigured environment silently
    // writes settings under whatever directory brume happened to be
    // launched from, and the user's "saved" UX evaporates on next
    // boot. Compute the base once so all sites stay in sync if we
    // ever move from ~/.brume/ to an XDG path.
    let home_dir = resolve_home_dir();

    // MIDI. Load the ControlMatrix from `~/.brume/cc-bindings.json`
    // if present so the user's saved channel routing + CC bindings
    // come back across restarts. Falls back to defaults on any read
    // / parse / version-gate failure — a bad file should never block
    // startup, but each failure mode logs to stderr so a corrupted
    // or future-versioned file doesn't silently revert the user's
    // bindings without explanation.
    let cc_bindings_path = home_dir.join(".brume").join("cc-bindings.json");
    let initial_matrix = match std::fs::read_to_string(&cc_bindings_path) {
        Ok(s) => match ControlMatrix::from_json(&s) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "brume: cc-bindings.json at {} rejected ({e}) — using defaults",
                    cc_bindings_path.display()
                );
                ControlMatrix::new()
            }
        },
        Err(_) => {
            eprintln!(
                "brume: no cc-bindings.json at {} — using defaults",
                cc_bindings_path.display()
            );
            ControlMatrix::new()
        }
    };

    // Transport state (BPM + clock source). Persisted alongside the
    // CC bindings so the user's last clock-source selection survives
    // a binary swap. Read-fail / parse-fail degrades to "no persisted
    // state, use defaults" — a malformed transport.json must never
    // block startup.
    let transport_path = home_dir.join(".brume").join("transport.json");
    let initial_transport = brume_ui_native::persist::load_transport(&transport_path);
    if let Some(s) = initial_transport {
        // Replay the persisted state to the engine BEFORE the UI
        // starts so it boots with the correct mode + tempo from the
        // user's previous session, rather than the engine defaulting
        // to Internal/120 and only catching up the moment the user
        // touches a control.
        let _ = tx.try_send(brume_app_protocol::UiToEngine::SetTempo(s.bpm));
        let mode = match s.clock_mode_idx {
            1 => "master",
            2 => "external",
            _ => "off",
        };
        let _ = tx.try_send(brume_app_protocol::UiToEngine::SetClockMode(
            mode.to_string(),
        ));
    } else {
        eprintln!(
            "brume: no transport.json at {} — using defaults",
            transport_path.display()
        );
    }
    let control_matrix = Arc::new(std::sync::RwLock::new(initial_matrix));
    let midi_activity = Arc::new(Mutex::new(brume_midi_io::MidiActivity::new()));

    // MIDI Learn plumbing. UI flips `midi_learn_active`; midi-io
    // diverts the next CC into `capture_tx` and self-clears the
    // flag so the very next CC turn locks the binding. The UI
    // drains `capture_rx` on Tick.
    let midi_learn_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (midi_learn_capture_tx, midi_learn_capture_rx) =
        crossbeam_channel::bounded::<brume_midi_io::CcCaptured>(16);
    let midi_learn_state = brume_midi_io::MidiLearnState {
        active: midi_learn_active.clone(),
        capture_tx: midi_learn_capture_tx,
    };

    // The shipped control surface registry — shared by midi-io
    // (port detection + RT short-circuit) and ui-native (UI dispatch
    // through ControlSurfaceApi). Both sides see the same drivers.
    let surfaces = Arc::new(brume_ui_native::controllers::shipped_registry());

    // MIDI → script bridge. Bounded so a script that never reads
    // (or stalls under sandbox time-budget) can't grow this queue
    // without bound; midi-io drops events on a full channel rather
    // than blocking the input thread. 256 is generous: at peak MIDI
    // traffic (drum machine ≈ 1k events/s) and the dispatcher's 50ms
    // drain cadence, steady-state occupancy is ~50.
    let (script_midi_tx, script_midi_rx) =
        crossbeam_channel::bounded::<brume_midi_io::MidiEvent>(256);

    let _midi = BrumeMidiInput::start_with_script_tx(
        tx.clone(),
        control_matrix.clone(),
        midi_activity.clone(),
        Some(script_midi_tx),
        Some(midi_learn_state),
        Some(engine_ui_tx),
        knob_mapping.clone(),
        surfaces.clone(),
    );

    // Welcome chime. Spawned as a detached thread so the iced UI on the
    // main thread can come up while the chime plays its 5 s melody.
    // Sends `UiToEngine` messages over the same bounded channel as
    // every other input source; the audio thread drains them at the
    // top of each block.
    {
        let chime_tx = tx.clone();
        thread::Builder::new()
            .name("brume-startup-chime".into())
            .spawn(move || startup_chime(&chime_tx))
            .ok();
    }

    // Hand off to iced. Blocks until the window closes. The MIDI page
    // reads `midi_activity` (live note counts), mutates
    // `control_matrix` (channel routing + CC bindings), drives the
    // Learn flag, and drains the capture channel; the persist worker
    // is spawned inside `run` so it owns its handle.
    let lib_path = home_dir.join(".brume").join("library");
    // Failure here used to .expect() — a panic on first launch when
    // the library directory was unreadable (NFS, full disk, perms)
    // surfaced as a bare "panicked at 'failed to open patch library'"
    // and a service exit code of 101. Replace with a clean exit and
    // an actionable message; the same exit happens, just with the
    // user knowing how to fix it.
    let patch_library = match brume_patch_store::PatchLibrary::open(&lib_path) {
        Ok(lib) => Arc::new(lib),
        Err(e) => {
            eprintln!(
                "brume: failed to open patch library at {}: {e}\n\
                 brume: check the directory exists and is readable+writable, then restart.",
                lib_path.display()
            );
            std::process::exit(1);
        }
    };

    // Note: user-script directory lives at `~/brume/scripts` (no
    // leading dot) — visible to file managers, not hidden state. The
    // dotted `~/.brume/` directory is reserved for engine-internal
    // persistence (settings, library, cc-bindings, transport).
    let scripts_path = home_dir.join("brume").join("scripts");
    let _ = std::fs::create_dir_all(&scripts_path);
    let script_engine =
        brume_scripting::ScriptEngine::with_sample_rate(tx.clone(), &scripts_path, SAMPLE_RATE)
            .map(|se| Arc::new(std::sync::Mutex::new(se)))
            .ok();
    if script_engine.is_some() {
        eprintln!("brume: Lua scripting ready ({})", scripts_path.display());
    }

    // The script_midi_rx is only useful when the script engine
    // built; otherwise the receiver hangs around with no consumer
    // and midi-io's send-or-drop wastes a tiny bit of work. Pass
    // None when scripting is unavailable so the dispatcher in
    // ui-native skips the MIDI drain entirely.
    let script_midi_rx = script_engine.as_ref().map(|_| script_midi_rx);

    if let Err(e) = brume_ui_native::run(
        tx,
        app_tx,
        engine_ui_rx,
        knob_mapping,
        midi_activity,
        control_matrix,
        midi_learn_active,
        midi_learn_capture_rx,
        cc_bindings_path,
        transport_path,
        initial_transport,
        patch_library,
        script_engine,
        script_midi_rx,
        fx_slot_tx,
        surfaces,
    ) {
        eprintln!("brume-ui-native: {e}");
    }
}
