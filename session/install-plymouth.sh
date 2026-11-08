#!/bin/sh
# Installs the ConsoleUI boot splash (Plymouth theme, Story 6.11) and makes it the default, which
# rebuilds the initramfs so that it shows from early boot on. Needs Plymouth's script plugin
# (Fedora: plymouth-plugin-script; Debian, Ubuntu and Arch ship it with plymouth) and `rhgb quiet`
# (or `splash`) on the kernel command line, which most desktop installs have.
#
# With SDDM it also hands the screen over without a gap, like GDM does by itself: SDDM deactivates
# Plymouth instead of waiting for it to quit, so the splash stays on screen until the session's
# compositor shows its first frame, and Plymouth quits a moment later, keeping its last image.
#
#   sudo session/install-plymouth.sh             install and select the theme
#   sudo session/install-plymouth.sh --no-initrd  only install and select it (rebuild later)
#   sudo session/install-plymouth.sh --restore    back to the previous theme (or the given one)
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
dest=/usr/share/plymouth/themes/consoleui
state=/var/lib/consoleui/plymouth-previous-theme
handover=/etc/systemd/system/sddm.service.d/consoleui-plymouth.conf
# Written by an earlier version of this script
old_handover=/etc/systemd/system/plymouth-quit.service.d/consoleui-handover.conf

if ! command -v plymouth-set-default-theme >/dev/null; then
    echo "Plymouth is not installed." >&2
    exit 1
fi

remove_handover() {
    for file in "$handover" "$old_handover"; do
        [ -e "$file" ] || continue
        rm -f "$file"
        rmdir "${file%/*}" 2>/dev/null || true
    done
    systemctl daemon-reload 2>/dev/null || true
}

case "${1:-}" in
--restore)
    previous="${2:-}"
    [ -n "$previous" ] || previous="$(cat "$state" 2>/dev/null || true)"
    if [ -n "$previous" ] && [ "$previous" != consoleui ]; then
        echo "Switching back to \"$previous\" and rebuilding the initramfs..."
        plymouth-set-default-theme -R "$previous"
    else
        echo "Switching back to the distribution's theme and rebuilding the initramfs..."
        plymouth-set-default-theme --reset -R
    fi
    rm -rf "$dest" "$state"
    remove_handover
    echo "Done."
    exit 0
    ;;
esac

if [ -z "$(find /usr/lib*/plymouth /usr/lib/*/plymouth -maxdepth 1 -name script.so 2>/dev/null | head -1)" ]; then
    echo "Plymouth's script plugin is missing (Fedora: sudo dnf install plymouth-plugin-script)." >&2
    exit 1
fi

current="$(plymouth-set-default-theme)"
if [ "$current" != consoleui ]; then
    install -Dm644 /dev/null "$state"
    echo "$current" >"$state"
fi
rm -rf "$dest"
install -d "$dest"
install -m644 "$root"/session/plymouth/consoleui/* "$dest"/
echo "Installed the theme to $dest (previous theme: $(cat "$state" 2>/dev/null || echo unknown))."

remove_handover
if [ -e /usr/lib/systemd/system/sddm.service ] || [ -e /etc/systemd/system/sddm.service ]; then
    install -d "${handover%/*}"
    cat >"$handover" <<'CONF'
# Installed by ConsoleUI's session/install-plymouth.sh: hand the screen from the boot splash to the
# session without showing the text console in between (what GDM does by itself). SDDM deactivates
# Plymouth instead of waiting for plymouth-quit, so the splash stays on screen while the session's
# compositor starts; Plymouth quits with --retain-splash once the ConsoleUI compositor's control
# socket exists plus a moment for its first frame (after 20 s for any other session).
[Unit]
Conflicts=plymouth-quit.service
OnFailure=plymouth-quit.service

[Service]
ExecStartPre=-/usr/bin/plymouth deactivate
ExecStartPost=-/bin/sh -c '(i=0; while [ $i -lt 200 ]; do if ls /run/user/*/consoleui-*.sock >/dev/null 2>&1; then sleep 3; break; fi; sleep 0.1; i=$((i + 1)); done; /usr/bin/plymouth quit --retain-splash) >/dev/null 2>&1 &'
CONF
    systemctl daemon-reload 2>/dev/null || true
    echo "Installed $handover (the splash stays until the session draws)."
fi

if [ "${1:-}" = --no-initrd ]; then
    plymouth-set-default-theme consoleui
    echo "Selected; rebuild the initramfs to show it at boot (Fedora: dracut -f)."
else
    echo "Selecting it and rebuilding the initramfs (takes a while)..."
    plymouth-set-default-theme -R consoleui
fi
case "$(cat /proc/cmdline)" in
*rhgb* | *splash*) ;;
*) echo "Note: the kernel command line has neither 'rhgb' nor 'splash'; add one to see the splash." ;;
esac
echo "Done. Try it without rebooting: sudo plymouthd; sudo plymouth show-splash; sleep 5; sudo plymouth quit"
