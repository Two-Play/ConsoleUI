# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ConsoleUI is a gamepad-driven, PS5/Xbox-style console shell for Linux, written in Rust with Slint.
The roadmap, Scrum backlog (epics, story IDs, sprint plan) and sprint results live in `BACKLOG.md`;
reference story IDs (e.g. "Story 1.5") in code comments when deferring work. The user communicates
in German, and UI strings and comments are German.

Planned target architecture: the shell runs as a fullscreen client inside **gamescope** (like Steam's
Game Mode); an own Smithay compositor is an optional later phase. Keep the shell independent of the
compositor.

## Commands

Rust is installed via rustup; in non-login shells `~/.cargo/bin` may be missing from `PATH`.

```bash
cargo run -- --windowed            # run the shell in a window (default: fullscreen)
cargo run -- --windowed --fps      # with FPS / render-time overlay
cargo run --release -- --windowed --tiles 50 --bench   # perf benchmark, prints results and exits
cargo test --workspace
cargo test -p consoleui-core carousel   # single test / filter
cargo clippy --workspace --all-targets -- -D warnings  # must pass (Definition of Done)
```

Quit the running shell with Ctrl+Q.

## Architecture

Cargo workspace (edition 2024, `default-members` = the shell):

- `crates/core` (`consoleui-core`): UI-free logic with unit tests: the input-agnostic `Action` enum,
  `Carousel` focus navigation, and the `AppEntry` library model (currently `dummy_library`; Steam and
  `.desktop` discovery arrive in Sprint 2). New logic belongs here when it can be tested without Slint.
- `crates/shell` (binary `consoleui`): Slint UI plus glue.
  - `ui/main.slint`: `AppWindow`, the home screen. All sizes scale from a 1920×1080 reference via the
    `s` factor; tiles are positioned absolutely and animated (`animate x, y, width, height`).
  - `ui/theme.slint`: the `Theme` global with design tokens (colors, durations). Use it instead of
    hard-coded values.
  - `ui/types.slint`: `NavAction` enum and `AppTile` struct shared with Rust.
  - `src/main.rs`: wires the callbacks, maps `NavAction` → `core::Action` → `apply_action`, and
    runs the clock and benchmark timers.
  - `src/perf.rs`: frame stats from Slint's rendering notifier. `src/cover.rs`: procedural
    placeholder covers. `src/options.rs`: hand-rolled CLI parsing.

Input flow: keyboard events are handled in the `FocusScope` in `main.slint`, which emits
`action(NavAction)`. Rust updates state and pushes properties back (`focused`, `pulse`). Gamepad
input (Story 3.1, `gilrs`) must feed the same `Action` path, not a separate one.

Note: `[profile.dev]` uses `opt-level = 1` and dependencies use `opt-level = 3`, so debug builds
render smoothly.

The git repository root is this `ConsoleUI/` directory.
