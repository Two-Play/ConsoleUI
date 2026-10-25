#!/bin/sh
# Installs ConsoleUI as sessions that can be picked on the login screen (SDDM/GDM):
#   "ConsoleUI"                  gamescope session (session/consoleui-session)
#   "ConsoleUI (own compositor)" Epic 7 compositor (session/consoleui-compositor-session), only
#                                if the compositor was built
# Build first as a normal user:  cargo build --release
#   and for the compositor:      dev/toolbox.sh run cargo build --release -p consoleui-compositor
# Then:                          sudo session/install.sh
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/consoleui"
compositor="${CONSOLEUI_COMPOSITOR_BUILD:-$root/target/toolbox/release/consoleui-compositor}"
if [ ! -x "$bin" ]; then
    echo "Run 'cargo build --release' first." >&2
    exit 1
fi

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
echo "Done. Log out and pick the session on the login screen."
