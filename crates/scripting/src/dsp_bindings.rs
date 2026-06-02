// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! DSP primitives exposed to Lua as userdata objects.
//!
//! Scripts can create and use these to build custom effects:
//! ```lua
//! local ap = dsp.allpass(1024, 0.5)
//! local dl = dsp.delay(48000)
//! local lp = dsp.lowpass(48000, 1000)
//! local sat = dsp.saturator()
//!
//! -- Per-sample processing
//! local out = ap:process(input)
//! ```

use mlua::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

use brume_dsp_core::delay::{AllpassDelay, DelayLine};
use brume_dsp_core::filter::OnePole;
use brume_dsp_core::saturation::{SaturationType, Saturator, Wavefolder};

// ── Lua wrappers ──
// Each wraps a DSP object in Arc<parking_lot::Mutex> so Lua userdata
// references stay safe to share. parking_lot::Mutex doesn't poison on
// panic, so a panic anywhere in script-driven DSP code can't kill the
// audio thread on the next sample by way of an unwrap on a poisoned
// std::sync::Mutex.

struct LuaDelay(Arc<Mutex<DelayLine>>);
struct LuaAllpass(Arc<Mutex<AllpassDelay>>);
struct LuaFilter(Arc<Mutex<OnePole>>);
struct LuaSaturator(Arc<Mutex<Saturator>>);
struct LuaWavefolder(Arc<Mutex<Wavefolder>>);

impl LuaUserData for LuaDelay {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("process", |_, this, input: f32| {
            Ok(this.0.lock().process(input))
        });
        methods.add_method_mut("set_time", |_, this, samples: f32| {
            this.0.lock().set_delay_samples(samples);
            Ok(())
        });
        methods.add_method_mut("reset", |_, this, ()| {
            this.0.lock().reset();
            Ok(())
        });
    }
}

impl LuaUserData for LuaAllpass {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("process", |_, this, input: f32| {
            Ok(this.0.lock().process(input))
        });
        methods.add_method_mut("set_time", |_, this, samples: f32| {
            this.0.lock().set_delay_samples(samples);
            Ok(())
        });
        methods.add_method_mut("set_gain", |_, this, gain: f32| {
            this.0.lock().set_gain(gain);
            Ok(())
        });
        methods.add_method_mut("reset", |_, this, ()| {
            this.0.lock().reset();
            Ok(())
        });
    }
}

impl LuaUserData for LuaFilter {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("process", |_, this, input: f32| {
            Ok(this.0.lock().process(input))
        });
        methods.add_method_mut("set_cutoff", |_, this, hz: f32| {
            this.0.lock().set_cutoff(hz);
            Ok(())
        });
        methods.add_method_mut("reset", |_, this, ()| {
            this.0.lock().reset();
            Ok(())
        });
    }
}

impl LuaUserData for LuaSaturator {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("process", |_, this, input: f32| {
            Ok(this.0.lock().process(input))
        });
        methods.add_method_mut("set_drive", |_, this, drive: f32| {
            this.0.lock().set_drive(drive);
            Ok(())
        });
        methods.add_method_mut("set_type", |_, this, t: u8| {
            let sat_type = match t {
                0 => SaturationType::Soft,
                1 => SaturationType::Hard,
                2 => SaturationType::Tape,
                _ => SaturationType::Tube,
            };
            this.0.lock().set_type(sat_type);
            Ok(())
        });
        methods.add_method_mut("set_mix", |_, this, mix: f32| {
            this.0.lock().set_mix(mix);
            Ok(())
        });
    }
}

impl LuaUserData for LuaWavefolder {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("process", |_, this, input: f32| {
            Ok(this.0.lock().process(input))
        });
        methods.add_method_mut("set_drive", |_, this, drive: f32| {
            this.0.lock().set_drive(drive);
            Ok(())
        });
        methods.add_method_mut("set_stages", |_, this, stages: u32| {
            this.0.lock().set_stages(stages);
            Ok(())
        });
    }
}

/// Registers the `dsp` factory table on the Lua state.
pub fn register_dsp(lua: &Lua, sample_rate: f32) -> LuaResult<()> {
    let dsp = lua.create_table()?;

    // dsp.delay(max_samples) → DelayLine
    dsp.set(
        "delay",
        lua.create_function(move |_, max_samples: usize| {
            let dl = DelayLine::new(max_samples);
            Ok(LuaDelay(Arc::new(Mutex::new(dl))))
        })?,
    )?;

    // dsp.allpass(max_samples, gain) → AllpassDelay
    dsp.set(
        "allpass",
        lua.create_function(move |_, (max_samples, gain): (usize, f32)| {
            let mut ap = AllpassDelay::new(max_samples);
            ap.set_gain(gain);
            ap.set_delay_samples((max_samples - 1) as f32);
            Ok(LuaAllpass(Arc::new(Mutex::new(ap))))
        })?,
    )?;

    // dsp.lowpass(sample_rate, cutoff) → OnePole lowpass filter
    dsp.set(
        "lowpass",
        lua.create_function(move |_, cutoff: f32| {
            let mut f = OnePole::lowpass(sample_rate);
            f.set_cutoff(cutoff);
            Ok(LuaFilter(Arc::new(Mutex::new(f))))
        })?,
    )?;

    // dsp.highpass(cutoff) → OnePole highpass filter
    dsp.set(
        "highpass",
        lua.create_function(move |_, cutoff: f32| {
            let mut f = OnePole::highpass(sample_rate);
            f.set_cutoff(cutoff);
            Ok(LuaFilter(Arc::new(Mutex::new(f))))
        })?,
    )?;

    // dsp.saturator() → Saturator
    dsp.set(
        "saturator",
        lua.create_function(move |_, ()| Ok(LuaSaturator(Arc::new(Mutex::new(Saturator::new())))))?,
    )?;

    // dsp.wavefolder() → Wavefolder
    dsp.set(
        "wavefolder",
        lua.create_function(move |_, ()| {
            Ok(LuaWavefolder(Arc::new(Mutex::new(Wavefolder::new()))))
        })?,
    )?;

    // dsp.sample_rate → number
    dsp.set("sample_rate", sample_rate)?;

    lua.globals().set("dsp", dsp)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_delay_roundtrip() {
        let lua = Lua::new();
        register_dsp(&lua, 48000.0).unwrap();

        let result: f32 = lua
            .load(
                r#"
            local d = dsp.delay(100)
            d:set_time(10)
            d:process(1.0)  -- write impulse
            local out = 0
            for i = 1, 10 do out = d:process(0.0) end
            return out
        "#,
            )
            .eval()
            .unwrap();

        assert!(
            result > 0.5,
            "delay should output the impulse: got {result}"
        );
    }

    #[test]
    fn dsp_allpass_bounded() {
        let lua = Lua::new();
        register_dsp(&lua, 48000.0).unwrap();

        let result: f32 = lua
            .load(
                r#"
            local ap = dsp.allpass(100, 0.5)
            ap:set_time(50)
            local peak = 0
            for i = 1, 200 do
                local out = ap:process(i == 1 and 1.0 or 0.0)
                if math.abs(out) > peak then peak = math.abs(out) end
            end
            return peak
        "#,
            )
            .eval()
            .unwrap();

        assert!(result < 2.0, "allpass should be bounded: got {result}");
    }

    #[test]
    fn dsp_filter_works() {
        let lua = Lua::new();
        register_dsp(&lua, 48000.0).unwrap();

        let result: f32 = lua
            .load(
                r#"
            local lp = dsp.lowpass(200)
            local out = 0
            for i = 1, 100 do
                out = lp:process(1.0)
            end
            return out
        "#,
            )
            .eval()
            .unwrap();

        assert!(result > 0.5, "lowpass should pass DC: got {result}");
    }
}
