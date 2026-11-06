#!/bin/sh
# Installs ConsoleUI as sessions that can be picked on the login screen (SDDM/GDM):
#   "ConsoleUI"                  gamescope session (session/consoleui-session)
#   "ConsoleUI (own compositor)" Epic 7 compositor (session/consoleui-compositor-session), only
#                                if the compositor was built
# Build first as a normal user:  cargo build --release -p consoleui -p consoleui-store -p consoleui-files -p consoleui-media
#   and for the compositor:      dev/toolbox.sh run cargo build --release -p consoleui-compositor
#   (or everything in the toolbox: dev/toolbox.sh run cargo build --release --workspace)
# Then:                          sudo session/install.sh
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
# The host build of the shell, else the toolbox build (e.g. in the test VM, which has no Rust)
bin="${CONSOLEUI_BUILD:-$root/target/release/consoleui}"
[ -x "$bin" ] || bin="$root/target/toolbox/release/consoleui"
compositor="${CONSOLEUI_COMPOSITOR_BUILD:-$root/target/toolbox/release/consoleui-compositor}"
if [ ! -x "$bin" ]; then
    echo "Run 'cargo build --release' first." >&2
    exit 1
fi
echo "Shell: $bin"

install -Dm755 "$bin" /usr/local/bin/consoleui
install -Dm755 "$root/session/consoleui-session" /usr/local/bin/consoleui-session
install -Dm644 "$root/session/consoleui.desktop" /usr/share/wayland-sessions/consoleui.desktop
echo "Installed the session \"ConsoleUI\" (gamescope)."

if [ -x "$compositor" ]; then
    install -Dm755 "$compositor" /usr/local/bin/consoleui-compositor
    install -Dm755 "$root/session/consoleui-compositor-session" /usr/local/bin/consoleui-compositor-session
    install -Dm644 "$root/session/consoleui-compositor.desktop" \
        /usr/share/wayland-sessions/consoleui-compositor.desktop
    echo "Installed the session \"ConsoleUI (own compositor)\"."
else
    echo "No compositor build at $compositor; skipped the \"ConsoleUI (own compositor)\" session."
fi
store="${CONSOLEUI_STORE_BUILD:-$root/target/release/consoleui-store}"
[ -x "$store" ] || store="$root/target/toolbox/release/consoleui-store"
if [ -x "$store" ]; then
    install -Dm755 "$store" /usr/local/bin/consoleui-store
    install -Dm644 "$root/crates/store/consoleui-store.desktop" /usr/share/applications/consoleui-store.desktop
    install -Dm644 "$root/crates/store/consoleui-store.svg" \
        /usr/share/icons/hicolor/scalable/apps/consoleui-store.svg
    echo "Installed the store."
else
    echo "No store build; skipped it (cargo build --release -p consoleui-store)."
fi
files="${CONSOLEUI_FILES_BUILD:-$root/target/release/consoleui-files}"
[ -x "$files" ] || files="$root/target/toolbox/release/consoleui-files"
if [ -x "$files" ]; then
    install -Dm755 "$files" /usr/local/bin/consoleui-files
    install -Dm644 "$root/crates/files/consoleui-files.desktop" /usr/share/applications/consoleui-files.desktop
    install -Dm644 "$root/crates/files/consoleui-files.svg" \
        /usr/share/icons/hicolor/scalable/apps/consoleui-files.svg
    echo "Installed the file manager."
else
    echo "No file manager build; skipped it (cargo build --release -p consoleui-files)."
fi
media="${CONSOLEUI_MEDIA_BUILD:-$root/target/release/consoleui-media}"
[ -x "$media" ] || media="$root/target/toolbox/release/consoleui-media"
if [ -x "$media" ]; then
    install -Dm755 "$media" /usr/local/bin/consoleui-media
    install -Dm644 "$root/crates/media/consoleui-media.desktop" /usr/share/applications/consoleui-media.desktop
    install -Dm644 "$root/crates/media/consoleui-media.svg" \
        /usr/share/icons/hicolor/scalable/apps/consoleui-media.svg
    echo "Installed the media player."
    command -v mpv >/dev/null || echo "The media player needs mpv to play anything."
else
    echo "No media player build; skipped it (cargo build --release -p consoleui-media)."
fi
echo "Done. Log out and pick the session on the login screen."
