#!/bin/sh
# install-brumectl.sh — install the brumectl companion CLI on your computer.
#
# Downloads the latest brumectl release matching your OS/arch, verifies its
# SHA-256, and installs it to ~/.local/bin (no sudo). Then run:
#   brumectl install <device-host>
# to set Brume up on your Raspberry Pi over SSH.
#
#   curl -fsSL https://raw.githubusercontent.com/aftertonesignal/brume/main/scripts/install-brumectl.sh | sh
#
# Override the install dir with BRUMECTL_INSTALL_DIR=/some/path.
set -eu

REPO="aftertonesignal/brume"
INSTALL_DIR="${BRUMECTL_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'install-brumectl: %s\n' "$*" >&2; exit 1; }

# --- which release artifact matches this machine? ---
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    [ "$arch" = "arm64" ] || err "unsupported macOS arch '$arch'. brumectl ships for Apple Silicon (arm64) only; build from source: https://github.com/$REPO"
    target="macos-arm64" ;;
  Linux)
    [ "$arch" = "x86_64" ] || err "unsupported Linux arch '$arch'. brumectl ships for x86_64 only; build from source: https://github.com/$REPO"
    target="linux-x86_64" ;;
  *)
    err "unsupported OS '$os'. brumectl runs on macOS and Linux." ;;
esac

# --- resolve the latest release tag ---
say "Finding the latest brumectl release..."
tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep '"tag_name"' | head -1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
[ -n "$tag" ] || err "could not determine the latest release. Check https://github.com/$REPO/releases"
ver="${tag#v}"
asset="brumectl-${ver}-${target}"
base="https://github.com/$REPO/releases/download/${tag}"

# --- download + verify in a temp dir ---
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
say "Downloading ${asset}..."
curl -fL --proto '=https' --progress-bar "${base}/${asset}" -o "$tmp/brumectl" \
  || err "download failed: ${base}/${asset}"
curl -fsSL --proto '=https' "${base}/${asset}.sha256" -o "$tmp/brumectl.sha256" \
  || err "could not fetch the SHA-256 sidecar"

want="$(awk '{print $1}' "$tmp/brumectl.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  got="$(sha256sum "$tmp/brumectl" | awk '{print $1}')"
else
  got="$(shasum -a 256 "$tmp/brumectl" | awk '{print $1}')"
fi
[ "$want" = "$got" ] || err "checksum mismatch (expected $want, got $got). Aborting."

# --- install ---
mkdir -p "$INSTALL_DIR"
chmod +x "$tmp/brumectl"
mv "$tmp/brumectl" "$INSTALL_DIR/brumectl"
say "Installed brumectl ${ver} -> $INSTALL_DIR/brumectl"

# --- make sure it's on PATH ---
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;  # already on PATH
  *)
    line="export PATH=\"$INSTALL_DIR:\$PATH\""
    shrc=""
    case "${SHELL:-}" in
      *zsh)  shrc="$HOME/.zshrc" ;;
      *bash) [ "$os" = "Darwin" ] && shrc="$HOME/.bash_profile" || shrc="$HOME/.bashrc" ;;
    esac
    if [ -n "$shrc" ]; then
      printf '\n# added by the brumectl installer\n%s\n' "$line" >> "$shrc"
      say ""
      say "Added $INSTALL_DIR to your PATH in $shrc."
      say "Restart your shell, or run:  source $shrc"
    else
      say ""
      say "$INSTALL_DIR is not on your PATH. Add this to your shell profile:"
      say "  $line"
    fi ;;
esac

say ""
say "Done. Next, set up Brume on your Pi over SSH:"
say "  brumectl install <device-host>"
