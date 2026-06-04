// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! Deterministic render benchmark for the Brume engine hot path.
//!
//! This is intentionally dependency-light and not a Criterion benchmark. EVO
//! needs one stable numeric score plus behavior gates; this example prints both
//! per-scenario timings and aggregate metrics in a simple text format.

use std::hint::black_box;
use std::time::{Duration, Instant};

use brume_app_protocol::UiToEngine;
use brume_common::{OscillatorMode, ParameterId};
use brume_engine_runtime::{BrumeEngine, BrumeVoice, Part};

const SAMPLE_RATE: f32 = 48_000.0;
const DEFAULT_BLOCKS: usize = 2_000;
const DEFAULT_WARMUP_BLOCKS: usize = 128;
const DEFAULT_BLOCK_SIZE: usize = 512;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    channels: usize,
    block_size: usize,
    min_peak: f32,
    min_rms: f32,
    min_zero_crossings: usize,
    configure: fn(&mut BrumeEngine),
}

struct Metrics {
    elapsed: Duration,
    frames: usize,
    checksum: f64,
    peak: f32,
    rms: f32,
    zero_crossings: usize,
}

#[derive(Clone, Copy)]
struct VoiceScenario {
    name: &'static str,
    min_peak: f32,
    min_rms: f32,
    min_zero_crossings: usize,
    configure: fn(&mut BrumeVoice),
}

#[derive(Clone, Copy)]
struct PartScenario {
    name: &'static str,
    mode: OscillatorMode,
    min_peak: f32,
    min_rms: f32,
    min_zero_crossings: usize,
    configure: fn(&mut Part),
}

fn main() {
    let blocks = env_usize("BRUME_RENDER_BENCH_BLOCKS", DEFAULT_BLOCKS);
    let warmup_blocks = env_usize("BRUME_RENDER_BENCH_WARMUP_BLOCKS", DEFAULT_WARMUP_BLOCKS);
    let block_size = env_usize("BRUME_RENDER_BENCH_BLOCK_SIZE", DEFAULT_BLOCK_SIZE);

    let voice_scenarios = [
        VoiceScenario {
            name: "voice_fm_dense",
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_voice_fm_dense,
        },
        VoiceScenario {
            name: "voice_harmonic_scan",
            min_peak: 0.01,
            min_rms: 0.002,
            min_zero_crossings: 1_000,
            configure: configure_voice_harmonic_scan,
        },
        VoiceScenario {
            name: "voice_timbral_shaped",
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_voice_timbral_shaped,
        },
        VoiceScenario {
            name: "voice_granular_cloud",
            min_peak: 0.02,
            min_rms: 0.005,
            min_zero_crossings: 1_000,
            configure: configure_voice_granular_cloud,
        },
    ];
    let voice_scenario_count = voice_scenarios.len();

    let part_scenarios = [
        PartScenario {
            name: "part_fm_dense",
            mode: OscillatorMode::Fm,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_part_fm_dense,
        },
        PartScenario {
            name: "part_harmonic_scan",
            mode: OscillatorMode::Harmonic,
            min_peak: 0.02,
            min_rms: 0.005,
            min_zero_crossings: 1_000,
            configure: configure_part_harmonic_scan,
        },
        PartScenario {
            name: "part_timbral_shaped",
            mode: OscillatorMode::Timbral,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_part_timbral_shaped,
        },
        PartScenario {
            name: "part_granular_cloud",
            mode: OscillatorMode::Granular,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_part_granular_cloud,
        },
    ];
    let part_scenario_count = part_scenarios.len();

    let engine_scenarios = [
        Scenario {
            name: "fm_dense_stereo",
            channels: 2,
            block_size,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_fm_dense,
        },
        Scenario {
            name: "harmonic_scan_stereo",
            channels: 2,
            block_size,
            min_peak: 0.02,
            min_rms: 0.005,
            min_zero_crossings: 1_000,
            configure: configure_harmonic_scan,
        },
        Scenario {
            name: "timbral_shaped_stereo",
            channels: 2,
            block_size,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_timbral_shaped,
        },
        Scenario {
            name: "granular_cloud_stereo",
            channels: 2,
            block_size,
            min_peak: 0.05,
            min_rms: 0.01,
            min_zero_crossings: 1_000,
            configure: configure_granular_cloud,
        },
        Scenario {
            name: "full_engine_stereo",
            channels: 2,
            block_size,
            min_peak: 0.10,
            min_rms: 0.02,
            min_zero_crossings: 1_000,
            configure: configure_full_engine,
        },
        Scenario {
            name: "full_engine_8ch",
            channels: 8,
            block_size,
            min_peak: 0.10,
            min_rms: 0.02,
            min_zero_crossings: 1_000,
            configure: configure_full_engine,
        },
    ];
    let engine_scenario_count = engine_scenarios.len();

    println!(
        "brume_render_bench sample_rate={} blocks={} warmup_blocks={} block_size={}",
        SAMPLE_RATE, blocks, warmup_blocks, block_size
    );

    let mut voice_total_ns_per_frame = 0.0_f64;
    let mut voice_total_frames = 0_usize;
    let mut voice_total_checksum = 0.0_f64;

    for scenario in voice_scenarios {
        let metrics = run_voice_scenario(scenario, blocks, warmup_blocks, block_size);
        let ns_per_frame = metrics.elapsed.as_nanos() as f64 / metrics.frames as f64;
        let rendered_seconds = metrics.frames as f64 / f64::from(SAMPLE_RATE);
        let realtime_multiple = rendered_seconds / metrics.elapsed.as_secs_f64();

        println!(
            "voice_scenario={} block_size={} frames={} elapsed_ms={:.3} ns_per_frame={:.3} realtime_x={:.3} peak={:.6} rms={:.6} zero_crossings={} checksum={:.6}",
            scenario.name,
            block_size,
            metrics.frames,
            metrics.elapsed.as_secs_f64() * 1_000.0,
            ns_per_frame,
            realtime_multiple,
            metrics.peak,
            metrics.rms,
            metrics.zero_crossings,
            metrics.checksum,
        );

        voice_total_ns_per_frame += ns_per_frame;
        voice_total_frames += metrics.frames;
        voice_total_checksum += metrics.checksum;
    }

    let mut part_total_ns_per_frame = 0.0_f64;
    let mut part_total_frames = 0_usize;
    let mut part_total_checksum = 0.0_f64;

    for scenario in part_scenarios {
        let metrics = run_part_scenario(scenario, blocks, warmup_blocks, block_size);
        let ns_per_frame = metrics.elapsed.as_nanos() as f64 / metrics.frames as f64;
        let rendered_seconds = metrics.frames as f64 / f64::from(SAMPLE_RATE);
        let realtime_multiple = rendered_seconds / metrics.elapsed.as_secs_f64();

        println!(
            "part_scenario={} block_size={} frames={} elapsed_ms={:.3} ns_per_frame={:.3} realtime_x={:.3} peak={:.6} rms={:.6} zero_crossings={} checksum={:.6}",
            scenario.name,
            block_size,
            metrics.frames,
            metrics.elapsed.as_secs_f64() * 1_000.0,
            ns_per_frame,
            realtime_multiple,
            metrics.peak,
            metrics.rms,
            metrics.zero_crossings,
            metrics.checksum,
        );

        part_total_ns_per_frame += ns_per_frame;
        part_total_frames += metrics.frames;
        part_total_checksum += metrics.checksum;
    }

    let mut engine_total_ns_per_frame = 0.0_f64;
    let mut engine_total_frames = 0_usize;
    let mut engine_total_checksum = 0.0_f64;
    let mut total_checksum = 0.0_f64;

    for scenario in engine_scenarios {
        let metrics = run_scenario(scenario, blocks, warmup_blocks);
        let ns_per_frame = metrics.elapsed.as_nanos() as f64 / metrics.frames as f64;
        let rendered_seconds = metrics.frames as f64 / f64::from(SAMPLE_RATE);
        let realtime_multiple = rendered_seconds / metrics.elapsed.as_secs_f64();

        println!(
            "scenario={} channels={} block_size={} frames={} elapsed_ms={:.3} ns_per_frame={:.3} realtime_x={:.3} peak={:.6} rms={:.6} zero_crossings={} checksum={:.6}",
            scenario.name,
            scenario.channels,
            scenario.block_size,
            metrics.frames,
            metrics.elapsed.as_secs_f64() * 1_000.0,
            ns_per_frame,
            realtime_multiple,
            metrics.peak,
            metrics.rms,
            metrics.zero_crossings,
            metrics.checksum,
        );

        engine_total_ns_per_frame += ns_per_frame;
        engine_total_frames += metrics.frames;
        engine_total_checksum += metrics.checksum;
    }

    let voice_mean_ns_per_frame = voice_total_ns_per_frame / voice_scenario_count as f64;
    let part_mean_ns_per_frame = part_total_ns_per_frame / part_scenario_count as f64;
    let engine_mean_ns_per_frame = engine_total_ns_per_frame / engine_scenario_count as f64;
    total_checksum += voice_total_checksum + part_total_checksum + engine_total_checksum;
    let all_scenario_count = voice_scenario_count + part_scenario_count + engine_scenario_count;
    let mean_ns_per_frame =
        (voice_total_ns_per_frame + part_total_ns_per_frame + engine_total_ns_per_frame)
            / all_scenario_count as f64;
    println!(
        "summary voice_scenarios={} voice_frames={} voice_mean_ns_per_frame={:.3} part_scenarios={} part_frames={} part_mean_ns_per_frame={:.3} engine_scenarios={} engine_frames={} engine_mean_ns_per_frame={:.3} all_scenarios={} mean_ns_per_frame={:.3} checksum={:.6}",
        voice_scenario_count,
        voice_total_frames,
        voice_mean_ns_per_frame,
        part_scenario_count,
        part_total_frames,
        part_mean_ns_per_frame,
        engine_scenario_count,
        engine_total_frames,
        engine_mean_ns_per_frame,
        all_scenario_count,
        mean_ns_per_frame,
        total_checksum
    );
}

fn run_part_scenario(
    scenario: PartScenario,
    blocks: usize,
    warmup_blocks: usize,
    block_size: usize,
) -> Metrics {
    let mut part = Part::new(scenario.mode, SAMPLE_RATE);
    (scenario.configure)(&mut part);
    let mut buffer = vec![0.0_f32; block_size];

    for _ in 0..warmup_blocks {
        part.process_buffer(block_size, &mut buffer);
        black_box(&buffer);
    }

    let start = Instant::now();
    let mut checksum = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut zero_crossings = 0_usize;
    let mut prev = 0.0_f32;
    let mut have_prev = false;

    for _ in 0..blocks {
        part.process_buffer(block_size, &mut buffer);
        for &sample in &buffer {
            if !sample.is_finite() {
                panic!("{} produced non-finite sample: {sample}", scenario.name);
            }
            if have_prev && ((prev <= 0.0 && sample > 0.0) || (prev >= 0.0 && sample < 0.0)) {
                zero_crossings += 1;
            }
            prev = sample;
            have_prev = true;

            let abs = sample.abs();
            peak = peak.max(abs);
            checksum += f64::from(sample);
            sum_sq += f64::from(sample) * f64::from(sample);
        }
        black_box(&buffer);
    }

    let elapsed = start.elapsed();
    let frames = blocks * block_size;
    let rms = (sum_sq / frames as f64).sqrt() as f32;
    if peak < scenario.min_peak || rms < scenario.min_rms {
        panic!(
            "{} failed activity floor: peak={peak} rms={rms} min_peak={} min_rms={}",
            scenario.name, scenario.min_peak, scenario.min_rms
        );
    }
    if zero_crossings < scenario.min_zero_crossings {
        panic!(
            "{} failed zero-crossing floor: zero_crossings={zero_crossings} min_zero_crossings={}",
            scenario.name, scenario.min_zero_crossings
        );
    }

    Metrics {
        elapsed,
        frames,
        checksum,
        peak,
        rms,
        zero_crossings,
    }
}

fn run_voice_scenario(
    scenario: VoiceScenario,
    blocks: usize,
    warmup_blocks: usize,
    block_size: usize,
) -> Metrics {
    let mut voice = BrumeVoice::new(SAMPLE_RATE);
    (scenario.configure)(&mut voice);
    let mut buffer = vec![0.0_f32; block_size];

    for _ in 0..warmup_blocks {
        render_voice_block(&mut voice, &mut buffer);
        black_box(&buffer);
    }

    let start = Instant::now();
    let mut checksum = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut zero_crossings = 0_usize;
    let mut prev = 0.0_f32;
    let mut have_prev = false;

    for _ in 0..blocks {
        render_voice_block(&mut voice, &mut buffer);
        for &sample in &buffer {
            if !sample.is_finite() {
                panic!("{} produced non-finite sample: {sample}", scenario.name);
            }
            if have_prev && ((prev <= 0.0 && sample > 0.0) || (prev >= 0.0 && sample < 0.0)) {
                zero_crossings += 1;
            }
            prev = sample;
            have_prev = true;

            let abs = sample.abs();
            peak = peak.max(abs);
            checksum += f64::from(sample);
            sum_sq += f64::from(sample) * f64::from(sample);
        }
        black_box(&buffer);
    }

    let elapsed = start.elapsed();
    let frames = blocks * block_size;
    let rms = (sum_sq / frames as f64).sqrt() as f32;
    if peak < scenario.min_peak || rms < scenario.min_rms {
        panic!(
            "{} failed activity floor: peak={peak} rms={rms} min_peak={} min_rms={}",
            scenario.name, scenario.min_peak, scenario.min_rms
        );
    }
    if zero_crossings < scenario.min_zero_crossings {
        panic!(
            "{} failed zero-crossing floor: zero_crossings={zero_crossings} min_zero_crossings={}",
            scenario.name, scenario.min_zero_crossings
        );
    }

    Metrics {
        elapsed,
        frames,
        checksum,
        peak,
        rms,
        zero_crossings,
    }
}

fn render_voice_block(voice: &mut BrumeVoice, buffer: &mut [f32]) {
    for sample in buffer {
        *sample = voice.process();
    }
}

fn run_scenario(scenario: Scenario, blocks: usize, warmup_blocks: usize) -> Metrics {
    let mut engine = BrumeEngine::new(SAMPLE_RATE);
    (scenario.configure)(&mut engine);

    let mut buffer = vec![0.0_f32; scenario.block_size * scenario.channels];

    for _ in 0..warmup_blocks {
        engine.process_block(&mut buffer, scenario.channels);
        black_box(&buffer);
    }

    let start = Instant::now();
    let mut checksum = 0.0_f64;
    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut sample_count = 0_usize;
    let mut zero_crossings = 0_usize;
    let mut prev_primary = 0.0_f32;
    let mut have_prev_primary = false;

    for _ in 0..blocks {
        engine.process_block(&mut buffer, scenario.channels);
        for frame in buffer.chunks_exact(scenario.channels) {
            let sample = frame[0];
            if have_prev_primary
                && ((prev_primary <= 0.0 && sample > 0.0) || (prev_primary >= 0.0 && sample < 0.0))
            {
                zero_crossings += 1;
            }
            prev_primary = sample;
            have_prev_primary = true;
        }
        for &sample in &buffer {
            if !sample.is_finite() {
                panic!("{} produced non-finite sample: {sample}", scenario.name);
            }
            let abs = sample.abs();
            peak = peak.max(abs);
            checksum += f64::from(sample);
            sum_sq += f64::from(sample) * f64::from(sample);
            sample_count += 1;
        }
        black_box(&buffer);
    }

    let elapsed = start.elapsed();
    let rms = (sum_sq / sample_count as f64).sqrt() as f32;
    if peak < scenario.min_peak || rms < scenario.min_rms {
        panic!(
            "{} failed activity floor: peak={peak} rms={rms} min_peak={} min_rms={}",
            scenario.name, scenario.min_peak, scenario.min_rms
        );
    }
    if zero_crossings < scenario.min_zero_crossings {
        panic!(
            "{} failed zero-crossing floor: zero_crossings={zero_crossings} min_zero_crossings={}",
            scenario.name, scenario.min_zero_crossings
        );
    }

    Metrics {
        elapsed,
        frames: blocks * scenario.block_size,
        checksum,
        peak,
        rms,
        zero_crossings,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| value > 0)
        .unwrap_or(default)
}

fn set(engine: &mut BrumeEngine, part: u8, id: ParameterId, value: f32) {
    engine.handle_message(UiToEngine::SetParameter { part, id, value });
}

fn note_on(engine: &mut BrumeEngine, part: u8, note: u8, velocity: f32) {
    engine.handle_message(UiToEngine::NoteOn {
        part,
        note,
        velocity,
    });
}

fn configure_fm_dense(engine: &mut BrumeEngine) {
    set(engine, 0, ParameterId::Algorithm, 4.0);
    set(engine, 0, ParameterId::FmIndex, 6.5);
    set(engine, 0, ParameterId::FmFeedback, 0.55);
    set(engine, 0, ParameterId::FilterCutoff, 14_000.0);
    set(engine, 0, ParameterId::FilterResonance, 0.2);
    set(engine, 0, ParameterId::AmpAttack, 2.0);
    set(engine, 0, ParameterId::AmpRelease, 600.0);
    for (id, value) in [
        (ParameterId::Op1Level, 0.85),
        (ParameterId::Op2Level, 0.65),
        (ParameterId::Op3Level, 0.55),
        (ParameterId::Op4Level, 0.45),
        (ParameterId::Op5Level, 0.40),
        (ParameterId::Op6Level, 0.35),
        (ParameterId::Op2Ratio, 1.997),
        (ParameterId::Op3Ratio, 2.75),
        (ParameterId::Op4Ratio, 3.5),
        (ParameterId::Op5Ratio, 5.0),
        (ParameterId::Op6Ratio, 8.0),
    ] {
        set(engine, 0, id, value);
    }
    play_chord(engine, 0, &[40, 47, 52, 55, 59, 64], 0.82);
}

fn configure_harmonic_scan(engine: &mut BrumeEngine) {
    set(engine, 1, ParameterId::HarmonicTilt, 0.75);
    set(engine, 1, ParameterId::HarmonicOddEven, 0.25);
    set(engine, 1, ParameterId::Inharmonicity, 0.8);
    set(engine, 1, ParameterId::ScanCenter, 0.62);
    set(engine, 1, ParameterId::ScanWidth, 0.22);
    set(engine, 1, ParameterId::HarmonicMorph, 0.65);
    set(engine, 1, ParameterId::HarmonicFmDepth, 2.75);
    set(engine, 1, ParameterId::HarmonicFmRatio, 3.0);
    set(engine, 1, ParameterId::HarmonicSpread, 0.45);
    set(engine, 1, ParameterId::FilterCutoff, 10_000.0);
    play_chord(engine, 1, &[48, 52, 55, 59, 64, 67], 0.72);
}

fn configure_timbral_shaped(engine: &mut BrumeEngine) {
    set(engine, 2, ParameterId::Timbre, 0.92);
    set(engine, 2, ParameterId::Symmetry, 0.78);
    set(engine, 2, ParameterId::TimbralFmDepth, 2.0);
    set(engine, 2, ParameterId::TimbralFmRatio, 2.5);
    set(engine, 2, ParameterId::SubLevel, 0.55);
    set(engine, 2, ParameterId::TimbralFeedback, 0.42);
    set(engine, 2, ParameterId::MultiplierStages, 4.0);
    set(engine, 2, ParameterId::FilterCutoff, 8_500.0);
    set(engine, 2, ParameterId::FilterResonance, 0.18);
    play_chord(engine, 2, &[36, 43, 48, 52, 55, 60], 0.75);
}

fn configure_granular_cloud(engine: &mut BrumeEngine) {
    set(engine, 3, ParameterId::GranularDensity, 1.0);
    set(engine, 3, ParameterId::GranularGrainSize, 0.32);
    set(engine, 3, ParameterId::GranularMorph, 0.72);
    set(engine, 3, ParameterId::GranularScatter, 0.65);
    set(engine, 3, ParameterId::GranularSpread, 0.8);
    set(engine, 3, ParameterId::GranularDrift, 0.85);
    set(engine, 3, ParameterId::GranularFmDepth, 2.4);
    set(engine, 3, ParameterId::GranularFmRatio, 1.5);
    set(engine, 3, ParameterId::GranularPosition, 0.35);
    set(engine, 3, ParameterId::GranularGrainShape, 0.72);
    set(engine, 3, ParameterId::FilterCutoff, 7_000.0);
    play_chord(engine, 3, &[43, 48, 50, 55, 59, 62], 0.7);
}

fn configure_full_engine(engine: &mut BrumeEngine) {
    configure_fm_dense(engine);
    configure_harmonic_scan(engine);
    configure_timbral_shaped(engine);
    configure_granular_cloud(engine);
    engine.handle_message(UiToEngine::SetPartDelaySend {
        part: 0,
        level: 0.45,
    });
    engine.handle_message(UiToEngine::SetPartDelaySend {
        part: 1,
        level: 0.35,
    });
    engine.handle_message(UiToEngine::SetPartReverbSend {
        part: 2,
        level: 0.5,
    });
    engine.handle_message(UiToEngine::SetPartReverbSend {
        part: 3,
        level: 0.6,
    });
    engine.handle_message(UiToEngine::SetMasterVolume(0.72));
}

fn play_chord(engine: &mut BrumeEngine, part: u8, notes: &[u8], velocity: f32) {
    for &note in notes {
        note_on(engine, part, note, velocity);
    }
}

fn configure_part_fm_dense(part: &mut Part) {
    part.set_parameter(ParameterId::Algorithm, 4.0);
    part.set_parameter(ParameterId::FmIndex, 6.5);
    part.set_parameter(ParameterId::FmFeedback, 0.55);
    part.set_parameter(ParameterId::FilterCutoff, 14_000.0);
    part.set_parameter(ParameterId::FilterResonance, 0.2);
    part.set_parameter(ParameterId::AmpAttack, 2.0);
    part.set_parameter(ParameterId::AmpRelease, 600.0);
    for (id, value) in [
        (ParameterId::Op1Level, 0.85),
        (ParameterId::Op2Level, 0.65),
        (ParameterId::Op3Level, 0.55),
        (ParameterId::Op4Level, 0.45),
        (ParameterId::Op5Level, 0.40),
        (ParameterId::Op6Level, 0.35),
        (ParameterId::Op2Ratio, 1.997),
        (ParameterId::Op3Ratio, 2.75),
        (ParameterId::Op4Ratio, 3.5),
        (ParameterId::Op5Ratio, 5.0),
        (ParameterId::Op6Ratio, 8.0),
    ] {
        part.set_parameter(id, value);
    }
    play_part_chord(part, &[40, 47, 52, 55, 59, 64], 0.82);
}

fn configure_part_harmonic_scan(part: &mut Part) {
    part.set_parameter(ParameterId::HarmonicTilt, 0.75);
    part.set_parameter(ParameterId::HarmonicOddEven, 0.25);
    part.set_parameter(ParameterId::Inharmonicity, 0.8);
    part.set_parameter(ParameterId::ScanCenter, 0.62);
    part.set_parameter(ParameterId::ScanWidth, 0.22);
    part.set_parameter(ParameterId::HarmonicMorph, 0.65);
    part.set_parameter(ParameterId::HarmonicFmDepth, 2.75);
    part.set_parameter(ParameterId::HarmonicFmRatio, 3.0);
    part.set_parameter(ParameterId::HarmonicSpread, 0.45);
    part.set_parameter(ParameterId::FilterCutoff, 10_000.0);
    play_part_chord(part, &[48, 52, 55, 59, 64, 67], 0.72);
}

fn configure_part_timbral_shaped(part: &mut Part) {
    part.set_parameter(ParameterId::Timbre, 0.92);
    part.set_parameter(ParameterId::Symmetry, 0.78);
    part.set_parameter(ParameterId::TimbralFmDepth, 2.0);
    part.set_parameter(ParameterId::TimbralFmRatio, 2.5);
    part.set_parameter(ParameterId::SubLevel, 0.55);
    part.set_parameter(ParameterId::TimbralFeedback, 0.42);
    part.set_parameter(ParameterId::MultiplierStages, 4.0);
    part.set_parameter(ParameterId::FilterCutoff, 8_500.0);
    part.set_parameter(ParameterId::FilterResonance, 0.18);
    play_part_chord(part, &[36, 43, 48, 52, 55, 60], 0.75);
}

fn configure_part_granular_cloud(part: &mut Part) {
    part.set_parameter(ParameterId::GranularDensity, 1.0);
    part.set_parameter(ParameterId::GranularGrainSize, 0.32);
    part.set_parameter(ParameterId::GranularMorph, 0.72);
    part.set_parameter(ParameterId::GranularScatter, 0.65);
    part.set_parameter(ParameterId::GranularSpread, 0.8);
    part.set_parameter(ParameterId::GranularDrift, 0.85);
    part.set_parameter(ParameterId::GranularFmDepth, 2.4);
    part.set_parameter(ParameterId::GranularFmRatio, 1.5);
    part.set_parameter(ParameterId::GranularPosition, 0.35);
    part.set_parameter(ParameterId::GranularGrainShape, 0.72);
    part.set_parameter(ParameterId::FilterCutoff, 7_000.0);
    play_part_chord(part, &[43, 48, 50, 55, 59, 62], 0.7);
}

fn play_part_chord(part: &mut Part, notes: &[u8], velocity: f32) {
    for &note in notes {
        part.note_on(note, velocity);
    }
}

fn configure_voice_fm_dense(voice: &mut BrumeVoice) {
    voice.set_oscillator_mode(OscillatorMode::Fm);
    set_voice_defaults(voice);
    voice.set_parameter(ParameterId::Algorithm, 4.0);
    voice.set_parameter(ParameterId::FmIndex, 6.5);
    voice.set_parameter(ParameterId::FmFeedback, 0.55);
    voice.set_parameter(ParameterId::FilterCutoff, 14_000.0);
    voice.set_parameter(ParameterId::FilterResonance, 0.2);
    for (id, value) in [
        (ParameterId::Op1Level, 0.85),
        (ParameterId::Op2Level, 0.65),
        (ParameterId::Op3Level, 0.55),
        (ParameterId::Op4Level, 0.45),
        (ParameterId::Op5Level, 0.40),
        (ParameterId::Op6Level, 0.35),
        (ParameterId::Op2Ratio, 1.997),
        (ParameterId::Op3Ratio, 2.75),
        (ParameterId::Op4Ratio, 3.5),
        (ParameterId::Op5Ratio, 5.0),
        (ParameterId::Op6Ratio, 8.0),
    ] {
        voice.set_parameter(id, value);
    }
    voice.note_on(52, 0.9);
}

fn configure_voice_harmonic_scan(voice: &mut BrumeVoice) {
    voice.set_oscillator_mode(OscillatorMode::Harmonic);
    set_voice_defaults(voice);
    voice.set_parameter(ParameterId::HarmonicTilt, 0.75);
    voice.set_parameter(ParameterId::HarmonicOddEven, 0.25);
    voice.set_parameter(ParameterId::Inharmonicity, 0.8);
    voice.set_parameter(ParameterId::ScanCenter, 0.62);
    voice.set_parameter(ParameterId::ScanWidth, 0.22);
    voice.set_parameter(ParameterId::HarmonicMorph, 0.65);
    voice.set_parameter(ParameterId::HarmonicFmDepth, 2.75);
    voice.set_parameter(ParameterId::HarmonicFmRatio, 3.0);
    voice.set_parameter(ParameterId::HarmonicSpread, 0.45);
    voice.set_parameter(ParameterId::FilterCutoff, 10_000.0);
    voice.note_on(55, 0.85);
}

fn configure_voice_timbral_shaped(voice: &mut BrumeVoice) {
    voice.set_oscillator_mode(OscillatorMode::Timbral);
    set_voice_defaults(voice);
    voice.set_parameter(ParameterId::Timbre, 0.92);
    voice.set_parameter(ParameterId::Symmetry, 0.78);
    voice.set_parameter(ParameterId::TimbralFmDepth, 2.0);
    voice.set_parameter(ParameterId::TimbralFmRatio, 2.5);
    voice.set_parameter(ParameterId::SubLevel, 0.55);
    voice.set_parameter(ParameterId::TimbralFeedback, 0.42);
    voice.set_parameter(ParameterId::MultiplierStages, 4.0);
    voice.set_parameter(ParameterId::FilterCutoff, 8_500.0);
    voice.set_parameter(ParameterId::FilterResonance, 0.18);
    voice.note_on(48, 0.9);
}

fn configure_voice_granular_cloud(voice: &mut BrumeVoice) {
    voice.set_oscillator_mode(OscillatorMode::Granular);
    set_voice_defaults(voice);
    voice.set_parameter(ParameterId::GranularDensity, 1.0);
    voice.set_parameter(ParameterId::GranularGrainSize, 0.32);
    voice.set_parameter(ParameterId::GranularMorph, 0.72);
    voice.set_parameter(ParameterId::GranularScatter, 0.65);
    voice.set_parameter(ParameterId::GranularSpread, 0.8);
    voice.set_parameter(ParameterId::GranularDrift, 0.85);
    voice.set_parameter(ParameterId::GranularFmDepth, 2.4);
    voice.set_parameter(ParameterId::GranularFmRatio, 1.5);
    voice.set_parameter(ParameterId::GranularPosition, 0.35);
    voice.set_parameter(ParameterId::GranularGrainShape, 0.72);
    voice.set_parameter(ParameterId::FilterCutoff, 7_000.0);
    voice.note_on(50, 0.9);
}

fn set_voice_defaults(voice: &mut BrumeVoice) {
    voice.set_parameter(ParameterId::AmpAttack, 2.0);
    voice.set_parameter(ParameterId::AmpDecay, 200.0);
    voice.set_parameter(ParameterId::AmpSustain, 1.0);
    voice.set_parameter(ParameterId::AmpRelease, 1_000.0);
    voice.set_parameter(ParameterId::FilterEnvDepth, 0.2);
}
