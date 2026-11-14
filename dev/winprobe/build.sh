#!/usr/bin/env bash
# Builds the Windows stand-in game (probe.c) into target/winprobe/probe.exe, in the dev container.
# Run it with Proton outside Steam: dev/winprobe/run.sh
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$REPO_DIR/target/winprobe"
exec "$REPO_DIR/dev/toolbox.sh" run x86_64-w64-mingw32-gcc -O2 -mwindows -o target/winprobe/probe.exe \
    dev/winprobe/probe.c -ld3d11 -ldxgi -lxinput -luuid -lm
