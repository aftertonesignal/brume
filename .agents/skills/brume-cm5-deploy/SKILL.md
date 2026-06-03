---
name: brume-cm5-deploy
description: >
  Deploying a Brume change to a Raspberry Pi CM5 for live testing.
  Cross-compiles from the maintainer's workstation, scps the binary
  to `/usr/bin/brume`, restarts the service. The CM5 never compiles.
  Documents the one-time sysroot pull, the deploy-cm5.sh wrapper,
  and how to recover when something goes sideways. Triggers on
  "deploy", "cm5 deploy", "deploy to device", "ssh to brume",
  "build for cm5", "test on cm5", "install brume on cm5", "restart
  brume service", "iterate on cm5", "brume.service".
---

# Deploying a change to a CM5

The development loop on Brume is: edit on the dev host,
**cross-compile aarch64 from the workstation**, scp the binary to
the device, restart the service. The CM5 never runs cargo. Round
trips are seconds; clean rebuilds are ~60 s on Apple Silicon.

This skill is the procedural form of `DEPLOY.md`. Read this for
"how do I deploy"; read `DEPLOY.md` for the broader context
including network setup, troubleshooting, and the rationale for
moving builds off the device.

## One-time setup

These run once on the workstation. After that every iteration is
the single `scripts/deploy-cm5.sh` command.

```sh
# 1. Rust target + zig-based cross linker
rustup target add aarch64-unknown-linux-gnu
brew install zig                # macOS; pacman/apt on Linux
cargo install cargo-zigbuild

# 2. Pull the device's aarch64 sysroot. alsa-sys + a few other
# build.rs scripts run pkg-config at compile time, so they need
# ALSA / wayland / etc. headers + .pc files. The CM5's own
# /usr/include + /usr/lib/aarch64-linux-gnu is the most accurate
# sysroot — its libs match the runtime exactly. ~1.2 GB.
SYSROOT=~/.local/share/brume-sysroot/aarch64-debian-trixie
mkdir -p "$SYSROOT/usr/lib/aarch64-linux-gnu"
rsync -a brume@brume.local:/usr/include/ \
  "$SYSROOT/usr/include/"
rsync -a brume@brume.local:/usr/lib/aarch64-linux-gnu/ \
  "$SYSROOT/usr/lib/aarch64-linux-gnu/"
```

## Steady-state loop

```sh
scripts/deploy-cm5.sh
```

That's it. The script:

1. Cross-builds `brume-main` for `aarch64-unknown-linux-gnu.2.41`
   (glibc pin matches Trixie's runtime).
2. scps the binary to `/tmp/brume.new` on the CM5.
3. `systemctl --user stop brume.service`.
4. `sudo install -m 0755 -o root -g root /tmp/brume.new
   /usr/bin/brume.new` (staging path).
5. `sudo setcap cap_sys_nice=eip /usr/bin/brume.new`.
6. `sudo mv /usr/bin/brume.new /usr/bin/brume` (atomic).
7. `systemctl --user start brume.service`.
8. Confirms the service is `active`.

The `setcap` step is **load-bearing**: brume's audio thread
elevates itself to `SCHED_FIFO` rtprio 80 at first cpal callback,
which requires `CAP_SYS_NICE` on the binary's xattrs. `cp` strips
file caps by default, so a naive `sudo cp /tmp/brume.new
/usr/bin/brume` wipes the capability — the next start logs
`failed to set SCHED_FIFO (OS(1))`, runs `SCHED_OTHER`, and
audibly crackles under sustained MIDI load. Always go through
the staged install + setcap + mv chain, never a bare `cp`.

`libcap2-bin` (`setcap` / `getcap`) must be installed on the
device. `sudo apt install libcap2-bin` if the chain fails with
`setcap: command not found`.

`--no-restart` skips steps 3–6 if you want the binary on the
device but don't want to bounce the running service.

Env-var overrides (see the script header):
- `BRUME_HOST` — target hostname (default: `brume.local`)
- `BRUME_USER` — target username (default: `brume`)
- `BRUME_SSH_KEY` — identity file (default: ssh-agent / `~/.ssh/config`)
- `BRUME_SYSROOT` — sysroot path
- `BRUME_TARGET` — cargo triple incl. glibc pin

## Why /usr/bin/brume and not the cargo target

The systemd unit's `ExecStart=/usr/bin/brume` gives the binary a
stable, predictable location independent of where source happens
to be checked out. `deploy-cm5.sh` always swaps that file; there
is no `~/brume/target/` on the device anymore.

## Restart vs. stop → cp → start

`deploy-cm5.sh` uses `stop → cp → start` because we have to swap
the binary itself, not just bounce the running process. This adds
a brief (sub-second) window during which `/usr/bin/brume` is the
new file but the service hasn't restarted yet — that's fine; the
service is down during this window.

Don't `kill -9` brume to "reload" — the persist workers may have
unflushed state, and a hard kill on the audio thread can leave
ALSA in a confused state until the next reboot.

## Stopping brume without restarting

```sh
ssh brume@brume.local 'systemctl --user stop brume.service'
```

Useful when you want to run ad-hoc with extra env vars:

```sh
ssh brume@brume.local '
  systemctl --user stop brume.service
  WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
    BRUME_LOG_CC=1 /usr/bin/brume > /tmp/brume.log 2>&1
'
```

When you're done, `systemctl --user start brume.service` brings
the service back. Or just reboot.

## Killing a stuck instance

`pkill -x brume` (exact match) over SSH. **Do not** use
`pkill -f brume` — the pattern matches the SSH session's argv on
the remote (which contains "brume" in the shell command being
run), and the kill takes out your own session. This is a real
trap that's been hit before.

## Logs and debugging

- `journalctl --user -u brume.service` — current boot.
- `journalctl --user -u brume.service -b -1` — previous boot.
  Useful when brume crashed and rebooted.
- `journalctl --user -u brume.service -f -n 50` — live tail.
- `tail -f /run/user/1000/brume.log` — brume's own stdout/stderr,
  separate from the journal because the service unit redirects to
  `append:%t/brume.log` for chime/script `print` capture.
- `top` on the device — watch CPU usage. Brume should sit at
  10–20 % on one core under steady audio load. Higher than that
  for sustained periods is a perf regression worth investigating.
- `cat /proc/asound/UAC2Gadget/pcm0p/sub0/status` — Meridian USB
  audio status. If `hw_ptr` and `appl_ptr` are diverging, the Mac
  side isn't pulling, which wedges the cpal_alsa_out thread (a
  documented gotcha — see `reference_meridian_wedge_diagnosis`).

## When you don't have a CM5

Code-only contributions are welcome. Run:

```sh
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- \
  -D clippy::correctness -D clippy::suspicious -D clippy::perf \
  -A clippy::pedantic
```

These pass on macOS or Linux desktops. The audio engine and UI
*may* run on a desktop Linux box (cpal does, iced does), but
that's an experimental compatibility surface, not a supported
user path. Expect maintainer review feedback that exercises your
change on a CM5 before merge.

`brumectl` (the host-side companion CLI) is the exception — it
builds and runs natively on macOS and Linux desktops, and
`cargo test -p brumectl` covers it.

## Anti-patterns

- **Don't build on the device.** Earlier versions of this skill
  recommended on-device builds. They worked for tiny incremental
  rebuilds, but a public-trait change in a low-level crate (e.g.
  `ControlSurfaceApi`) forces a full ui-native recompile that
  ran 12 + minutes on the CM5 and once tipped the device into a
  hard freeze. Cross-compile from the workstation is the
  canonical path now.
- **Don't edit files directly on the device** unless you're
  diagnosing something. The dev source is authoritative; the
  CM5 has no source tree anymore — only the deployed binary, Lua
  scripts in `~/brume/scripts/`, and runtime state in `~/.brume/`.
- **Don't disable systemd hardening for "convenience."** The unit
  has `NoNewPrivileges`, `ProtectSystem`, etc. for good reasons.
  If brume needs a capability it doesn't have, add the capability
  explicitly (the pattern is `cap_sys_nice` for SCHED_FIFO).
- **Don't skip the journal check.** A successful service start
  doesn't mean a clean start; the engine can come up and panic on
  a missing config file or a corrupt patch and exit immediately,
  with systemd dutifully restarting it in a loop. Always tail the
  journal after a deploy until you see it past initialization.

## Out of scope for this skill

- First-time CM5 bring-up (flashing, network, USB gadget). See
  `INSTALL.md`.
- Cutting a release (cross-built `brume` + `brumectl` binaries). See
  `BUILD_AND_RELEASE.md` and `release.yml`.
- Hardware setup (IO board, screen, audio HAT). See `HARDWARE.md`.
