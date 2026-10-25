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
cargo run --release -- --windowed --dummy --fake-network --start settings --actions "down,down,down,down,confirm" --snapshot /tmp/wifi.png
cargo test --workspace
cargo test -p consoleui-core carousel   # single test / filter
cargo clippy --workspace --all-targets -- -D warnings  # must pass (Definition of Done)
```

Quit the running shell with Ctrl+Q. Pressing Enter on a real entry really launches that game or app.
Use `--snapshot` to check UI changes visually instead of screenshotting the desktop: a screen capture
can pick up the user's other windows. Set `XDG_CONFIG_HOME` to a scratch directory when scripted actions
touch the settings page (otherwise they overwrite the user's `~/.config/consoleui/settings.json`), and
keep in mind that UI sounds really play through the speakers unless `"sounds": false` is set there.
Snapshot, `--actions` and `--bench` start only after the boot screen is gone (library loaded + 1.2 s);
with `--actions` the snapshot waits until the script is done (+0.7 s).
The WLAN/Bluetooth pages really change system state on ✕/□/≡ (connect, radio, discovery, pairing).
Use `--fake-network` for scripted tests; without it, only opening the pages is read-only.

Console session (see `docs/spike-gamescope.md`; gamescope runs in Steam mode `-e` and the shell acts as Steam):

```bash
session/consoleui-nested   # simulate the console session in a desktop window (nested gamescope -e)
sudo session/install.sh   # after `cargo build --release`; installs to /usr/local/bin + wayland-sessions
```

Inside gamescope, children get `DISPLAY=:1` and an empty `WAYLAND_DISPLAY`, so the shell and everything
it launches run through XWayland. In Steam mode only windows with a `STEAM_GAME` atom are shown at
all: the shell (`src/gamescope.rs`, x11rb) tags every new top-level window, resolving `_NET_WM_PID`
through `consoleui_core::ownership` (`SteamAppId` env, systemd scope cgroup, parent chain). It picks
the visible app with `GAMESCOPECTRL_BASELAYER_APPID` (`[app, SHELL_APP_ID]` or `[SHELL_APP_ID]`). Gamescope
mode is enabled only with `CONSOLEUI_GAMESCOPE=steam`, which the session script sets.

End-to-end test without real input (nested gamescope, screenshots of gamescope's own output):

```bash
CONSOLEUI_GAMESCOPE=steam gamescope -e -W 960 -H 540 -- target/release/consoleui --dummy --windowed \
  --dummy-launch glxgears --actions "wait:1500,confirm,wait:4000,guide"
# inside gamescope: xprop -root -f GAMESCOPECTRL_REQUEST_SCREENSHOT 32c -set GAMESCOPECTRL_REQUEST_SCREENSHOT 2
#   writes /tmp/gamescope.png; type 2 = full composition (type 1 omits overlays)
```

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
  - `settings.rs` (persisted `Settings`, the rows of the settings page), `sound.rs` (synthesized UI sounds
    as raw PCM), `input::evdev` (key codes/axis normalization for the exclusive grab).
  - `wifi.rs` / `bluetooth.rs`: D-Bus-free models for NetworkManager/BlueZ (merge/sort networks, AP
    security flags, device kind from `Icon`/Class/name, visible devices). `network.rs`: `Request`,
    `Response`, the `Backend` trait and `spawn` (backend thread, coalesces refreshes); `fakenet.rs` is the
    simulated backend (`--fake-network`); `netpages.rs` holds the WLAN/Bluetooth sub-page state (focus,
    password entry via `OnScreenKeyboard::password()` with a Shift layer, remove confirmation, busy
    requests) and builds plain view rows.
  - `status.rs` reads network/battery from sysfs; `system.rs` wraps `wpctl` (volume) and
    `systemctl suspend|reboot|poweroff`; `accent::from_rgba` derives tile colors from covers.
- `crates/shell` (binary `consoleui`): Slint UI plus glue.
  - `ui/main.slint`: `AppWindow`, the home screen. All sizes scale from a 1920×1080 reference via the
    `s` factor; tiles are positioned absolutely and animated (`animate x, y, width, height`).
  - `ui/theme.slint`: the `Theme` global with design tokens (colors, durations). Use it instead of
    hard-coded values.
  - `ui/types.slint`: `NavAction` enum and `AppTile` struct shared with Rust.
  - `src/shell.rs`: `Shell` owns entries plus a parallel `tiles` vector, all `running` apps (each with a
    gamescope app ID, a stop handle and an `Owner`), the `foreground` app, the three tile models (home =
    first 12 entries + library tile, library view, search results), the current `Screen`, focus, history,
    the hero cache, the tracked running app and the quit confirmation. `Shell::apply(action)` is the single
    entry point for input; it dispatches by screen and ends with `sync()` (screen, focus, hero, hints).
    `refresh()` rebuilds the models after a launch, an app exit or a re-sort. The hero background uses two
    image layers (`hero-a`/`hero-b` + `hero-front-a`) for crossfades; a launch moves the entry to index 0
    under the fullscreen launch overlay.
  - `src/tiles.rs`: `AppEntry` → `AppTile` (loads cover, icon and logo, overrides the accent from the cover).
  - `src/loader.rs`: background thread that discovers the library and decodes covers/logos with the
    `image` crate into `SharedPixelBuffer`s (Slint `Image`s are created on the UI thread in `tiles.rs`);
    `main.rs` polls the channel, calls `Shell::set_library` and then hides the boot screen.
  - `src/audio.rs`: persistent `pw-cat --raw` process fed by a writer thread. `Shell::apply` compares a
    `Feedback` snapshot before/after each action to pick the sound.
  - `src/grab.rs`: while the quick menu is open, `EVIOCGRAB` on the controller's evdev nodes (matched by
    gilrs `os_name`) and read them directly so the game in front gets no input.
  - `src/gamepad.rs`: gilrs polling → `Action`s (only devices with an SDL mapping); PS/Guide → `Action::Guide`.
  - `src/gamescope.rs`: X11 control of gamescope (tagging windows, baselayer, preparing the overlay window).
  - `src/quick.rs`: quick menu ("Steuerzentrale") items and input handling. In gamescope it is shown in
    `OverlayWindow` (`ui/overlay.slint`, transparent, gets `STEAM_OVERLAY` and is remapped before first
    use); otherwise it is the `QuickMenu` layer inside `AppWindow`.
  - `src/main.rs`: wiring and timers: gamepad poll (8 ms), status/clock (1 s), app watcher (500 ms), bench.
  - `ui/statusbar.slint`: the right side of the top bar; icons in `ui/icons/*.svg` are tinted with `colorize`.
  - `ui/home.slint`, `ui/library.slint`, `ui/search.slint`: one component per `Screen`, all stacked in
    `main.slint` and cross-faded. Each exposes `focus-x/-y/-size` so the launch overlay starts on the right
    tile. `library.slint` computes `columns` from the window width; Rust reads it for grid navigation.
  - `ui/theme.slint`: `Theme.accent` and all durations are `in-out` and set from Rust (`Shell::apply_settings`),
    for the main window and the overlay window separately (globals are per component instance).
  - `ui/settings.slint`, `ui/boot.slint`, `ui/toast.slint`: settings page, boot screen, toast.
  - `ui/glyph.slint`: `ButtonGlyph` (button symbol per `ControllerStyle`) and `HintBar`; Rust fills `hints`.
  - `src/dbus.rs`: real WLAN/Bluetooth backend (zbus blocking API on the system bus, own BlueZ pairing
    agent); only ever runs on the network thread. `src/network.rs`: glue between `NetPages`, the thread
    and the `NetView` Slint struct (`ui/network.slint`, shown over the settings page while a sub-page is
    open). `main.rs` polls responses every 100 ms (`Shell::poll_network`); results become toasts.
  - `src/perf.rs`: frame stats from Slint's rendering notifier. `src/cover.rs`: procedural
    placeholder covers. `src/options.rs`: hand-rolled CLI parsing.

Input flow: keyboard events are handled in the `FocusScope` in `main.slint`, which emits
`action(NavAction)`; gamepad events come from `Gamepads::poll`. Both end up in `Shell::apply(Action)`,
and Rust pushes properties back (`focused`, `pulse`, `launching`, …). New input sources must use the same path.
In the search screen the `FocusScope` sends printable keys to `typed(text)` instead of the letter
shortcuts (WASD, Q/E, F, /). Gamepad input is dropped while the window is not focused
(`window-active`), because gilrs reads devices directly and would otherwise react to presses meant
for the game in front. Exceptions (`Shell::accepts_unfocused`): the Guide button in gamescope mode and
everything while the quick menu is open. `--actions` script steps bypass the gating.

Note: `[profile.dev]` uses `opt-level = 1` and dependencies use `opt-level = 3`, so debug builds
render smoothly.

The git repository root is this `ConsoleUI/` directory.
