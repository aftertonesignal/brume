# Brume Hardware Design Notes

**Last updated:** 2026-04-17

---

## Reference Platform (current)

- **Module:** Raspberry Pi Compute Module 5 (WiFi SKU), 8 GB RAM, 32 GB eMMC
- **Carrier:** Waveshare CM5 IO Board (PoE variant), USB_OTG jumper fitted, full-size HDMI × 2, USB-A × 2, GbE + PoE, RJ45
- **OS:** Raspberry Pi OS Lite 64-bit (Trixie / Debian 13 base), PREEMPT kernel 6.12.x
- **Cooling:** Active (heatsink + fan), required for sustained builds and audio workloads
- **Display:** MAGEX 10.1" HDMI IPS touchscreen, 1920×1200 10-point capacitive; enclosure case with built-in dual speakers (HDMI audio path) and a mini fan. USB touch controller reports as `TSTP MTouch`, VID:PID `0416:c169`. The Brume UI is designed against a **1024×600 canonical logical resolution**; it auto-fits to any panel at startup via `webview.zoom(min(w/1024, h/600))`. Validated panels: 1024×600 (original WIMAXIT 10.1" with QDtech MPI7003 on the Pi 5 prototype), 1920×1200 (current MAGEX on CM5). Override with `BRUME_UI_SCALE=<f>`.
- **Audio (current):** HDMI stereo via vc4-hdmi (native 48 kHz; cpal `default` handles 44.1 kHz transparently via the ALSA plug layer)
- **Audio (pending):** InnoMaker HiFi DAC Pro Hat (ES9038Q2M) on I²S, providing a stereo 3.5mm TRS line out (a cleaner path than HDMI audio)
- **Touch input:** USB HID multitouch (MT_SLOT protocol B)
- **Compositor:** labwc (Wayland), tty1 autologin, swaybg background
- **SSH:** `ssh brume@brume.local` with passwordless sudo

## Prototype Platform (legacy, retained as reference)

- **Board:** Raspberry Pi 5 Model B Rev 1.0, 4 GB RAM
- **OS:** Raspberry Pi OS Lite (Trixie), PREEMPT kernel 6.12.25
- **Role:** Bench reference unit only; not used for new Brume development. Same 10.1" touchscreen previously bolted here; now moved to the CM5.
- **SSH:** `ssh <user>@<pi5-host>.local`

---

## Memory Budget

Pre-iced figures from a Pi 5 prototype, kept for directional
reference only — the iced + wgpu rewrite collapsed the old
binary-plus-webview split into a single process, so the totals are
overdue for a remeasure on a current CM5 build.

| Component | Pre-iced RSS (Pi 5) |
|-----------|---------------------|
| Brume binary (engine + DSP + FX chain) | 159 MB |
| labwc (Wayland compositor) | 91 MB |

The engine at 159 MB includes all audio buffers, delay lines
(reverb allpass chains, delay line buffers), FX chain state, 24
voice states, and modulation routers. With the WebKit webview
retired the second-largest component is gone; the iced single-
process equivalent has not yet been profiled on the CM5.

---

## Production Hardware: Custom CM5 Carrier Board

The CM5 (Compute Module 5) is the production path: it carries the same BCM2712 chip as the Pi 5 but in a module form factor designed to be mounted on a custom carrier board.

### What Brume needs at the board level

```
┌─────────────────────────────────────────────────┐
│                BRUME CARRIER BOARD               │
│                                                  │
│  ┌──────────┐                                    │
│  │   CM5    │  2GB sufficient, 4GB comfortable   │
│  │  module  │  eMMC for OS (no SD card)          │
│  │          │  WiFi/BT for updates (optional)    │
│  └────┬─────┘                                    │
│       │                                          │
│  ┌────┴─────────────────────────────────────┐    │
│  │              I2S bus                      │    │
│  └────┬─────────────────────────────────────┘    │
│       │                                          │
│  ┌────┴─────┐                                    │
│  │ DAC      │  ES9038Q2M or PCM5102A             │
│  │ (stereo) │  3.5mm TRS line out                │
│  └──────────┘  (headphone amp optional)          │
│                                                  │
│  ┌──────────┐  USB host ports                    │
│  │ USB × 2  │  Port 1: MIDI keyboard             │
│  │ Type-A   │  Port 2: nanoKONTROL2 / hub        │
│  └──────────┘                                    │
│                                                  │
│  ┌──────────┐                                    │
│  │ HDMI     │  to 10.1" touchscreen              │
│  └──────────┘  (or DSI ribbon for custom display)│
│                                                  │
│  ┌──────────┐                                    │
│  │ USB-C    │  power input (5V 3A)               │
│  │ power    │                                    │
│  └──────────┘                                    │
│                                                  │
│  ┌──────────┐  Optional                          │
│  │ DIN MIDI │  5-pin DIN in + out                │
│  │ in/out   │  via UART + optocoupler            │
│  └──────────┘                                    │
│                                                  │
│  ┌──────────┐  Optional                          │
│  │ ADC      │  stereo audio input                │
│  │ (input)  │  for future sampling/FX            │
│  └──────────┘                                    │
│                                                  │
│  Status LED × 2 (power + activity)               │
│  Reset button (recessed)                         │
└─────────────────────────────────────────────────┘
```

### CM5 Migration (Complete, 2026-04-17)

**Hardware in use:**
- Waveshare CM5: CM5 8GB RAM, 32GB eMMC, WiFi SKU, antenna, heatsink
- Waveshare CM5 IO Board (PoE variant), USB_OTG jumper fitted for future gadget mode, full-size HDMI × 2, USB-A × 2, GbE + PoE

**USB Audio Gadget (Overbridge-style):**

Unlike the Pi 5 Model B, the CM5's USB-C port supports OTG/device mode via the DWC2 controller. With `dr_mode=peripheral` and the USB_OTG jumper on the IO board, the CM5 presents itself as a UAC2 + MIDI composite USB gadget to the host computer. The DAW sees "Brume" as an 8-channel audio input plus MIDI device over a single USB-C cable.

```
Brume (CM5)                              Mac (DAW)
───────────                              ──────────
Part 0 (Complex)  → channels 1-2 ─┐
Part 1 (Harmonic) → channels 3-4 ─┼─ USB-C ─→ "Brume" 8-ch audio input
Part 2 (Timbral)  → channels 5-6 ─┤          + stereo master on 7-8
Master mix        → channels 7-8 ─┘          + MIDI in/out
```

**Requirements:**
- `config.txt`: `dtoverlay=dwc2,dr_mode=peripheral` under `[cm5]`
- USB_OTG jumper on IO board J2 pins 9-10 (note: Rev 1 silkscreen is swapped, with pins labeled "SYNC_OUT" actually carrying USB_OTG)
- ConfigFS setup: `usb_f_uac2` + `usb_f_midi` composite gadget via `libcomposite`
- Kernel modules confirmed present: `libcomposite`, `usb_f_uac2`, `usb_f_midi`, `u_audio`

**Latency:** Estimated 4-8ms one-way, 8-16ms round-trip. USB 2.0 isochronous. Not as low as Elektron Overbridge (~1-2ms proprietary protocol) but competitive for standard USB audio.

**Risk:** DWC2 interrupt storm with macOS hosts: high interrupt rates (250K+/sec) observed on CM4. CM5's faster CPU (A76 vs A72) may handle it. Needs testing under sustained audio load.

### Key design choices

**DAC:** The ES9038Q2M (from the InnoMaker hat we're already planning for) is a high-quality option. For cost-reduced production, the PCM5102A is widely used in Pi audio projects: cheaper, still very good, I2S interface. Either way, the DAC goes directly on the carrier board with a proper analog output stage and 3.5mm TRS jack.

**Display connection:** Two options:
- **HDMI** (what we're using now): simple, works with off-the-shelf displays, but bulky connector.
- **DSI ribbon:** thinner, more integrated, direct digital connection. Would allow a custom display assembly where the carrier board mounts directly behind the display panel. This is what Norns does with its OLED.

**Storage:** eMMC soldered to the carrier board instead of an SD card. Faster boot, more reliable (no card to corrupt or fall out), write-leveled. 8-16GB is plenty for the OS + Brume binary + presets.

**DIN MIDI:** Traditional 5-pin DIN MIDI jacks are still used in studios. A UART from the CM5 + an optocoupler circuit = hardware MIDI I/O without USB. This lets Brume connect to vintage gear, Eurorack MIDI-to-CV modules, etc.

**No Ethernet jack:** this is an instrument rather than a server. WiFi (from the CM5) handles updates if needed.

### Form factor concept

The board would be designed to fit behind the 10.1" display, making a self-contained unit:

```
 Side view:
 ┌──────────────────────────┐
 │      10.1" display       │  ← touchscreen
 ├──────────────────────────┤
 │     carrier board + CM5  │  ← ~4mm thick
 └──────────────────────────┘
   │  │  │  │  │
   │  │  │  │  └── USB-C power
   │  │  │  └───── 3.5mm audio out
   │  │  └──────── USB-A × 2 (MIDI)
   │  └─────────── DIN MIDI in/out
   └────────────── (optional audio in)
```

All I/O on the bottom or side edges. The whole thing is maybe 25mm thick total: display, PCB, and enclosure. That's about the thickness of an iPad.

### Production cost estimate (100-500 units)

| Component | Rough cost |
|-----------|-----------|
| CM5 2GB | ~$30-35 |
| Custom carrier PCB (fabrication + assembly) | ~$25-40 |
| DAC (PCM5102A) + analog output stage | ~$5 |
| Connectors (USB, HDMI/DSI, 3.5mm, DIN) | ~$8 |
| eMMC 16GB | ~$8 |
| Power regulation | ~$3 |
| 10.1" touchscreen display | ~$40-60 |
| Enclosure (injection molded at volume) | ~$10-20 |
| **Total BOM** | **~$130-180** |

A retail price of $400-500 would be reasonable for a multi-timbral synth instrument with a touchscreen. For reference: Norns Shield (DIY kit) was $180, Norns (assembled) was $800, Deluge is $1000+.

---

## Appliance Boot Configuration

### Current (live on CM5)

- tty1 autologin enabled via `raspi-config nonint do_boot_behaviour B2` (systemd dropin at `/etc/systemd/system/getty@tty1.service.d/autologin.conf`)
- `~/.bash_profile` on tty1: `exec dbus-run-session -- labwc`
- `~/.config/labwc/autostart`: launches `swaybg -c "#060608"` (dark background). Brume launched on-demand via SSH for now; moves to autostart once UX stabilizes.
- `~/.config/labwc/rc.xml`: minimal config, touch-category libinput block, identity calibration matrix.

### Future (production hardening)

- Suppress labwc default root menu in `rc.xml` (so tapping empty background is a no-op)
- systemd service at `/etc/systemd/system/brume.service` for supervised restart on crash
- Target boot time: power-on → Brume UI visible in < 10 seconds (currently ~23 s to labwc; Brume launch adds ~2-3 s)

### Services to disable for production (post-launch)

| Service | Purpose | Needed by Brume? |
|---------|---------|-----------------|
| bluetooth.service | Bluetooth stack | No |
| avahi-daemon.service | mDNS/DNS-SD | **Keep:** `brume.local` resolution for `brumectl` |
| triggerhappy.service | Hotkey daemon | No |
| udisks2.service | Disk manager | No |
| polkit.service | Authorization manager | No |
| ModemManager.service | Modem management | No |
| NetworkManager.service | Network management | **Keep:** WiFi connection management |
