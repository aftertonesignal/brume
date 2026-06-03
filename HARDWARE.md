# Brume Hardware

Brume's reference platform and the hardware-level requirements for its
USB audio + MIDI bridge (Meridian). The software targets the stock
Raspberry Pi kernel and ALSA, so it isn't tied to one IO board or
panel; reports from other hardware are welcome. The exact parts we
used, with purchase links, are in [BOM.md](BOM.md).

## Reference platform

- **Module:** Raspberry Pi Compute Module 5 (WiFi SKU), 8 GB RAM, 32 GB eMMC
- **Carrier:** a CM5 IO board with the **USB_OTG jumper fitted** — the reference unit is a Waveshare CM5 IO Board (PoE variant). The OTG jumper is what enables USB device mode, which Meridian needs.
- **OS:** Raspberry Pi OS Lite 64-bit (Trixie / Debian 13), PREEMPT kernel 6.12.x
- **Cooling:** active (heatsink + fan), for sustained audio workloads
- **Display:** 10.1" HDMI IPS touchscreen, 1920×1200, 10-point capacitive. The UI is designed against a 1024×600 canonical logical resolution and auto-fits to whatever panel is connected at startup; override with `BRUME_UI_SCALE=<f>`.
- **Touch input:** USB HID multitouch (MT_SLOT protocol B)
- **Compositor:** labwc (Wayland), tty1 autologin

## Audio + MIDI: Meridian (USB gadget)

The CM5's USB controller supports device (peripheral) mode via DWC2.
With the OTG jumper fitted and the dwc2 peripheral overlay enabled, the
CM5 presents itself to a host computer as a class-compliant UAC2 audio
+ MIDI composite gadget over a single USB cable — no drivers needed.
Connected this way, Brume exposes eight audio channels (a stereo stem
per part) plus MIDI in and out:

```
Brume (CM5)                          Host (DAW)
───────────                          ──────────
Part 0 (FM)       → channels 1-2 ┐
Part 1 (Harmonic) → channels 3-4 ┤   USB → "Brume" 8-ch input
Part 2 (Timbral)  → channels 5-6 ┤        + MIDI in/out
Part 3 (Granular) → channels 7-8 ┘
```

Without a USB gadget connection, Brume falls back to stereo (e.g. HDMI
audio). Latency over USB 2.0 isochronous is roughly 4–8 ms one-way.

### Requirements

- `/boot/firmware/config.txt`: `dtoverlay=dwc2,dr_mode=peripheral` under a `[cm5]` section. `brumectl install` adds this automatically when it detects a CM5.
- The **USB_OTG jumper** fitted on the IO board. On the Waveshare CM5 IO Board this is J2 pins 9-10 — note the Rev 1 silkscreen labels these "SYNC_OUT".
- Kernel modules (present in stock Pi OS): `libcomposite`, `usb_f_uac2`, `usb_f_midi`, `u_audio`.
