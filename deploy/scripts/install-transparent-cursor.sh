#!/bin/sh
# Install the brume-transparent Xcursor theme and wire it into labwc's
# environment so the compositor renders no visible pointer in appliance
# mode. This is the compositor-level fix for touchscreens that expose
# a mouse handler alongside the touch handler (e.g. the MAGEX 10.1"
# 1920×1200, which reports as TSTP MTouch + mouse0 under Pi OS).
#
# Brume's own in-webview CSS cursors are left untouched — semantically
# correct for any mouse user who connects one; this theme just ensures
# labwc doesn't paint the cursor at all.
#
# Idempotent: replaces any previous installation at
# ~/.local/share/icons/brume-transparent/.
#
# Applies on next labwc start. Reboot is the simplest way to take effect.

set -eu

THEME_DIR="$HOME/.local/share/icons/brume-transparent"
CURSORS_DIR="$THEME_DIR/cursors"

mkdir -p "$CURSORS_DIR"

# index.theme tells libxcursor what this theme is called and what
# (non-existent) parent to inherit from so no system cursors leak through.
cat > "$THEME_DIR/index.theme" <<'EOF'
[Icon Theme]
Name=Brume Transparent
Comment=Fully transparent cursor for touch-only appliance mode
Inherits=
EOF

# Generate a 1x1 fully transparent Xcursor binary (68 bytes). Format per
# libxcursor's file.c: file header (16) + TOC entry (12) + image chunk (36
# bytes header + 4 bytes BGRA). Nominal size 32 = the size libxcursor
# matches against at 32px; libxcursor will rescale for other sizes.
python3 - <<'PY' > "$CURSORS_DIR/default"
import struct, sys
MAGIC = b'Xcur'
CURSOR_TYPE = 0xfffd0002
NOMINAL = 32
# File header: magic, header_size, version, ntoc
sys.stdout.buffer.write(struct.pack('<4sIII', MAGIC, 16, 0x10000, 1))
# TOC entry: type, subtype (nominal size), position (byte offset of image chunk)
sys.stdout.buffer.write(struct.pack('<III', CURSOR_TYPE, NOMINAL, 28))
# Image chunk: header_size=36, type, subtype, version=1, width, height, xhot, yhot, delay
sys.stdout.buffer.write(struct.pack('<IIIIIIIII', 36, CURSOR_TYPE, NOMINAL, 1, 1, 1, 0, 0, 0))
# Pixel data: 1 BGRA pixel, fully transparent (all zero)
sys.stdout.buffer.write(struct.pack('<I', 0))
PY

# Symlink every commonly-used cursor shape name to the transparent default.
# libxcursor looks up shapes by name; anything unresolved fails back to a
# system cursor (undesirable). Enumerating the standard set covers it.
cd "$CURSORS_DIR"
for shape in \
  left_ptr arrow pointer hand hand1 hand2 \
  text xterm ibeam \
  crosshair cross \
  wait watch progress left_ptr_watch \
  help question_arrow whats_this \
  ew-resize ns-resize nesw-resize nwse-resize col-resize row-resize \
  n-resize s-resize e-resize w-resize ne-resize nw-resize se-resize sw-resize \
  size_hor size_ver size_bdiag size_fdiag \
  move grab grabbing openhand closedhand all-scroll \
  not-allowed no-drop forbidden \
  dnd-copy dnd-move dnd-none dnd-link dnd-ask dnd-no-drop \
  top_left_corner top_right_corner bottom_left_corner bottom_right_corner \
  top_side bottom_side left_side right_side \
  sb_h_double_arrow sb_v_double_arrow bd_double_arrow fd_double_arrow \
  sb_left_arrow sb_right_arrow sb_up_arrow sb_down_arrow \
  double_arrow plus context-menu copy link alias \
  cell zoom-in zoom-out vertical-text; do
  ln -sf default "$shape"
done

# Wire into labwc's environment so the compositor picks up the theme on
# next start. Replace any existing XCURSOR_THEME= line; otherwise append.
ENV_FILE="$HOME/.config/labwc/environment"
mkdir -p "$(dirname "$ENV_FILE")"
touch "$ENV_FILE"
if grep -q '^XCURSOR_THEME=' "$ENV_FILE"; then
    sed -i 's|^XCURSOR_THEME=.*|XCURSOR_THEME=brume-transparent|' "$ENV_FILE"
else
    echo "XCURSOR_THEME=brume-transparent" >> "$ENV_FILE"
fi

echo "brume-transparent theme: $THEME_DIR"
echo "  default cursor: $(stat -c %s "$CURSORS_DIR/default") bytes"
echo "  shape symlinks: $(find "$CURSORS_DIR" -maxdepth 1 -type l | wc -l)"
echo "labwc env: $(grep XCURSOR_THEME "$ENV_FILE")"
echo
echo "Applies on next labwc start (reboot or re-login tty1)."
