#!/usr/bin/env bash
#
# Install the bot-hq AppImage on a Linux host whose graphics stack is newer
# than the AppImage payload (Fedora 40+, Arch, recent openSUSE, …).
#
# Why this exists
# ---------------
# The AppImage is built on Ubuntu 22.04, so it carries that era's
# libwayland-* and libepoxy alongside the app. It does NOT carry libEGL or
# libGL -- those always come from the host. On a host with a much newer Mesa,
# the new host libEGL is loaded next to the old bundled libwayland-client and
# EGL display creation fails:
#
#     Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
#
# WebKitWebProcess aborts, so the backend starts (sockets bind, the DB is
# written) but no window is ever mapped -- the app looks like it does nothing.
# Setting WEBKIT_DISABLE_DMABUF_RENDERER / GDK_BACKEND does not help, because
# the mismatch is in the library set, not the renderer.
#
# The fix is to drop the five bundled libs so the host's matching copies load.
# Everything else in the payload (WebKit itself, GTK, the app binary) is kept.
#
# Usage:  scripts/install-appimage-linux.sh <path-to-AppImage>
#
set -euo pipefail

APPIMAGE="${1:-}"
if [ -z "$APPIMAGE" ] || [ ! -f "$APPIMAGE" ]; then
    echo "usage: $0 <path-to-bot-hq_*.AppImage>" >&2
    exit 2
fi
APPIMAGE="$(readlink -f "$APPIMAGE")"

PREFIX="${PREFIX:-$HOME/.local}"
LIBDIR="$PREFIX/lib/bot-hq"
BINDIR="$PREFIX/bin"
DESKTOPDIR="$PREFIX/share/applications"
ICONDIR="$PREFIX/share/icons/hicolor/256x256/apps"

# Libraries whose bundled copies conflict with a newer host Mesa/Wayland stack.
CONFLICTING_LIBS=(
    libwayland-client.so.0
    libwayland-egl.so.1
    libwayland-server.so.0
    libwayland-cursor.so.0
    libepoxy.so.0
)

echo "==> extracting $(basename "$APPIMAGE")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
chmod +x "$APPIMAGE"
( cd "$WORK" && "$APPIMAGE" --appimage-extract >/dev/null )

PAYLOAD="$WORK/squashfs-root"
[ -d "$PAYLOAD" ] || { echo "extraction produced no squashfs-root" >&2; exit 1; }

echo "==> removing bundled libs that conflict with the host graphics stack"
removed=0
for lib in "${CONFLICTING_LIBS[@]}"; do
    if [ -e "$PAYLOAD/usr/lib/$lib" ]; then
        rm -f "$PAYLOAD/usr/lib/$lib"
        echo "    removed $lib"
        removed=$((removed + 1))
    fi
done
[ "$removed" -gt 0 ] || echo "    (none present -- payload may already be patched)"

echo "==> installing to $LIBDIR"
mkdir -p "$LIBDIR" "$BINDIR" "$DESKTOPDIR" "$ICONDIR"
rm -rf "${LIBDIR:?}/squashfs-root"
mv "$PAYLOAD" "$LIBDIR/squashfs-root"

cat > "$BINDIR/bot-hq" <<LAUNCHER
#!/usr/bin/env bash
# bot-hq launcher -- patched AppImage payload (see scripts/install-appimage-linux.sh)
exec "$LIBDIR/squashfs-root/AppRun" "\$@"
LAUNCHER
chmod +x "$BINDIR/bot-hq"
echo "    launcher -> $BINDIR/bot-hq"

icon_src="$LIBDIR/squashfs-root/bot-hq.png"
[ -f "$icon_src" ] && cp -f "$icon_src" "$ICONDIR/bot-hq.png"

cat > "$DESKTOPDIR/bot-hq.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=bot-hq
Comment=Drive AI-assisted coding sessions through an agent harness
Exec=$BINDIR/bot-hq
Icon=bot-hq
Terminal=false
Categories=Development;
DESKTOP
echo "    desktop entry -> $DESKTOPDIR/bot-hq.desktop"

command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database "$DESKTOPDIR" >/dev/null 2>&1 || true

echo
echo "done. run 'bot-hq' (ensure $BINDIR is on your PATH) or launch it from your app menu."
