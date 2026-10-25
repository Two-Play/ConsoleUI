#!/usr/bin/env bash
# Development container for ConsoleUI (Fedora toolbox).
#
# The container holds the -devel packages and test tools, so the host stays clean. The source code
# stays on the host: toolbox mounts $HOME, so the container works on this checkout directly and uses
# the host's rustup toolchain in ~/.cargo. Builds inside the container go to target/toolbox, so
# they never invalidate the host's target/ directory.
#
# GPU: the container has Mesa only (llvmpipe/lavapipe), not the host's NVIDIA userspace driver.
# Everything that must run on the NVIDIA GPU (gamescope performance, real games) is tested on the
# host. Wayland/X11 windows from the container appear on the host desktop.
#
#   dev/toolbox.sh create          create the container and install the packages (idempotent)
#   dev/toolbox.sh run <cmd...>    run a command in the container, in this checkout
#   dev/toolbox.sh enter           interactive shell in the container
#   dev/toolbox.sh remove          delete the container (the code is not affected)
set -euo pipefail

NAME=consoleui-dev
RELEASE=44
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PACKAGES=(
    # Toolchain basics (Rust itself comes from the host's rustup in ~/.cargo)
    gcc gcc-c++ make cmake pkgconf-pkg-config git
    # Shell: Slint (winit, femtovg), gilrs (libudev), fonts
    fontconfig-devel freetype-devel libxkbcommon-devel libxkbcommon-x11-devel wayland-devel
    systemd-devel libX11-devel libxcb-devel
    # Compositor (Epic 7, Smithay with the udev backend)
    libinput-devel mesa-libgbm-devel libseat-devel libdrm-devel libdisplay-info-devel libei-devel
    mesa-libEGL-devel mesa-libGLES-devel pixman-devel xorg-x11-server-Xwayland
    # Runtime and test tools (Mesa software rendering, nested runs, snapshots)
    mesa-dri-drivers mesa-vulkan-drivers vulkan-tools mesa-demos gamescope
    xprop xdpyinfo wayland-utils pipewire-utils
)

exists() { podman container exists "$NAME" 2>/dev/null; }

case "${1:-}" in
create)
    if ! exists; then
        toolbox create --assumeyes --release "$RELEASE" "$NAME"
    fi
    toolbox run -c "$NAME" sudo dnf install -y --setopt=install_weak_deps=False "${PACKAGES[@]}"
    echo "Container '$NAME' is ready. Try: dev/toolbox.sh run cargo test --workspace"
    ;;
run)
    shift
    exists || { echo "No container '$NAME' yet, run: dev/toolbox.sh create" >&2; exit 1; }
    # Login shells in the container may not have ~/.cargo/bin on PATH.
    exec toolbox run -c "$NAME" env PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$REPO_DIR/target/toolbox" \
        bash -c 'cd "$0" && exec "$@"' "$REPO_DIR" "$@"
    ;;
enter)
    exists || { echo "No container '$NAME' yet, run: dev/toolbox.sh create" >&2; exit 1; }
    exec toolbox enter "$NAME"
    ;;
remove)
    podman rm -f "$NAME"
    ;;
*)
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
