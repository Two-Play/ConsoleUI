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
cargo run -- --windowed --dummy    # made-up entries instead of the real Steam/desktop library
cargo run --release -- --windowed --dummy --tiles 50 --bench   # perf benchmark, prints results and exits
cargo run -p consoleui-core --example list_library   # print what discovery finds on this machine
cargo test --workspace
cargo test -p consoleui-core carousel   # single test / filter
cargo clippy --workspace --all-targets -- -D warnings  # must pass (Definition of Done)
```

Quit the running shell with Ctrl+Q. Pressing Enter on a real entry really launches that game or app.

## Architecture

Cargo workspace (edition 2024, `default-members` = the shell):

- `crates/core` (`consoleui-core`): UI-free logic with unit tests. New logic belongs here when it can
  be tested without Slint.
  - `AppEntry` (in `library.rs`) is the common model for every source. Its `id` is prefixed by source
    (`steam:<appid>`, `desktop:<desktop-file-id>`, `dummy:<n>`), and that ID is also the key in the history.
    `Artwork` holds only paths; images are loaded in the shell.
  - `discover_library()` = `steam::discover` (own KeyValues parser in `vdf.rs`) + `desktop::discover`
    (own `.desktop` parser, category allow/deny lists, `icons.rs` lookup). Then `apply_history` +
    `sort_by_recent` (most recent first, then alphabetical).
  - `launch::spawn` starts detached processes; `History` persists to `$XDG_DATA_HOME/consoleui/history.json`.
  - `Action` / `Carousel` handle input-agnostic navigation; `accent::from_rgba` derives tile colors from covers.
- `crates/shell` (binary `consoleui`): Slint UI plus glue.
  - `ui/main.slint`: `AppWindow`, the home screen. All sizes scale from a 1920×1080 reference via the
    `s` factor; tiles are positioned absolutely and animated (`animate x, y, width, height`).
  - `ui/theme.slint`: the `Theme` global with design tokens (colors, durations). Use it instead of
    hard-coded values.
  - `ui/types.slint`: `NavAction` enum and `AppTile` struct shared with Rust.
  - `src/shell.rs`: `Shell` owns entries, the `VecModel<AppTile>`, focus, history and the hero cache.
    `Shell::apply(action)` is the single entry point for input. The hero background uses two image layers
    (`hero-a`/`hero-b` + `hero-front-a`) for crossfades; a launch moves the entry to index 0 under the
    fullscreen launch overlay.
  - `src/tiles.rs`: `AppEntry` → `AppTile` (loads cover, icon and logo, overrides the accent from the cover).
  - `src/main.rs`: wiring, `NavAction` → `core::Action`, the clock and benchmark timers.
  - `src/perf.rs`: frame stats from Slint's rendering notifier. `src/cover.rs`: procedural
    placeholder covers. `src/options.rs`: hand-rolled CLI parsing.

Input flow: keyboard events are handled in the `FocusScope` in `main.slint`, which emits
`action(NavAction)`. Rust updates state and pushes properties back (`focused`, `pulse`). Gamepad
input (Story 3.1, `gilrs`) must feed the same `Action` path, not a separate one.

Note: `[profile.dev]` uses `opt-level = 1` and dependencies use `opt-level = 3`, so debug builds
render smoothly.

The git repository root is this `ConsoleUI/` directory.
