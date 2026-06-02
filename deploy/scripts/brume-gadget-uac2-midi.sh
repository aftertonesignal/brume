#!/bin/sh
# Brume Meridian USB gadget: UAC2 stereo + MIDI composite function.
#
# Runs on the CM5 to present Brume to the host Mac over USB-C as a
# class-compliant audio + MIDI device. Corresponds to Stage 6a
# (2-channel 48 kHz UAC2 + 16x16 MIDI ports,
# IAD composite, no Feature Unit stripped yet — that's Stage 8).
#
# Prerequisites:
#   - /boot/firmware/config.txt has `dtoverlay=dwc2,dr_mode=peripheral`
#     under `[cm5]`, reboot applied.
#   - USB_OTG jumper on Waveshare CM5 IO Board J2 (silkscreen labeled
#     SYNC_OUT on Rev 1, but actually routes USB_OTG).
#   - `libcomposite`, `usb_f_uac2`, `usb_f_midi`, `u_audio` kernel modules
#     present (they are in the stock Pi kernel).
#
# Usage (idempotent — deletes any existing `brume` gadget first):
#   sudo ~/brume-gadget/uac2-midi.sh
#
# To tear down:
#   sudo sh -c 'echo > /sys/kernel/config/usb_gadget/brume/UDC && \
#     find /sys/kernel/config/usb_gadget/brume/configs/c.1 -type l -delete && \
#     rmdir /sys/kernel/config/usb_gadget/brume/configs/c.1/strings/* \
#           /sys/kernel/config/usb_gadget/brume/configs/c.1 \
#           /sys/kernel/config/usb_gadget/brume/functions/* \
#           /sys/kernel/config/usb_gadget/brume/strings/* \
#           /sys/kernel/config/usb_gadget/brume'

set -eu

CONFIGFS=/sys/kernel/config
GADGET_ROOT="$CONFIGFS/usb_gadget"
GADGET="$GADGET_ROOT/brume"

# --- Preconditions ---------------------------------------------------------

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (sudo)" >&2
    exit 1
fi

if ! mountpoint -q "$CONFIGFS"; then
    mount -t configfs none "$CONFIGFS"
fi

modprobe libcomposite

UDC="$(ls /sys/class/udc 2>/dev/null | head -n1 || true)"
if [ -z "$UDC" ]; then
    echo "error: no UDC in /sys/class/udc" >&2
    echo "  check /boot/firmware/config.txt has dtoverlay=dwc2,dr_mode=peripheral under [cm5]" >&2
    echo "  check USB_OTG jumper fitted on Waveshare CM5 IO Board J2" >&2
    exit 1
fi

# --- Tear down any previous Brume gadget ---------------------------------
#
# Earlier versions swallowed every teardown error with `2>/dev/null
# || true`, which kept the script idempotent on a fresh boot but
# masked a real failure mode: if the gadget was still bound (UDC
# write failed) the subsequent rmdirs silently couldn't progress, the
# closing `rmdir "$GADGET"` also silently failed, and the build phase
# below ran `mkdir -p "$GADGET"` against a half-populated tree. The
# resulting "fresh" gadget inherited stale strings/functions/configs
# and presented to the host as a malformed device.
#
# Replace with a per-step helper that distinguishes "doesn't exist"
# (skip, idempotency) from "exists but couldn't remove" (fail loud).
# The script now refuses to proceed against a stale tree it can't
# clean up; the operator can inspect /sys/kernel/config/usb_gadget/brume
# manually, root-cause why the kernel is holding state, then re-run.

# Remove a configfs entry. Three states: doesn't exist (no-op),
# exists and removes cleanly (return 0), exists but rmdir fails (die
# with diagnostic). The "doesn't exist" case is silent so a normal
# fresh-boot run prints no spurious teardown noise.
gadget_remove() {
    local path="$1"
    if [ ! -e "$path" ] && [ ! -L "$path" ]; then
        return 0
    fi
    if [ -L "$path" ]; then
        rm -f "$path" || {
            echo "error: failed to remove symlink $path" >&2
            exit 1
        }
    elif [ -d "$path" ]; then
        rmdir "$path" || {
            echo "error: failed to rmdir $path (gadget may still be bound; check 'cat $GADGET/UDC')" >&2
            exit 1
        }
    elif [ -f "$path" ]; then
        rm -f "$path" || {
            echo "error: failed to remove file $path" >&2
            exit 1
        }
    fi
}

if [ -d "$GADGET" ]; then
    # Unbind from UDC first. The UDC sysfs file expects a write of
    # empty string (NOT a remove). If we can't write, the gadget is
    # still bound and the subsequent rmdirs would all fail anyway —
    # surface the real error here.
    if [ -f "$GADGET/UDC" ]; then
        if ! echo "" > "$GADGET/UDC" 2>/dev/null; then
            # If UDC was already empty (gadget already unbound), the
            # write may report ENODEV or similar; check the contents
            # to distinguish "already unbound" from "bound and won't
            # release."
            current_udc="$(cat "$GADGET/UDC" 2>/dev/null || true)"
            if [ -n "$current_udc" ]; then
                echo "error: failed to unbind gadget from UDC '$current_udc'" >&2
                echo "  the kernel is holding the gadget bound; check dmesg" >&2
                exit 1
            fi
        fi
    fi

    # Remove config function links (e.g. configs/c.1/uac2.0).
    # `set -u` would barf on the unexpanded glob if no matches, so
    # iterate explicitly and skip when the path isn't a symlink.
    for f in "$GADGET"/configs/*/*.*; do
        [ -L "$f" ] && rm -f "$f"
    done

    # Strings, configs, functions, gadget — bottom-up. Each step
    # fails loud if the directory exists but rmdir refuses.
    for d in "$GADGET"/configs/*/strings/*; do
        gadget_remove "$d"
    done
    for d in "$GADGET"/configs/*; do
        gadget_remove "$d"
    done
    for d in "$GADGET"/functions/*; do
        gadget_remove "$d"
    done
    for d in "$GADGET"/strings/*; do
        gadget_remove "$d"
    done
    gadget_remove "$GADGET"
fi

# --- Build composite gadget ------------------------------------------------

mkdir -p "$GADGET"
cd "$GADGET"

# USB VID/PID.
#
# Dev default: Linux Foundation Multifunction Composite Gadget IDs
# (`0x1d6b:0x0104`). Class-compliant, works on macOS/Windows/Linux without
# any vendor binding — fine for the class-compliant audio path.
#
# Ship / Meridian DEXT target: pid.codes sub-PID under VID 0x1209 (open-
# source community VID). Required for the macOS DEXT match dictionary to
# reliably claim *our* device (not some other composite gadget).
#
# Override via env at boot: BRUME_USB_VID and BRUME_USB_PID. Defaults
# below ride on the Linux Foundation dummy. The earlier pid.codes
# sub-PID registration (1209/B17E) was withdrawn 2026-04-28; USB
# identity strategy is open for now.
: "${BRUME_USB_VID:=0x1d6b}"
: "${BRUME_USB_PID:=0x0104}"
echo "$BRUME_USB_VID" > idVendor
echo "$BRUME_USB_PID" > idProduct
echo 0x0100 > bcdDevice
echo 0x0200 > bcdUSB

# Interface Association Descriptor composite class.
# Apple's UAC2 driver still splits the device into per-direction HAL objects,
# but this is the correct IAD signature for multifunction gadgets.
echo 0xEF > bDeviceClass
echo 0x02 > bDeviceSubClass
echo 0x01 > bDeviceProtocol

# English strings (serial number derived from CM5 die-unique serial, if
# available — lets the Mac distinguish multiple Brume units later).
#
# A version prefix forces macOS CoreMIDI to treat descriptor changes (e.g.
# the 16-port → 1-port MIDI reshape) as a new device, so old cached
# entities don't persist as ghost entries in MIDI Studio. Bump `-vN` when
# the USB-visible descriptor shape changes meaningfully.
mkdir -p strings/0x409
SERIAL="brume-v5-$(cat /sys/firmware/devicetree/base/serial-number 2>/dev/null | tr -d '\0' | head -c 16 || date +%s)"
echo "$SERIAL" > strings/0x409/serialnumber
echo "Aftertone" > strings/0x409/manufacturer
echo "Brume" > strings/0x409/product

# Single configuration containing both functions
mkdir -p configs/c.1/strings/0x409
echo "Meridian: audio + midi" > configs/c.1/strings/0x409/configuration
echo 250 > configs/c.1/MaxPower

# --- UAC2 function (stereo 48 kHz capture + playback) --------------------

mkdir -p functions/uac2.0
echo 48000 > functions/uac2.0/c_srate
echo 48000 > functions/uac2.0/p_srate
# c_chmask / p_chmask are BITMASKS, not channel counts. Each bit
# selects a spatial-position channel per the UAC2 class spec (bit 0 = FL,
# bit 1 = FR, 2 = FC, 3 = LFE, 4 = BL, 5 = BR, 6 = FLC, 7 = FRC, ...).
#
# Capture (Brume → Mac): Stage 5 exposes 8 channels for per-part stems
# (Complex 1-2, Harmonic 3-4, Timbral 5-6, Granular 7-8). Bitmask 0xFF = 255
# selects the first 8 spatial positions. We don't use the positional
# semantics — the Mac's UAC2 driver always labels by direction + alt-status
# anyway — but u_audio requires the mask to be a valid non-zero UAC2 set.
#
# Playback (Mac → Brume): mirror the capture mask at 255. Brume has no
# audio-input processing and doesn't consume incoming data, but the Pi
# kernel (6.12.75) u_audio driver caps the ALSA PCM channel count on
# this gadget at min(c_chmask_count, p_chmask_count) when both directions
# are configured — asymmetric masks (e.g. c=255, p=3) collapse the
# capture side down to 2 channels. Keeping p_chmask=255 pairs cleanly and
# lets the full 8-channel capture come through.
#
# We tried p_chmask=0 (disable playback entirely) and it broke the capture
# side: u_audio removed pcm0p from the gadget, leaving no output PCM for
# Brume to write to. Matching both masks is the reliable workaround on
# this kernel version. Mac-side cost: Brume appears as both an 8-in AND
# an 8-out device, but since we don't route anything into the output
# side, it's effectively a benign label artifact.
echo 255 > functions/uac2.0/c_chmask
echo 255 > functions/uac2.0/p_chmask

# Per-channel labels — written into the Input Terminal's iChannelNames
# string descriptor by u_audio. Apple's class-compliant UAC2 driver
# reads this string and shows custom labels in DAW input pickers + Audio
# MIDI Setup's channel column, instead of the default UAC2 spatial
# positions (Front Left, Front Right, LFE, Back Left, etc.).
#
# Format: single tab-separated string, one name per channel in
# bitmask order. Mechanism documented at
# developer.apple.com/forums/thread/814790 — Apple reads the string
# as authored; u_audio wires c_it_ch_name → iChannelNames on the
# Input Terminal descriptor via f_uac2.c's afunc_bind().
#
# Caveat: confirmed working on macOS ≤ 15; reported broken on
# macOS 26 with no Apple response. Worst case the labels fall back
# to the spatial defaults — setting these can't make things worse.
printf 'Complex L\tComplex R\tHarmonic L\tHarmonic R\tTimbral L\tTimbral R\tGranular L\tGranular R' \
  > functions/uac2.0/c_it_ch_name
printf 'Complex L\tComplex R\tHarmonic L\tHarmonic R\tTimbral L\tTimbral R\tGranular L\tGranular R' \
  > functions/uac2.0/p_it_ch_name
# Stage 8 (usb-audio-gadget.md) will strip the Feature Unit for bit-perfect
# passthrough. For now we leave defaults so the Mac sees a volume fader.

# --- MIDI function (single USB cable pair) --------------------------------
# One in + one out port. Brume's internal MIDI routing uses MIDI channels
# within a single cable (CH 1→Complex, 2→Harmonic, 3→Timbral, 4→Granular,
# plus clock on any channel). More ports would give macOS CoreMIDI no
# extra semantic value and would cause DAW MIDI pickers to list "Brume
# Port 1"..."Brume Port 16" as separate entities, drowning the UI.

mkdir -p functions/midi.0
echo 1 > functions/midi.0/in_ports
echo 1 > functions/midi.0/out_ports
echo "Brume MIDI" > functions/midi.0/id

# --- Bind functions + activate ---------------------------------------------

ln -sf functions/uac2.0 configs/c.1/
ln -sf functions/midi.0 configs/c.1/

echo "$UDC" > UDC

echo "brume-gadget: bound to UDC $UDC as Aftertone Brume (UAC2 + MIDI)" >&2
