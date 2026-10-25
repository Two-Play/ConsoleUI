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
cargo run --release -- --windowed --start library --snapshot /tmp/lib.png   # render own window to PNG and exit
cargo run --release -- --windowed --start search --query red --snapshot /tmp/search.png
cargo test --workspace
cargo test -p consoleui-core carousel   # single test / filter
cargo clippy --workspace --all-targets -- -D warnings  # must pass (Definition of Done)
```

Quit the running shell with Ctrl+Q. Pressing Enter on a real entry really launches that game or app.
Use `--snapshot` to check UI changes visually instead of screenshotting the desktop: a screen capture
can pick up the user's other windows.

Console session (see `docs/spike-gamescope.md` for why gamescope runs without `-e`):

```bash
CONSOLEUI_BIN=$PWD/target/release/consoleui CONSOLEUI_GAMESCOPE_ARGS="-W 1280 -H 720" session/consoleui-session   # nested test
sudo session/install.sh   # after `cargo build --release`; installs to /usr/local/bin + wayland-sessions
```

Inside gamescope, children get `DISPLAY=:1` and an empty `WAYLAND_DISPLAY`, so the shell and everything
it launches run through XWayland. Focus always goes to the newest window, so the shell never switches
windows itself; it only detects when the launched app ends.

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
  - `launch::spawn` starts detached processes (desktop apps in a `systemd-run --user --scope` unit when the
    user bus exists) and returns a `ProcessHandle` that can `stop()`; Steam games are stopped via
    `steam::stop_game` (SIGTERM to processes with `SteamAppId=<id>`). `History` persists to
    `$XDG_DATA_HOME/consoleui/history.json`.
  - `Action` / `Carousel` (row) / `Grid` (library) handle input-agnostic navigation; `osk.rs` is the
    on-screen keyboard; `search.rs` ranks titles (`normalize` folds case and umlauts, also used for A–Z
    sorting); `library_view` applies the library filter and sort.
  - `input.rs` has the device-free gamepad logic
    (`DirectionRepeat` for hold-to-repeat, `Stick` with hysteresis, `glyph_style` by device name).
  - `running::RunningApp` decides when the launch overlay closes: process exit for commands,
    `steam::running_app_id()` (reads `~/.steam/registry.vdf`) for Steam games, with a minimum hold time.
  - `status.rs` reads network/battery from sysfs; `accent::from_rgba` derives tile colors from covers.
- `crates/shell` (binary `consoleui`): Slint UI plus glue.
  - `ui/main.slint`: `AppWindow`, the home screen. All sizes scale from a 1920×1080 reference via the
    `s` factor; tiles are positioned absolutely and animated (`animate x, y, width, height`).
  - `ui/theme.slint`: the `Theme` global with design tokens (colors, durations). Use it instead of
    hard-coded values.
  - `ui/types.slint`: `NavAction` enum and `AppTile` struct shared with Rust.
  - `src/shell.rs`: `Shell` owns entries plus a parallel `tiles` vector, the three tile models (home =
    first 12 entries + library tile, library view, search results), the current `Screen`, focus, history,
    the hero cache, the tracked running app and the quit confirmation. `Shell::apply(action)` is the single
    entry point for input; it dispatches by screen and ends with `sync()` (screen, focus, hero, hints).
    `refresh()` rebuilds the models after a launch, an app exit or a re-sort. The hero background uses two
    image layers (`hero-a`/`hero-b` + `hero-front-a`) for crossfades; a launch moves the entry to index 0
    under the fullscreen launch overlay.
  - `src/tiles.rs`: `AppEntry` → `AppTile` (loads cover, icon and logo, overrides the accent from the cover).
  - `src/gamepad.rs`: gilrs polling → `Action`s (only devices with an SDL mapping).
  - `src/main.rs`: wiring and timers: gamepad poll (8 ms), status/clock (1 s), app watcher (500 ms), bench.
  - `ui/statusbar.slint`: the right side of the top bar; icons in `ui/icons/*.svg` are tinted with `colorize`.
  - `ui/home.slint`, `ui/library.slint`, `ui/search.slint`: one component per `Screen`, all stacked in
    `main.slint` and cross-faded. Each exposes `focus-x/-y/-size` so the launch overlay starts on the right
    tile. `library.slint` computes `columns` from the window width; Rust reads it for grid navigation.
  - `ui/glyph.slint`: `ButtonGlyph` (button symbol per `ControllerStyle`) and `HintBar`; Rust fills `hints`.
  - `src/perf.rs`: frame stats from Slint's rendering notifier. `src/cover.rs`: procedural
    placeholder covers. `src/options.rs`: hand-rolled CLI parsing.

Input flow: keyboard events are handled in the `FocusScope` in `main.slint`, which emits
`action(NavAction)`; gamepad events come from `Gamepads::poll`. Both end up in `Shell::apply(Action)`,
and Rust pushes properties back (`focused`, `pulse`, `launching`, …). New input sources must use the same path.
In the search screen the `FocusScope` sends printable keys to `typed(text)` instead of the letter
shortcuts (WASD, Q/E, F, /). Gamepad input is dropped while the window is not focused
(`window-active`), because gilrs reads devices directly and would otherwise react to presses meant
for the game in front.

Note: `[profile.dev]` uses `opt-level = 1` and dependencies use `opt-level = 3`, so debug builds
render smoothly.

The git repository root is this `ConsoleUI/` directory.
