# Brume Controller Profiles

This directory holds bundled controller profiles — pre-baked CC → parameter mappings for known MIDI controllers. They're Brume's "Tier 2" integration story:

- **Tier 1 — MIDI Learn**: any controller, 30 seconds, no code. Built into the CC MAPPING panel.
- **Tier 2 — Profiles** (this directory): one tap to load a curated mapping for a known device.
- **Tier 3 — Lua scripts**: full programmatic control (bank switching, LED feedback, screen actions). Lives in `scripts/`.

## Currently shipped

| File | Device | Notes |
|------|--------|-------|
| `nanokontrol2.json` | Korg nanoKONTROL2 | Knobs map to Complex morph + FM controls; sliders to filter envelope + master volume. Buttons deferred. |

## Format

Each profile is a JSON envelope wrapping a literal serde-serialized `brume-control-model::ControlMatrix`. Open `~/.brume/cc-bindings.json` after running Brume once to see the canonical matrix shape.

```json
{
  "format": "brume-controller-profile/1",
  "name": "...",
  "description": "...",
  "controller_setup": "...",
  "author": "...",
  "match": {
    "port_name_contains": ["substring of MIDI port name"]
  },
  "matrix": { /* full ControlMatrix */ }
}
```

## Loading

When the loader implementation lands, profiles will be installed to `~/.brume/profiles/` on first run (copied from this bundled set). Loading a profile from the SYS UI is full-replacement — your existing `cc-bindings.json` is overwritten with the profile's matrix.

If you want to share custom bindings as a profile, the SYS UI will eventually grow a "Save current bindings as profile…" export flow. Until then, you can hand-author by:

1. Run Brume, do MIDI Learn until your bindings feel right.
2. Copy `~/.brume/cc-bindings.json` somewhere safe.
3. Wrap it in the envelope above (replace `"matrix": { ... }` with your file's contents).
4. Drop it in `~/.brume/profiles/` (or this `profiles/` directory if you want it bundled).

## Authoring a new profile

1. Get the controller into a known mode (factory CC mode, scene 1, etc.). Document the setup in `controller_setup`.
2. Identify the device's MIDI port name (visible in Brume's MIDI window header or via `aplay -l` / `aconnect -i` on Linux). Add a substring to `match.port_name_contains`.
3. Use Lua's `print()` to dump CC numbers as you turn each knob (see `scripts/harmonic-knobs.lua` for a template).
4. Hand-author the matrix bindings, or use Learn + the export flow once it lands.
5. Set ranges thoughtfully — `FmIndex` is 0-10, `ModulatorRatio` is 0.5-16, most other parameters are 0-1. Cutoff is in Hz (20-20000).
6. Submit a PR. Once the second profile lands here, future contributions move to a separate `brume-community-controllers` repo.

## What profiles can't do

Profiles are CC → parameter only. They can't:

- Trigger UI actions (page navigation, mute, solo, transport)
- Send MIDI back to the controller (LED feedback)
- Implement bank switching or modal behavior
- Transform values beyond linear range mapping

For any of those, write a Lua script and pair it with the profile. Convention: a "certified" controller ships as `profiles/<name>.json` + `scripts/controllers/<name>.lua`. Both are independent — load either, both, or neither.

## Implementation status

The profile *format* is locked. The *loader* is not yet implemented — these JSON files don't do anything at runtime yet.