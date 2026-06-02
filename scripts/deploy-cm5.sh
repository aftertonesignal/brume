#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Brandon Huey <hello@aftertone.co>
#
# deploy-cm5.sh — cross-compile brume on the maintainer's Mac and
# deploy the binary to a running CM5. Replaces the older "rsync source
# → cargo build on device" loop, which was 30 s on a good day and
# 12+ minutes when a public-trait change forced the full ui-native
# rebuild (see post-mortem in CHANGELOG / planning notes).
#
# Time-to-deploy on Apple Silicon: ~30 s incremental, ~60 s clean.
# The CM5 never compiles anything; the heavy lifting stays on
# the workstation.
#
# Prerequisites (one-time setup; see DEPLOY.md):
#   - rustup target add aarch64-unknown-linux-gnu
#   - cargo install cargo-zigbuild
#   - brew install zig
#   - rsync the device sysroot into ~/.local/share/brume-sysroot/
#     aarch64-debian-trixie/ (DEPLOY.md §sysroot has the exact
#     rsync invocation; ~1.1 GB, one-time pull)
#   - SSH key + ~/.ssh/config entry pointing at the CM5
#
# Usage:
#   scripts/deploy-cm5.sh                # cross-build + deploy
#   scripts/deploy-cm5.sh --no-restart   # build + scp only, leave service alone
#   BRUME_HOST=other.local scripts/deploy-cm5.sh   # deploy elsewhere
#
# Env overrides:
#   BRUME_HOST        target hostname (default: brume.local)
#   BRUME_USER        target username (default: brume)
#   BRUME_SSH_KEY     identity file path (default: ssh-agent / ~/.ssh/config)
#   BRUME_SYSROOT     aarch64 sysroot path (default: ~/.local/share/brume-sysroot/aarch64-debian-trixie)
#   BRUME_TARGET      cargo triple incl. glibc pin (default: aarch64-unknown-linux-gnu.2.41)

set -euo pipefail

HOST="${BRUME_HOST:-brume.local}"
USER="${BRUME_USER:-brume}"
SYSROOT="${BRUME_SYSROOT:-$HOME/.local/share/brume-sysroot/aarch64-debian-trixie}"

# SSH identity. Set BRUME_SSH_KEY to force a specific key; left unset,
# ssh/scp defer to the agent / ~/.ssh/config (matches brumectl). No
# maintainer-specific key path is baked into the script.
_ssh() { if [ -n "${BRUME_SSH_KEY:-}" ]; then ssh -i "$BRUME_SSH_KEY" "$@"; else ssh "$@"; fi; }
_scp() { if [ -n "${BRUME_SSH_KEY:-}" ]; then scp -i "$BRUME_SSH_KEY" "$@"; else scp "$@"; fi; }
TARGET="${BRUME_TARGET:-aarch64-unknown-linux-gnu.2.41}"
RESTART=1

for arg in "$@"; do
  case "$arg" in
    --no-restart) RESTART=0 ;;
    *) echo "unknown arg: $arg" >&2; exit 1 ;;
  esac
done

# Sysroot must exist or we'll burn 30 seconds of compile time only to
# fail at link with a confusing pkg-config error. Front-load the check.
if [ ! -d "$SYSROOT/usr/lib/aarch64-linux-gnu/pkgconfig" ]; then
  cat >&2 <<EOF
deploy-cm5: sysroot not found at $SYSROOT
Run the one-time pull from DEPLOY.md, or set BRUME_SYSROOT to point
at an existing aarch64 Debian sysroot.
EOF
  exit 1
fi

echo "==> cross-building brume-main for $TARGET"
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG_PATH="$SYSROOT/usr/lib/aarch64-linux-gnu/pkgconfig:$SYSROOT/usr/share/pkgconfig"
export PKG_CONFIG_ALLOW_CROSS=1
cargo zigbuild --release --target "$TARGET" -p brume-main

# zigbuild produces output under the bare triple, not the glibc-pinned
# variant — strip the suffix to find the artifact.
TRIPLE="${TARGET%.*.*}"
BINARY="target/$TRIPLE/release/brume"
if [ ! -x "$BINARY" ]; then
  echo "deploy-cm5: build did not produce $BINARY" >&2
  exit 1
fi

SIZE=$(du -h "$BINARY" | awk '{print $1}')
echo "==> built $BINARY ($SIZE)"

echo "==> scp to $USER@$HOST:/tmp/brume.new"
_scp "$BINARY" "$USER@$HOST:/tmp/brume.new"

# Push the in-tree factory preset tree to /usr/share/brume/factory/.
# `brumectl install` does this from a release-time tarball; the dev
# loop mirrors it from the source dir so the running brume sees the
# same factory layer the released binary would. The rsync pattern
# below stages under /tmp first because /usr/share/brume isn't
# writable by the SSH user — sudo mv-into-place is the simplest
# privileged step that doesn't require a passwordless rsync rule.
FACTORY_SRC="$(dirname "$0")/../crates/patch-store/factory"
if [ -d "$FACTORY_SRC" ]; then
  echo "==> push factory presets → $USER@$HOST:/usr/share/brume/factory"
  _ssh "$USER@$HOST" 'rm -rf /tmp/brume-factory && mkdir -p /tmp/brume-factory'
  # `tar | ssh tar` keeps directory metadata sane and avoids an rsync
  # dependency on the device. `COPYFILE_DISABLE=1` suppresses macOS's
  # AppleDouble metadata sidecars (`._<name>`) that bsdtar emits by
  # default — those leak into the receiving filesystem as legitimate-
  # looking `.json` siblings and the runtime's listing code (correctly
  # filters dotfiles, but keep the source clean too).
  COPYFILE_DISABLE=1 tar -C "$FACTORY_SRC" -cf - . \
    | _ssh "$USER@$HOST" 'tar -C /tmp/brume-factory -xf -'
fi


if [ "$RESTART" -eq 1 ]; then
  # Stop → atomic-install + setcap → start. The setcap step is
  # load-bearing: brume's audio thread elevates itself to SCHED_FIFO
  # rtprio 80 via sched_setscheduler() at first cpal callback, which
  # requires CAP_SYS_NICE on the binary's xattrs. Without it the
  # elevation EPERMs and the cpal worker runs SCHED_OTHER, where it
  # gets preempted by iced GUI repaint pressure under sustained MIDI
  # — audible as crackle at all gain levels.
  #
  # `cp` strips file caps (xattrs aren't preserved by default), so a
  # bare `sudo cp /tmp/brume.new /usr/bin/brume` was wiping the
  # capability on every deploy. The staging pattern below — install
  # + setcap on the staging path, then atomic `mv` into place — is
  # the same shape `brumectl install` uses; if setcap fails (rare,
  # happens on no-xattr filesystems) /usr/bin/brume is unchanged
  # rather than left in a no-cap state.
  echo "==> swap /usr/bin/brume + install factory + restart brume.service"
  _ssh "$USER@$HOST" '
    set -e
    systemctl --user stop brume.service
    sudo install -m 0755 -o root -g root /tmp/brume.new /usr/bin/brume.new
    sudo setcap cap_sys_nice=eip /usr/bin/brume.new
    sudo mv /usr/bin/brume.new /usr/bin/brume
    rm -f /tmp/brume.new
    if [ -d /tmp/brume-factory ]; then
      sudo rm -rf /usr/share/brume/factory
      sudo mkdir -p /usr/share/brume
      sudo mv /tmp/brume-factory /usr/share/brume/factory
      sudo chown -R root:root /usr/share/brume/factory
      sudo chmod -R a+rX /usr/share/brume/factory
    fi
    systemctl --user start brume.service
    sleep 2
    systemctl --user is-active brume.service
  '
else
  echo "==> --no-restart: binary at /tmp/brume.new on $HOST, service untouched"
fi

echo "==> done"
