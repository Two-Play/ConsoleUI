# Test VM for the compositor

`dev/vm.sh` runs a Fedora 44 guest in plain QEMU/KVM as the normal user (no libvirt, no sudo,
nothing installed on the host). The guest has its own seat, VTs and DRM device (`virtio-vga`), so the
DRM/KMS backend of `consoleui-compositor --drm` and the login session can be tested without touching
the host's display or GPU. The code stays on the host: the main checkout is shared with virtiofs at
the same absolute path, so binaries built in the toolbox (`target/toolbox/release/…`, also Fedora 44)
run directly in the guest.

## Usage

```bash
dev/vm.sh create                 # once (~10 min): download + verify the image, first boot installs packages
dev/vm.sh start                  # display in a QEMU window; `start --headless` for scripted runs
dev/vm.sh try                    # try it by hand: compositor --drm + shell (dummy entries, glxgears) in the window
dev/vm.sh try --real             # same with the guest's real library; extra args go to the shell
dev/vm.sh ssh 'ls target/toolbox/release'      # runs in the shared checkout, as user `tester` (uid 1000)
dev/vm.sh seat target/toolbox/release/consoleui-compositor --drm   # on seat0, VT 7, log: ~/seat.log
dev/vm.sh screenshot /tmp/vm.png # QMP screendump of the guest display, also headless (2nd arg: head)
dev/vm.sh keys ctrl-alt-f2 sleep:1500 ctrl-alt-f7 right ret   # real key events (QMP send-key)
dev/vm.sh output 0 1600x900      # new preferred mode for monitor 0; `output 1 1024x768` / `output 1 off`
dev/vm.sh seat-stop              # stop the seat session, back to VT 1
dev/vm.sh stop                   # clean poweroff
dev/vm.sh destroy                # delete disk, seed and SSH key (keeps the downloaded image)
```

`dev/vm.sh scp file :/tmp/` copies files that are not in the checkout (guest paths start with `:`),
`dev/vm.sh status` shows whether SSH works, the share is mounted and which logind sessions exist,
`dev/vm.sh console` follows the serial console.

Settings via environment: `VM_SHARE` (default: the main checkout, also when the script runs from a
worktree below `.claude/worktrees`), `VM_CPUS` (4), `VM_MEM` (6G), `VM_DISK` (20G overlay), `VM_RES`
(1280x800, the virtual monitor's mode; in a window the guest's preferred mode follows the
window size instead, so use `--headless` for reproducible screenshots), `VM_SSH_PORT` (2222, bound to 127.0.0.1),
`CONSOLEUI_VM_DIR` (`~/.local/share/consoleui-vm`), `VM_HEADS` (1; number of virtual monitors).

A second, independent VM (e.g. while another agent uses the first one) needs its own state dir and
port; copying the downloaded image and `CHECKSUM` there first saves the download:
`CONSOLEUI_VM_DIR=~/.local/share/consoleui-vm-b VM_SSH_PORT=2223 VM_MEM=4G VM_HEADS=2 dev/vm.sh create`
(then the same variables for every other command).

## Real keys, monitors and the login screen

- `keys` sends real key events through QMP `send-key` (QEMU qcodes: `a`, `ret`, `esc`, `tab`,
  `up`, `f1`, `slash`, `ctrl`, `alt`, `shift`, …; `-` joins a chord, `sleep:MS` waits). They
  arrive as a virtio keyboard through libinput, like a real keyboard, and do not need the window.
  Use them instead of the shell's `--actions` when keyboard focus matters. The guest keymap
  (`localectl`) is `us`, so qcodes match the printed keys unless the shell's "Keyboard layout"
  setting picks another layout (with `de`, qcode `y` types `z`).
- `output <head> <WxH|off>` acts like a client resizing a QEMU display: a minimal VNC client on
  `vnc<head>.sock` sends SetDesktopSize, QEMU passes it to virtio-gpu as the monitor's preferred
  mode and the guest gets a hotplug event; `off` (0x0) disconnects the head. Heads above 0 exist
  only with `VM_HEADS=2` (or more) and start disconnected. `screenshot <png> <head>` captures one.
- Display manager: `sudo dnf install sddm sddm-wayland-generic` in the guest (not enabled at
  boot, so `seat` keeps working), `sudo session/install.sh` from the shared checkout (takes the
  toolbox builds), then
  `printf '[General]\nDisplayServer=wayland\n[Wayland]\nCompositorCommand=weston --shell=kiosk-shell.so\n' | sudo tee /etc/sddm.conf.d/10-test.conf`
  and `sudo systemctl start sddm`: the greeter lists "ConsoleUI (own compositor)". The test user
  has no password; for a login without typing one, add `[Autologin]` with `User=tester`,
  `Session=consoleui-compositor` and `Relogin=false`. Quitting the shell ends the session and SDDM
  shows the greeter again. The session's output is in the journal:
  `journalctl --user -t consoleui -b`. `sudo systemctl stop sddm` when done.

## What is where

- `~/.local/share/consoleui-vm/`: the Fedora Cloud Base Generic image (checked against the
  `CHECKSUM` file, whose GPG signature is checked with the host's
  `/etc/pki/rpm-gpg/RPM-GPG-KEY-fedora-44-primary`), `disk.qcow2` (overlay on the image), the
  cloud-init NoCloud seed (`seed.iso`, source in `seed/`), the VM's SSH key `id_ed25519`, logs and
  the QMP/virtiofs sockets.
- Guest: user `tester` (uid 1000 like the host user, so files on the share keep their owner;
  passwordless sudo; groups video, render, input, audio), console autologin on tty1, the share at
  `$CONSOLEUI_REPO`. Packages: Mesa (llvmpipe, kms_swrast, lavapipe), libinput, libseat, seatd,
  libdisplay-info, libei, Xwayland, gamescope, kmscube, mesa-demos, weston, foot, pipewire,
  wireplumber, fonts. The first boot also upgrades all packages (the toolbox is up to date, and an
  outdated `libstdc++` breaks gamescope).

## Running something on the seat

The DRM backend needs a logind session that is on seat0 and active. `dev/vm.sh seat <cmd>` starts
the command like a display manager would:

```bash
sudo chvt 7
sudo systemd-run --unit=consoleui-seat --uid=tester -p PAMName=login -p TTYPath=/dev/tty7 \
    -p StandardInput=tty -p Environment=XDG_SESSION_TYPE=wayland … /bin/bash -lc '<cmd>'
```

`PAMName=login` runs `pam_systemd`, which registers a session for tty7 on seat0
(`loginctl show-session` reports `Seat=seat0 VTNr=7 Type=wayland Active=yes`). VT 7 is above
logind's `NAutoVTs=6`, so no getty competes for it, and `chvt 7` makes the session the active one.
libseat (logind backend) then gets the DRM and input devices through `TakeDevice`. Only one seat
command runs at a time (unit `consoleui-seat`); a new `seat` call replaces it. Output goes to
`~/seat.log` in the guest. Wayland clients started over SSH reach the compositor with
`WAYLAND_DISPLAY=wayland-1` (same user, same `XDG_RUNTIME_DIR=/run/user/1000`).

## Verified with Smithay's anvil as stand-in

- `dev/vm.sh seat kmscube`: renders on the virtio-vga output (llvmpipe).
- anvil (Smithay `24647fc8d001`, built in the toolbox) with `--tty-udev` on seat0 through libseat +
  logind: output `Virtual-1`, libinput, rendering with llvmpipe via kms_swrast.
- `target/toolbox/release/consoleui --windowed --dummy` as a Wayland client inside that anvil
  (scratch `XDG_*` dirs, `{"sounds": false}`).

Unmodified anvil refuses to start: it rejects software EGL devices (`egl_device.is_software()` →
`NoRenderNode`) and then panics with `PrimaryGpuMissing`. The test used a one-line patch that skips
the check when `ANVIL_ALLOW_SOFTWARE` is set, plus `ANVIL_DRM_DEVICE=/dev/dri/card0` (the render node
of virtio-gpu without virgl has no hardware EGL device):

```bash
dev/vm.sh scp <smithay>/target/release/anvil :
dev/vm.sh seat 'ANVIL_ALLOW_SOFTWARE=1 ANVIL_DRM_DEVICE=/dev/dri/card0 ~/anvil --tty-udev'
```

For Story 7.2: our `--drm` backend must accept a software EGL device (kms_swrast) on the primary
node, otherwise it cannot run in this VM. anvil logs "Unable to become drm master, assuming
unprivileged mode" here; modesetting works anyway, because the fd from logind's `TakeDevice` is
already DRM master.

## Limits

- No GPU acceleration: everything renders on the CPU (llvmpipe/lavapipe). Good for correctness of
  DRM/KMS, libinput, session and VT handling, not for performance. Host GL/virgl/venus are
  deliberately not used, so a fault can never hit the host's NVIDIA driver.
- `gamescope --backend drm` does not run: it needs a Vulkan device with a DRM node, lavapipe has
  none ("vulkan: not a valid physical device"). A nested `gamescope --backend wayland` inside anvil
  starts, but anvil drops it with a protocol error after a few seconds (not investigated).
- Monitors are virtual (virtio-gpu heads): hotplug and mode changes work through `output`, but
  there is no real EDID, HDR, VRR or refresh rate choice (QEMU reports 75 Hz).
- Gamepads: `VM_USB=054c:0ce6 dev/vm.sh start` passes a USB device (here a DualSense) through with
  `usb-host`; the host loses the device while the VM runs. QEMU needs write access to
  `/dev/bus/usb/BBB/DDD`; controllers covered by the `steam-devices` udev rules already have a
  `uaccess` ACL for the logged-in user, others need `sudo setfacl -m u:$USER:rw /dev/bus/usb/BBB/DDD`
  (until unplug). Bluetooth controllers cannot be passed through this way. Keyboard and mouse
  in the QEMU window arrive as virtio keyboard/tablet devices through libinput.
- No audio device in the guest (PipeWire runs with a dummy sink), keep `"sounds": false`.
