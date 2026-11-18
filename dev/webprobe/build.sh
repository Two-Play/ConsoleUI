#!/usr/bin/env bash
# Builds the web view spike program (Spike 11.1) into target/webprobe/release/webprobe, in the dev container.
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
exec "$REPO_DIR/dev/toolbox.sh" run cargo build --release --manifest-path dev/webprobe/Cargo.toml \
    --target-dir target/webprobe
