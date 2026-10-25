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
dev/vm.sh screenshot /tmp/vm.png # QMP screendump of the guest display, also headless
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
`CONSOLEUI_VM_DIR` (`~/.local/share/consoleui-vm`).

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
- One virtual monitor (`virtio-vga` exposes a single output; hotplug/multi-monitor needs more
  heads via `-device virtio-gpu-pci,max_outputs=2`, not wired up).
- Gamepads: `VM_USB=054c:0ce6 dev/vm.sh start` passes a USB device (here a DualSense) through with
  `usb-host`; the host loses the device while the VM runs. QEMU needs write access to
  `/dev/bus/usb/BBB/DDD`; controllers covered by the `steam-devices` udev rules already have a
  `uaccess` ACL for the logged-in user, others need `sudo setfacl -m u:$USER:rw /dev/bus/usb/BBB/DDD`
  (until unplug). Bluetooth controllers cannot be passed through this way. Keyboard and mouse
  in the QEMU window arrive as virtio keyboard/tablet devices through libinput.
- No audio device in the guest (PipeWire runs with a dummy sink), keep `"sounds": false`.
