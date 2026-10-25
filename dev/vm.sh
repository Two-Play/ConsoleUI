#!/usr/bin/env bash
# Test VM for the own compositor (Epic 7): a Fedora guest with its own seat, DRM device and TTYs.
#
# The DRM backend and the login session are tested here without touching the host's display. Plain
# QEMU/KVM runs as the user (no libvirt, no sudo). The host checkout is shared into the guest with
# virtiofs at the same absolute path, so binaries built in the toolbox (target/toolbox/release, same
# Fedora release) run directly in the guest. Graphics: virtio-vga without host GL, the guest renders
# with llvmpipe (kms_swrast); the host GPU is never used. VM files live in ~/.local/share/consoleui-vm.
#
#   dev/vm.sh create               download + verify the Fedora Cloud image, build disk and cloud-init seed,
#                                  first boot (installs packages), then shut down; idempotent
#   dev/vm.sh start [--headless]   boot the VM (display in a window unless --headless or no $DISPLAY/$WAYLAND_DISPLAY)
#   dev/vm.sh stop                 clean shutdown (poweroff over SSH, QMP quit as fallback)
#   dev/vm.sh status               running or not, SSH reachable, share mounted
#   dev/vm.sh ssh [cmd...]         SSH into the guest as the test user (cmd runs in the share directory;
#                                  one argument = a shell command line, several = one quoted command)
#   dev/vm.sh scp <src...> <dst>   copy files, prefix guest paths with ':' (e.g. dev/vm.sh scp a.bin :/tmp/)
#   dev/vm.sh seat <cmd...>        run cmd as a logind session on seat0 (VT 7, active), log in ~/seat.log
#   dev/vm.sh seat-stop            stop the seat session and switch back to VT 1
#   dev/vm.sh screenshot <png>     QMP screendump of the VM display as PNG (works headless too)
#   dev/vm.sh console              follow the serial console log
#   dev/vm.sh destroy              stop and delete disk, seed and keys (the downloaded image is kept)
#
# Environment: CONSOLEUI_VM_DIR (state dir), VM_SHARE (host dir to share, default: the main checkout),
# VM_CPUS (4), VM_MEM (6G), VM_DISK (20G), VM_RES (1280x800), VM_SSH_PORT (2222),
# VM_USB (vendor:product[,vendor:product...] USB devices to pass through, e.g. a gamepad; needs
# write access to /dev/bus/usb/BBB/DDD on the host, see docs/dev-vm.md).
set -euo pipefail

RELEASE=44
IMAGE_BASE_URL="https://dl.fedoraproject.org/pub/fedora/linux/releases/$RELEASE/Cloud/x86_64/images"
IMAGE_NAME="Fedora-Cloud-Base-Generic-$RELEASE-1.7.x86_64.qcow2"
CHECKSUM_NAME="Fedora-Cloud-$RELEASE-1.7-x86_64-CHECKSUM"
GPG_KEY="/etc/pki/rpm-gpg/RPM-GPG-KEY-fedora-$RELEASE-primary"

VM_DIR="${CONSOLEUI_VM_DIR:-$HOME/.local/share/consoleui-vm}"
# Share the main checkout, also when this script runs from a git worktree below .claude/worktrees.
SCRIPT_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VM_SHARE="${VM_SHARE:-$(git -C "$SCRIPT_REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null | sed 's#/\.git$##' || echo "$SCRIPT_REPO")}"
[ -d "$VM_SHARE" ] || VM_SHARE="$SCRIPT_REPO"
VM_CPUS="${VM_CPUS:-4}"
VM_MEM="${VM_MEM:-6G}"
VM_DISK="${VM_DISK:-20G}"
VM_RES="${VM_RES:-1280x800}"
VM_SSH_PORT="${VM_SSH_PORT:-2222}"
VM_USB="${VM_USB:-}"
VM_USER=tester
VIRTIOFSD=/usr/libexec/virtiofsd

DISK="$VM_DIR/disk.qcow2"
SEED="$VM_DIR/seed.iso"
KEY="$VM_DIR/id_ed25519"
QMP="$VM_DIR/qmp.sock"
VFS_SOCK="$VM_DIR/virtiofs.sock"
PIDFILE="$VM_DIR/qemu.pid"
VFS_PIDFILE="$VM_DIR/virtiofsd.pid"
SERIAL_LOG="$VM_DIR/serial.log"

die() { echo "vm.sh: $*" >&2; exit 1; }
log() { echo "vm.sh: $*" >&2; }

SSH_OPTS=(-i "$KEY" -p "$VM_SSH_PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes)
vm_ssh() { ssh "${SSH_OPTS[@]}" "$VM_USER@127.0.0.1" "$@"; }

running() { [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; }
ssh_ready() { vm_ssh true 2>/dev/null; }

wait_ssh() { # wait_ssh <seconds>
    local i
    for ((i = 0; i < $1; i += 2)); do
        ssh_ready && return 0
        running || die "QEMU exited, see $SERIAL_LOG"
        sleep 2
    done
    die "no SSH after $1 s, see $SERIAL_LOG"
}

# Send QMP commands (JSON objects as arguments) and print the replies.
qmp() {
    [ -S "$QMP" ] || die "no QMP socket, is the VM running?"
    python3 -I - "$QMP" "$@" <<'EOF'
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
f = s.makefile("rw")
json.loads(f.readline())  # greeting
for cmd in ["{\"execute\": \"qmp_capabilities\"}"] + sys.argv[2:]:
    f.write(cmd + "\n"); f.flush()
    while True:
        reply = json.loads(f.readline())
        if "event" not in reply:
            break
    if "error" in reply:
        sys.exit("QMP error: " + reply["error"]["desc"])
print(json.dumps(reply.get("return")))
EOF
}

check_kvm() {
    [ -r /dev/kvm ] && [ -w /dev/kvm ] || die "no access to /dev/kvm (fix on the host: sudo usermod -aG kvm $USER, then log in again)"
}

download_image() {
    mkdir -p "$VM_DIR"
    local img="$VM_DIR/$IMAGE_NAME" sums="$VM_DIR/$CHECKSUM_NAME"
    [ -f "$sums" ] || curl -fL --retry 3 -o "$sums" "$IMAGE_BASE_URL/$CHECKSUM_NAME"
    # The CHECKSUM file is signed with the Fedora release key that the host already ships.
    if [ -f "$GPG_KEY" ] && command -v gpgv >/dev/null; then
        gpg --dearmor <"$GPG_KEY" >"$VM_DIR/fedora.gpg"
        gpgv --keyring "$VM_DIR/fedora.gpg" "$sums" 2>/dev/null || die "bad signature on $sums"
        log "CHECKSUM signature OK ($GPG_KEY)"
    else
        log "warning: cannot check the CHECKSUM signature (no $GPG_KEY or gpgv)"
    fi
    if [ ! -f "$img" ]; then
        curl -fL --retry 3 -C - -o "$img.part" "$IMAGE_BASE_URL/$IMAGE_NAME"
        mv "$img.part" "$img"
    fi
    local want have
    want="$(sed -n "s/^SHA256 ($IMAGE_NAME) = \([0-9a-f]*\)$/\1/p" "$sums")"
    [ -n "$want" ] || die "no checksum for $IMAGE_NAME in $sums"
    have="$(sha256sum "$img" | cut -d' ' -f1)"
    if [ "$want" != "$have" ]; then
        rm -f "$img"
        die "checksum mismatch for $IMAGE_NAME (deleted, run create again)"
    fi
    log "image checksum OK"
}

write_seed() {
    local dir="$VM_DIR/seed"
    mkdir -p "$dir"
    [ -f "$KEY" ] || ssh-keygen -q -t ed25519 -N '' -C consoleui-vm -f "$KEY"
    cat >"$dir/meta-data" <<EOF
instance-id: consoleui-vm
local-hostname: consoleui-vm
EOF
    cat >"$dir/user-data" <<EOF
#cloud-config
users:
  - name: $VM_USER
    uid: 1000
    groups: [wheel, video, render, input, audio]
    shell: /bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    lock_passwd: true
    ssh_authorized_keys:
      - $(cat "$KEY.pub")
package_update: false
package_upgrade: true
packages:
  # DRM/KMS + software rendering (llvmpipe, kms_swrast), compositor runtime
  - mesa-dri-drivers
  - mesa-vulkan-drivers
  - mesa-libEGL
  - mesa-libGLES
  - mesa-libgbm
  - libdrm
  - libinput
  - libseat
  - seatd
  - libdisplay-info
  - libei
  - pixman
  - libxkbcommon
  - libxkbcommon-x11
  - xorg-x11-server-Xwayland
  - gamescope
  # test clients and tools
  - mesa-demos
  - kmscube
  - drm-utils
  - vulkan-tools
  - wayland-utils
  - weston
  - foot
  - xprop
  - xdpyinfo
  - kbd
  - evtest
  - libinput-utils
  # audio (the shell talks to pw-cat/wpctl)
  - pipewire
  - pipewire-utils
  - wireplumber
  # fonts for Slint
  - fontconfig
  - google-noto-sans-fonts
  - dejavu-sans-fonts
  - dejavu-sans-mono-fonts
write_files:
  # Console autologin on tty1, so the window shows a ready shell for manual tests.
  - path: /etc/systemd/system/getty@tty1.service.d/autologin.conf
    content: |
      [Service]
      ExecStart=
      ExecStart=-/sbin/agetty --autologin $VM_USER --noclear %I \$TERM
  - path: /etc/profile.d/consoleui-vm.sh
    content: |
      export CONSOLEUI_REPO="$VM_SHARE"
runcmd:
  - mkdir -p "$VM_SHARE"
  - echo 'consoleui $VM_SHARE virtiofs defaults,nofail 0 0' >> /etc/fstab
  - systemctl daemon-reload
  - mount "$VM_SHARE" || true
  - systemctl restart getty@tty1
  - touch /var/lib/consoleui-vm-ready
EOF
    rm -f "$SEED"
    xorriso -as mkisofs -quiet -output "$SEED" -volid cidata -joliet -rock "$dir/user-data" "$dir/meta-data" \
        2>"$VM_DIR/xorriso.log" || die "xorriso failed, see $VM_DIR/xorriso.log"
}

start_virtiofsd() {
    rm -f "$VFS_SOCK"
    # Unprivileged: the namespace sandbox works without root (user namespace); files keep the host
    # owner, which matches the guest user (uid 1000).
    setsid "$VIRTIOFSD" --socket-path="$VFS_SOCK" --shared-dir="$VM_SHARE" --sandbox=namespace \
        --cache=auto --announce-submounts >"$VM_DIR/virtiofsd.log" 2>&1 </dev/null &
    echo $! >"$VFS_PIDFILE"
    local i
    for ((i = 0; i < 50; i++)); do [ -S "$VFS_SOCK" ] && return 0; sleep 0.1; done
    die "virtiofsd did not start, see $VM_DIR/virtiofsd.log"
}

start_vm() { # start_vm <headless:0|1>
    check_kvm
    [ -f "$DISK" ] || die "no VM yet, run: dev/vm.sh create"
    if running; then log "already running"; return 0; fi
    start_virtiofsd
    local display=(-display none)
    if [ "$1" = 0 ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        display=(-display gtk,gl=off,zoom-to-fit=on)
    fi
    local usb=()
    if [ -n "$VM_USB" ]; then
        usb=(-device qemu-xhci,id=xhci)
        local dev
        IFS=, read -ra devs <<<"$VM_USB"
        for dev in "${devs[@]}"; do
            usb+=(-device "usb-host,bus=xhci.0,vendorid=0x${dev%%:*},productid=0x${dev##*:}")
        done
    fi
    rm -f "$QMP"
    : >"$SERIAL_LOG"
    setsid qemu-system-x86_64 \
        -name consoleui-vm \
        -machine q35,accel=kvm,memory-backend=mem -cpu host -smp "$VM_CPUS" -m "$VM_MEM" \
        -object "memory-backend-memfd,id=mem,size=$VM_MEM,share=on" \
        -drive "file=$DISK,if=virtio,discard=unmap" \
        -drive "file=$SEED,if=virtio,media=cdrom,readonly=on" \
        -chardev "socket,id=vfs,path=$VFS_SOCK" -device vhost-user-fs-pci,chardev=vfs,tag=consoleui \
        -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$VM_SSH_PORT-:22" \
        -vga none -device "virtio-vga,xres=${VM_RES%x*},yres=${VM_RES#*x}" \
        -device virtio-keyboard-pci -device virtio-tablet-pci \
        -device virtio-rng-pci \
        "${usb[@]}" \
        "${display[@]}" \
        -serial "file:$SERIAL_LOG" \
        -qmp "unix:$QMP,server=on,wait=off" \
        -pidfile "$PIDFILE" \
        >"$VM_DIR/qemu.log" 2>&1 </dev/null &
    local i
    for ((i = 0; i < 50; i++)); do running && break; sleep 0.1; done
    running || die "QEMU did not start: $(cat "$VM_DIR/qemu.log")"
    log "started (pid $(cat "$PIDFILE")), waiting for SSH on port $VM_SSH_PORT"
    wait_ssh 300
    log "ready: dev/vm.sh ssh"
}

stop_vm() {
    if running; then
        vm_ssh sudo systemctl poweroff 2>/dev/null || true
        local i
        for ((i = 0; i < 60; i++)); do running || break; sleep 1; done
        if running; then qmp '{"execute": "quit"}' >/dev/null 2>&1 || kill "$(cat "$PIDFILE")"; sleep 1; fi
    fi
    if [ -f "$VFS_PIDFILE" ]; then kill "$(cat "$VFS_PIDFILE")" 2>/dev/null || true; fi
    rm -f "$PIDFILE" "$VFS_PIDFILE" "$QMP" "$VFS_SOCK"
}

# Quote arguments for the remote shell.
# A single argument is taken as a shell command line (like ssh), several are quoted word by word.
quote() {
    if [ $# -eq 1 ]; then echo "$1"; return; fi
    local out=() a
    for a in "$@"; do out+=("$(printf '%q' "$a")"); done
    echo "${out[*]}"
}

case "${1:-}" in
create)
    check_kvm
    download_image
    write_seed
    if [ ! -f "$DISK" ]; then
        qemu-img create -q -f qcow2 -F qcow2 -b "$VM_DIR/$IMAGE_NAME" "$DISK" "$VM_DISK"
        log "first boot: cloud-init installs the packages (a few minutes)"
        start_vm 1
        vm_ssh 'sudo cloud-init status --wait >/dev/null; test -f /var/lib/consoleui-vm-ready' \
            || die "cloud-init failed, see: dev/vm.sh ssh sudo cat /var/log/cloud-init-output.log"
        stop_vm
    fi
    log "VM is ready, try: dev/vm.sh start"
    ;;
start)
    start_vm "$([ "${2:-}" = --headless ] && echo 1 || echo 0)"
    ;;
stop)
    stop_vm
    ;;
status)
    if running; then
        echo "running (pid $(cat "$PIDFILE")), ssh port $VM_SSH_PORT"
        if ssh_ready; then
            vm_ssh "findmnt -n -o SOURCE,TARGET $(printf '%q' "$VM_SHARE") || echo 'share not mounted'; loginctl list-sessions --no-legend"
        else
            echo "ssh not reachable (yet)"
        fi
    else
        echo "stopped"
    fi
    ;;
ssh)
    shift
    running || die "VM is not running, run: dev/vm.sh start"
    if [ $# -eq 0 ]; then exec ssh -t "${SSH_OPTS[@]}" "$VM_USER@127.0.0.1"; fi
    exec ssh "${SSH_OPTS[@]}" "$VM_USER@127.0.0.1" "cd $(printf '%q' "$VM_SHARE") 2>/dev/null; $(quote "$@")"
    ;;
scp)
    shift
    args=()
    for a in "$@"; do [[ "$a" == :* ]] && args+=("$VM_USER@127.0.0.1$a") || args+=("$a"); done
    exec scp -i "$KEY" -P "$VM_SSH_PORT" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR "${args[@]}"
    ;;
seat)
    shift
    [ $# -gt 0 ] || die "usage: dev/vm.sh seat <cmd...>"
    # A transient unit with PAMName=login + TTYPath gets a real logind session on seat0 (like a display
    # manager), so libseat (logind backend) can take the DRM device and the input devices. VT 7 is above
    # NAutoVTs, so no getty competes for it; chvt makes the session active.
    vm_ssh "sudo systemctl stop consoleui-seat 2>/dev/null; sudo systemctl reset-failed consoleui-seat 2>/dev/null; \
        sudo chvt 7; sudo systemd-run --quiet --unit=consoleui-seat --uid=$VM_USER \
        -p PAMName=login -p TTYPath=/dev/tty7 -p TTYReset=yes -p TTYVHangup=yes \
        -p StandardInput=tty \
        -p Environment=XDG_SESSION_TYPE=wayland -p Environment=XDG_SESSION_CLASS=user \
        -p WorkingDirectory=$(printf '%q' "$VM_SHARE") \
        /bin/bash -lc $(printf '%q' "exec >>~/seat.log 2>&1; echo \"== \$(date -Is) seat session\"; $(quote "$@")")"
    log "started on seat0/VT 7, log: dev/vm.sh ssh tail -f ~/seat.log"
    ;;
seat-stop)
    vm_ssh 'sudo systemctl stop consoleui-seat 2>/dev/null; sudo systemctl reset-failed consoleui-seat 2>/dev/null; sudo chvt 1' || true
    ;;
screenshot)
    [ -n "${2:-}" ] || die "usage: dev/vm.sh screenshot <file.png>"
    out="$(realpath -m "$2")"
    mkdir -p "$(dirname "$out")"
    qmp "{\"execute\": \"screendump\", \"arguments\": {\"filename\": \"$out\", \"format\": \"png\"}}" >/dev/null
    echo "$out"
    ;;
console)
    exec tail -f "$SERIAL_LOG"
    ;;
destroy)
    stop_vm
    rm -rf "$DISK" "$SEED" "$VM_DIR/seed" "$KEY" "$KEY.pub" "$SERIAL_LOG" "$VM_DIR"/*.log
    log "deleted the VM (kept $VM_DIR/$IMAGE_NAME)"
    ;;
*)
    sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
