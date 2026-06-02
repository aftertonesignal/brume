# Deploy: Brume on CM5

**Reference platform:** CM5 + Waveshare IO Board (Pi OS Lite Trixie, labwc). The Pi 5 prototype is retained untouched as reference; all new development targets the CM5.

This doc is the *dev-loop and deploy* reference: how to build, sync source to a CM5, restart the running service, troubleshoot common gotchas. For *release* procedure (cutting a tagged release, running CI, attaching artifacts) see `BUILD_AND_RELEASE.md` (or `RELEASING.md` once that's published).

## Target Hardware

- **Board**: Raspberry Pi Compute Module 5 (WiFi SKU), 8 GB RAM, 32 GB eMMC, on a Waveshare CM5 IO Board (PoE variant)
- **OS**: Raspberry Pi OS Lite 64-bit (Trixie / Debian 13 base), PREEMPT kernel 6.12.x
- **CPU**: Quad-core ARM Cortex-A76 @ 2.4 GHz, aarch64
- **Display**: WIMAXIT 10.1" capacitive touchscreen, 1024×600, full-size HDMI + USB touch (QDtech MPI7003)
- **Audio**: HDMI out via ALSA (`card 0: vc4-hdmi-0`, native 48 kHz). cpal opens `default` and transparent resampling handles 44.1 kHz
- **MIDI**: USB class-compliant via `midir`
- **Compositor**: labwc (Wayland), autologin on tty1, dbus-run-session

## SSH Access

```bash
ssh brume@brume.local
```

The `brume` user has passwordless sudo. Hostname `brume.local` resolves via mDNS over both WiFi and ethernet. If your SSH key isn't in the agent / `~/.ssh/config`, pass `-i <path>` or set `BRUME_SSH_KEY`.

## Prerequisites on CM5

These are installed on the working device. For a fresh setup:

```bash
sudo apt install -y \
  build-essential pkg-config \
  libasound2-dev \
  labwc seatd swaybg grim xwayland libinput-bin \
  dbus-user-session

# Rust toolchain (pinned to 1.87 per rust-toolchain.toml)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.87.0 --profile minimal
source ~/.cargo/env

# Disable PipeWire MIDI (prevents ALSA device conflict with midir)
# Only relevant if you install pipewire/wireplumber; Pi OS Lite ships without them by default.
mkdir -p ~/.config/wireplumber/wireplumber.conf.d
cat > ~/.config/wireplumber/wireplumber.conf.d/50-disable-midi.conf << 'EOF'
monitor.alsa.midi = false
EOF
systemctl --user restart wireplumber 2>/dev/null || true

# ALSA hardening: keep USB audio devices (iConnectMIDI4+, etc.) off card 0 and
# route default through a plug device pointed at vc4-hdmi-0 by name.
# Without this, plugging in the iConnect renumbers cards and Brume's audio
# goes into the USB device's line-out instead of the touchscreen's HDMI.
sudo tee /etc/modprobe.d/alsa-base.conf > /dev/null <<'EOF'
options snd-usb-audio index=-2
EOF
sudo tee /etc/asound.conf > /dev/null <<'EOF'
# Route ALSA default through plug → UAC2 gadget capture endpoint.
# Brume writes here; audio is delivered over USB-C to the host Mac
# as a class-compliant audio input ("Brume" aggregate via Audio MIDI Setup).
#
# Alternative: point at "hw:vc4hdmi0,0" for standalone use with HDMI-speaker
# monitoring when no DAW is tethered.
pcm.!default {
    type plug
    slave.pcm "hw:UAC2Gadget,0"
}
ctl.!default {
    type hw
    card UAC2Gadget
}
EOF

# Autologin + labwc on tty1 for appliance mode
sudo raspi-config nonint do_boot_behaviour B2
mkdir -p ~/.config/labwc ~/.config/systemd/user
cat > ~/.bash_profile << 'EOF'
if [ "$XDG_VTNR" = "1" ] && [ -z "$WAYLAND_DISPLAY" ]; then
  export XDG_RUNTIME_DIR=/run/user/$(id -u)
  exec labwc
fi
EOF
cat > ~/.config/labwc/autostart << 'EOF'
swaybg -c "#060608" &
systemctl --user start brume.service
EOF
chmod +x ~/.config/labwc/autostart

# labwc environment: forces the wlroots GL/ES2 renderer + xkb layout.
# Without WLR_RENDERER=gles2 on Pi OS Lite Trixie, wlroots sometimes
# auto-selects the Pixman software renderer and pegs a CPU core at
# ~80% compositing a fullscreen 1920×1200 webview surface. Forcing
# gles2 drops labwc to ~50% and uses the VC4/V3D GPU for composite.
# XCURSOR_THEME is set by deploy/scripts/install-transparent-cursor.sh;
# we add it here for documentation. Touch-only mode can skip it.
cat > ~/.config/labwc/environment << 'EOF'
WLR_RENDERER=gles2
XKB_DEFAULT_MODEL=pc105
XKB_DEFAULT_LAYOUT=us
XKB_DEFAULT_VARIANT=
XKB_DEFAULT_OPTIONS=
XCURSOR_THEME=brume-transparent
EOF

# Brume as a systemd user service (launched by labwc autostart)
cat > ~/.config/systemd/user/brume.service << 'EOF'
[Unit]
Description=Brume synthesizer
After=default.target

[Service]
Type=simple
Environment=WAYLAND_DISPLAY=wayland-0
Environment=XDG_RUNTIME_DIR=/run/user/%U
WorkingDirectory=%h/brume
ExecStart=%h/brume/target/release/brume
Restart=on-failure
RestartSec=3
StandardOutput=append:%t/brume.log
StandardError=inherit

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload

# Passwordless sudo for ongoing ops
echo "brume ALL=(ALL) NOPASSWD:ALL" | sudo tee /etc/sudoers.d/010_brume-nopasswd > /dev/null
sudo chmod 440 /etc/sudoers.d/010_brume-nopasswd

# USB Audio Gadget (presents Brume to the Mac DAW as a class-compliant audio device)
# Requires /boot/firmware/config.txt to have [cm5] dtoverlay=dwc2,dr_mode=peripheral
sudo sed -i 's/^dtoverlay=dwc2,dr_mode=host/dtoverlay=dwc2,dr_mode=peripheral/' /boot/firmware/config.txt
mkdir -p ~/brume-gadget
# Copy the canonical setup script from the repo:
#   scp deploy/scripts/brume-gadget-uac2-midi.sh brume@brume.local:~/brume-gadget/uac2-midi.sh
#   chmod +x ~/brume-gadget/uac2-midi.sh
# ~/brume-gadget/uac2-midi.sh: run after boot to bring up the gadget:
#   sudo ~/brume-gadget/uac2-midi.sh
# or autostart via systemd (future follow-up).
```

## On-device Layout

```
/usr/bin/brume            # Cross-compiled binary, swapped in by deploy-cm5.sh
~/brume/scripts/          # Lua scripts — control scripts, FX, studies
~/.brume/library/         # Runtime preset storage (created on first run)
```

There's no source tree on the device anymore. `brumectl scripts push` and
`scripts/deploy-cm5.sh` cover the two write paths in.

## Step 1: Run Tests Locally (macOS)

Before deploying, always verify on the dev machine:

```bash
cd /path/to/brume
cargo test --workspace
```

All tests must pass.

## Step 2: Cross-compile on the workstation

The dev loop is **cross-compile from the workstation, scp the binary**. The CM5 never compiles anything — the heavy lifting (iced, wgpu, alsa-sys, mlua…) stays on the dev machine, so the device's 8 GB RAM is never under build pressure while a working Brume is also running. A clean release build is ~60 s on Apple Silicon; incremental ~3 s.

Earlier prototypes built on the device. That worked for tiny incremental rebuilds (~30 s) but a public-trait change in a low-level crate could force a full `ui-native` recompile that ran 12 + minutes and once tipped the device into a hard freeze. Cross-compile takes that risk off the table entirely.

### One-time setup (~5 min, ~1.2 GB)

```bash
# 1. Rust target + zig-based cross linker
rustup target add aarch64-unknown-linux-gnu
brew install zig
cargo install cargo-zigbuild

# 2. Pull the device's aarch64 sysroot. alsa-sys + a few other build.rs
#    scripts run pkg-config at compile time, so they need ALSA and friends'
#    headers + .pc files. The CM5's own /usr/include + /usr/lib/aarch64-linux-gnu
#    is the most accurate sysroot — it matches the runtime libs by definition.
SYSROOT=~/.local/share/brume-sysroot/aarch64-debian-trixie
mkdir -p "$SYSROOT/usr/lib/aarch64-linux-gnu"
rsync -a brume@brume.local:/usr/include/                      "$SYSROOT/usr/include/"
rsync -a brume@brume.local:/usr/lib/aarch64-linux-gnu/        "$SYSROOT/usr/lib/aarch64-linux-gnu/"
```

### Build + deploy (every iteration)

```bash
scripts/deploy-cm5.sh
```

That wraps `cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.41 -p brume-main`, scps the binary to `/tmp/brume.new` on the device, atomically swaps it into `/usr/bin/brume` (with `cap_sys_nice=eip` re-applied via `setcap` — see "audio capability" below), and restarts `brume.service`. The `.2.41` glibc pin matches Trixie's runtime so the linker doesn't reach for symbols newer than what the device ships. See `scripts/deploy-cm5.sh` for env-var overrides (host, user, key, sysroot path, target triple).

**Audio capability — load-bearing.** Brume's audio thread elevates itself to `SCHED_FIFO` rtprio 80 at first cpal callback so the iced GUI can't preempt it. That elevation requires `CAP_SYS_NICE` on the binary's xattrs. `cp` strips file caps by default, so a naive `sudo cp /tmp/brume.new /usr/bin/brume` wipes the capability and the next start logs `failed to set SCHED_FIFO (OS(1))`, runs `SCHED_OTHER`, and audibly crackles under sustained MIDI load. `deploy-cm5.sh` re-applies the cap on every deploy via the staging pattern (`install` → `setcap` → atomic `mv`); the device needs `libcap2-bin` installed (`sudo apt install libcap2-bin`) for `setcap` to be available.

`--no-restart` skips the service swap if you only want the binary on the device:

```bash
scripts/deploy-cm5.sh --no-restart
```

## Step 3: Launch / Restart Brume

`deploy-cm5.sh` already restarts the service. If you need to trigger a restart by hand (e.g. after editing scripts in `~/brume/scripts/` on the device):

```bash
ssh brume@brume.local "systemctl --user restart brume.service"
```

Service file at `~/.config/systemd/user/brume.service` (see Prerequisites). On boot, `labwc` is exec'd from `~/.bash_profile` on tty1, and `~/.config/labwc/autostart` calls `systemctl --user start brume.service` once the Wayland display is up. Restart-on-failure is built in (`Restart=on-failure`, `RestartSec=3`).

### Ad-hoc launch (debug fallback)

Useful when iterating without going through the service:

```bash
ssh brume@brume.local \
  "systemctl --user stop brume.service; sleep 1; \
   WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
   nohup /usr/bin/brume > /tmp/brume.log 2>&1 &"
```

### Status + logs

```bash
ssh brume@brume.local \
  "systemctl --user status brume.service --no-pager && tail /run/user/1000/brume.log"
```

## Taking Screenshots

labwc + grim on `wayland-0`:

```bash
ssh brume@brume.local \
  "WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 grim /tmp/screenshot.png"

scp brume@brume.local:/tmp/screenshot.png ~/Desktop/brume-screenshot.png
```

## Viewing Logs

```bash
ssh brume@brume.local "cat /tmp/brume.log"
# or live:
ssh brume@brume.local "tail -f /tmp/brume.log"
```

## Thermals & Throttle Check

```bash
ssh brume@brume.local "vcgencmd measure_temp; vcgencmd get_throttled"
```

`throttled=0x0` is clean. Low bits (0x1-0xf) = currently throttling; high bits (0x10000-0xf0000) = throttling has occurred since last boot.

## Troubleshooting

### `deploy-cm5: sysroot not found`
One-time pull from §Step 2. Or set `BRUME_SYSROOT` if you keep one elsewhere.

### `pkg-config has not been configured to support cross-compilation`
A build.rs is reaching for system libs without the sysroot env vars. `deploy-cm5.sh` sets `PKG_CONFIG_SYSROOT_DIR` / `PKG_CONFIG_PATH` / `PKG_CONFIG_ALLOW_CROSS=1`; running `cargo zigbuild` directly without those will fail this way.

### `ld.lld: error: undefined reference: dlsym@GLIBC_2.X`
The glibc pin in `BRUME_TARGET` is older than the device's runtime. Run `ldd --version` on the CM5; pin `BRUME_TARGET=aarch64-unknown-linux-gnu.<that-version>`.

### `package ID specification 'brume' did not match any packages`
Use `-p brume-main` (package name), not `-p brume` (binary name).

### Audio crackling at all gain levels / `failed to set SCHED_FIFO (OS(1))`
The binary's `cap_sys_nice` xattr was stripped — usually because something other than `deploy-cm5.sh` swapped `/usr/bin/brume` (e.g. a manual `sudo cp`). Verify with `getcap /usr/bin/brume` (should print `cap_sys_nice=eip`); restore with `sudo setcap cap_sys_nice=eip /usr/bin/brume && systemctl --user restart brume.service`. Re-deploying via `scripts/deploy-cm5.sh` also restores it. If `setcap`/`getcap` aren't on the device, `sudo apt install libcap2-bin`.

### MIDI device not detected
Check that PipeWire MIDI is disabled (`50-disable-midi.conf`). List devices: `amidi -l`.

### No audio output
ALSA: `aplay -l` to list devices. vc4-hdmi-0 is native 48 kHz; cpal handles 44.1 kHz through the plug layer via the `default` sink. Don't open `hw:vc4hdmi0,0` directly with S16_LE/44100; use `default` or `plughw:`.

Check which card Brume is actually writing to:
```bash
for cn in 0 1 2; do cat /proc/asound/card$cn/pcm0p/sub0/status 2>/dev/null | grep -E "^state|owner"; echo "  card $cn"; done
```
The card with `state: RUNNING` owned by the Brume pid is where audio is going. If that's not vc4-hdmi-0, see the next entry.

### Brume "plays" but touchscreen is silent (audio hijack by USB device)
Symptom: `brume.log` says "UI ready", PCM state is RUNNING, but touchscreen is mute. Cause: `snd-usb-audio` (iConnectMIDI4+ or any USB audio-class device) claimed card 0 at boot, pushing vc4-hdmi-0 to a higher index. Brume's `default` → `hw:0,0` now routes into the USB device's nowhere-connected line-out.

Verify: `aplay -l | grep ^card`. Card 0 must be `vc4hdmi0`. If not, confirm the hardening is present:
```bash
cat /etc/modprobe.d/alsa-base.conf  # should have: options snd-usb-audio index=-2
cat /etc/asound.conf                # default should point to hw:UAC2Gadget,0 (DAW) or hw:vc4hdmi0,0 (standalone)
```
If both are correct but card 0 is still wrong, reboot so modprobe re-reads the options.

### Brume audio not reaching the DAW (UAC2 gadget path)
Symptom: tone test works, but Brume's live output plays through HDMI speakers instead of showing up in the Bitwig track. Cause: `/etc/asound.conf` is pointing at `hw:vc4hdmi0,0` (HDMI) rather than `hw:UAC2Gadget,0` (gadget).

Fix:
```bash
sudo sed -i 's|hw:vc4hdmi0,0|hw:UAC2Gadget,0|; s|card vc4hdmi0|card UAC2Gadget|' /etc/asound.conf
systemctl --user restart brume.service
```

Verify with `cat /proc/asound/card*/pcm0p/sub0/status`: the card with `state: RUNNING` and owner = Brume pid should be the `UAC2Gadget` card rather than `vc4hdmi0`.

### UAC2 gadget not enumerated on the Mac
Symptom: macOS sees no new USB device after plugging the CM5's USB-C in. Cause could be:
- `config.txt` still has `dr_mode=host` under `[cm5]`; change to `dr_mode=peripheral` and reboot.
- The `USB_OTG` jumper on the IO board isn't fitted; short that header.
- The gadget script hasn't been run; `sudo ~/brume-gadget/uac2-midi.sh`.
- The cable is charge-only; swap for a known-data USB-C cable.
- Physical replug needed; macOS is conservative about re-enumerating after a role change, so unplug and plug back in.

### UI doesn't appear (SSH launch)
The webview needs `WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000` and labwc running on tty1. If labwc isn't up, check `getty@tty1` autologin and `~/.bash_profile`.

### Autostart from labwc doesn't launch Brume
`systemctl --user start brume.service` in `~/.config/labwc/autostart` silently fails if the dbus env points at the wrong bus. This happens if `~/.bash_profile` wraps labwc with `dbus-run-session`, which creates a short-lived `/tmp/dbus-XXX` bus. Fix: `exec labwc` (no wrapper) so the standard user dbus at `$XDG_RUNTIME_DIR/bus` is used.

### labwc root menu pops up on touch of empty background
Expected default labwc behaviour: the menu disappears once Brume draws fullscreen. Suppress in `~/.config/labwc/rc.xml` for pure appliance mode if needed.
