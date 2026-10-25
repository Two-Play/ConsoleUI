#!/bin/sh
# Installiert ConsoleUI als Session, die man im Login-Bildschirm (SDDM/GDM) auswählen kann.
# Vorher als normaler Benutzer bauen:  cargo build --release
# Dann:                                 sudo session/install.sh
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/consoleui"
if [ ! -x "$bin" ]; then
    echo "Bitte zuerst 'cargo build --release' ausführen." >&2
    exit 1
fi

install -Dm755 "$bin" /usr/local/bin/consoleui
install -Dm755 "$root/session/consoleui-session" /usr/local/bin/consoleui-session
install -Dm644 "$root/session/consoleui.desktop" /usr/share/wayland-sessions/consoleui.desktop
echo "Fertig. Nach dem Abmelden im Login-Bildschirm die Session „ConsoleUI“ wählen."
