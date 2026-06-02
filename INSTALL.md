# Install Brume

Get Brume running on your CM5 in about 5 minutes once the
substrate is in place.

Brume is a Rust synthesizer with an iced + wgpu UI. It runs on Linux
with ALSA available at runtime. The reference platform (and the
only one with appliance-mode bring-up guaranteed) is a
**Raspberry Pi Compute Module 5** on a CM5 IO board running
**Raspberry Pi OS Lite (64-bit, Trixie)**.

---

## Precondition

Brume installs on top of an existing, working Pi OS Lite. Specifically:

- A CM5 booted on Pi OS Lite (64-bit, Trixie).
- The user account is named `brume`, with passwordless sudo and your
  SSH public key in `~/.ssh/authorized_keys` (Pi OS's default user
  setup gets you both).
- The device is on your network and reachable as `brume.local` (mDNS)
  or by IP.

How you get the CM5 to that state is out of scope for this guide.
The Raspberry Pi Foundation's
[getting-started documentation](https://www.raspberrypi.com/documentation/computers/getting-started.html)
covers OS install, customization, hostname, SSH keys, WiFi, locale,
and regulatory domain. Once `ssh brume@brume.local` works from your
controller machine, you're ready for the steps below.

---

## What you'll need

**Hardware:**

- Raspberry Pi Compute Module 5 (WiFi SKU recommended)
- CM5 IO board: any carrier that breaks out HDMI, USB, and (for Meridian) `USB_OTG` in peripheral mode
- HDMI display, ideally touchscreen (we use a 10.1" 1920×1200 panel)

**On your controller machine** (Mac, Linux, or WSL):

- An SSH key with its public half on the CM5 (we recommend a
  project-specific one, e.g. `~/.ssh/id_brume_cm5`).
- `brumectl`, the companion CLI. Build from source with
  `cargo build --release -p brumectl`, or download a tagged release
  from [GitHub Releases](https://github.com/aftertonesignal/brume/releases)
  (macOS arm64 and Linux x86_64 binaries are published with every
  tag; the resulting binary goes anywhere on your `$PATH`).

---

## From boot to Brume

`brumectl install` turns a working Pi OS Lite into a Brume
appliance over SSH. Two steps.

### Step 1. Verify the CM5 is reachable

```sh
ping -c 3 brume.local
```

If that resolves, move on. If not: see [Troubleshooting](#troubleshooting).

### Step 2. Run `brumectl install`

```sh
brumectl -i ~/.ssh/id_brume_cm5 install
```

What it does, in six phases:

1. **Probe:** verifies SSH works, user has passwordless sudo, arch is
   aarch64, OS is Debian-like.
2. **apt:** installs runtime deps: `labwc`, `seatd`, `swaybg`,
   `grim`, `xwayland`, `dbus-user-session`, `alsa-utils`, and the
   corresponding EGL/GLES/font packages.
3. **Binary:** copies the cross-built `brume` aarch64 ELF to
   `/usr/bin/brume` (root:root, 0755).
4. **Configs:** writes the appliance-mode setup: `~/.bash_profile`,
   `~/.config/labwc/autostart`, `~/.config/labwc/environment`,
   `~/.config/systemd/user/brume.service`, `/etc/asound.conf`,
   `/etc/systemd/system/getty@tty1.service.d/autologin.conf`, and the
   transparent-cursor Xcursor theme.
5. **Autologin:** `raspi-config nonint do_boot_behaviour B2` so tty1
   auto-logs-in as `brume`, who `.bash_profile` hands off to labwc.
6. **Verify:** confirms `brume.service` is enabled and
   `/usr/bin/brume` is present.

Reboot when it prompts you:

```sh
ssh brume@brume.local sudo reboot
```

### Step 3. Brume is live

The CM5 boots to: kernel → systemd → tty1 autologin → labwc → swaybg
black bg → brume.service starts → the UI appears on the HDMI panel
with sound and touch working.

SSH access persists: `ssh brume@brume.local` for maintenance.

---

## Updating Brume

Once installed, pushing a new Brume release is one command:

```sh
brumectl -i ~/.ssh/id_brume_cm5 install --update
```

`--update` skips apt + configs + autologin and just scps the new
`/usr/bin/brume` and restarts `brume.service`. End-to-end ~5 seconds.
No reboot needed.

Full re-install (overwrite configs as well) is just
`brumectl install` without `--update`.

---

## Troubleshooting

### `brume.local` doesn't resolve

The CM5 is either off the network or its mDNS responder (`avahi-daemon`)
isn't running. Try:

- Ping the CM5 directly if you know its IP (from your router admin page).
- Pass `--host brume@<IP>` explicitly:
  `brumectl --host brume@192.168.1.42 install`.
- On the CM5 console: `sudo systemctl status avahi-daemon`.

### SSH to brume@brume.local fails: the identity ssh tried did not authenticate

Your SSH key isn't being offered to the CM5. Try:

- `ssh-add ~/.ssh/id_brume_cm5` to load the key into your local agent.
- `export BRUME_SSH_KEY=~/.ssh/id_brume_cm5` for the session.
- `brumectl -i ~/.ssh/id_brume_cm5 install` per invocation.
- Add to `~/.ssh/config`:
  ```
  Host brume.local
    User brume
    IdentityFile ~/.ssh/id_brume_cm5
  ```

### Brume starts but no sound

Check ALSA card enumeration: `aplay -l` on the CM5 should list
`vc4-hdmi-0` as card 0. `/etc/asound.conf` routes ALSA default to
that card. If you've plugged in a USB audio device, `/etc/modprobe.d/alsa-base.conf`
(set by the install) keeps it off card 0 so it doesn't steal
Brume's output.

### brumectl can't find the brume binary

By default, `brumectl install` looks for the brume binary in this order:

1. `--binary <PATH>` (CLI flag)
2. `$BRUME_BINARY` environment variable
3. Local workspace `target/aarch64-unknown-linux-gnu/release/brume`
4. User cache (`~/.cache/brumectl/binaries/...` on Linux,
   `~/Library/Caches/brumectl/binaries/...` on macOS)
5. **Auto-fetch from GitHub Releases** — downloads the matching
   `brume-aarch64-unknown-linux-gnu` artifact for the brumectl
   version you're running, verifies the SHA-256 against the sidecar
   `release.yml` publishes, and caches the result. No further input
   needed.

If you'd rather not rely on the auto-fetch (offline install, strict
CI, etc.), pass `--no-download` and use one of the earlier options
to provide the binary explicitly. To re-fetch a stale cache (e.g. a
release was re-cut under the same tag), pass `--force-download`.

Building the brume binary locally is also fine if you have the
cross-build toolchain installed:

```
cargo build --release -p brume-main --target aarch64-unknown-linux-gnu
```

The local build wins over the auto-fetch — useful when iterating on
synth code.

---

## Contributing

See [DEPLOY.md](DEPLOY.md) for the dev loop details.
