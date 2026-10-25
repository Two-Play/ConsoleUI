# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ConsoleUI is a gamepad-driven, PS5/Xbox-style console shell for Linux, written in Rust with Slint.
The roadmap, Scrum backlog (epics, story IDs, sprint plan) and sprint results live in `BACKLOG.md`;
reference story IDs (e.g. "Story 1.5") in code comments when deferring work. The user communicates
in German, but everything in the repository is written in English from now on: code (identifiers),
comments, UI strings, `BACKLOG.md` and the docs. Older parts are still German: whenever you change a
piece of code, a UI string or a section of a document, translate the comments, strings or that
section to English as part of the change. The German UI comes from the translation catalog (see
"Translations" below).

Target architecture (hybrid, Epic 7): the shell is a fullscreen client of our own Smithay compositor
(`crates/compositor`); games that need upscaling/HDR run in a nested gamescope per entry (Story 7.10).
The gamescope session (shell inside `gamescope -e`, like Steam's Game Mode) stays as a fallback, and the
shell also runs as a plain window on any desktop. Keep the shell independent of the compositor: it only
talks to it through the control socket.

## Commands

Rust is installed via rustup; in non-login shells `~/.cargo/bin` may be missing from `PATH`.

```bash
cargo run -- --windowed            # run the shell in a window (default: fullscreen)
cargo run -- --windowed --fps      # with FPS / render-time overlay
cargo run -- --windowed --dummy    # made-up entries instead of the real Steam/desktop library
cargo run --release -- --windowed --dummy --tiles 50 --bench   # perf benchmark (fps, render time, max frame gap,
                                                               # slowest input step on the UI thread), then exits
cargo run -p consoleui-core --example list_library   # print what discovery finds on this machine
cargo run --release -- --windowed --start library --snapshot /tmp/lib.png   # render own window to PNG and exit
cargo run --release -- --windowed --start search --query red --snapshot /tmp/search.png
cargo run --release -- --windowed --dummy --fake-network --start settings --actions "down,down,down,down,confirm" --snapshot /tmp/wifi.png
cargo test -p consoleui-core -p consoleui            # on the host (the compositor needs the dev container)
cargo test -p consoleui-core carousel   # single test / filter
dev/toolbox.sh run cargo test --workspace                                   # all crates incl. the compositor
dev/toolbox.sh run cargo clippy --workspace --all-targets -- -D warnings    # must pass (Definition of Done)
cargo fmt --all                          # rustfmt.toml: max_width 120, use_small_heuristics = "Max"
```

Development container (Fedora toolbox `consoleui-dev`, holds all `-devel` packages incl. the Smithay
udev backend plus test tools; the code stays on the host, `$HOME` is shared, builds go to
`target/toolbox`):

```bash
dev/toolbox.sh create                              # once; idempotent, also installs new packages
dev/toolbox.sh run cargo test --workspace          # any command, run in this checkout
dev/toolbox.sh run target/toolbox/release/consoleui --windowed --dummy   # windows appear on the desktop
```

The container has Mesa only (llvmpipe/lavapipe), not the NVIDIA userspace driver. Build, test and
snapshot there; run gamescope on the NVIDIA GPU, real games and performance checks on the host.
Run GPU tests (nested gamescope, compositors) one after another, never from parallel agents: a crashed
gamescope can leave the NVIDIA driver in a reset-required state until the next reboot.

Own compositor (Epic 7; build in the dev container, binaries in `target/toolbox/release`):

```bash
dev/toolbox.sh run cargo build --release --workspace
target/toolbox/release/consoleui-compositor --winit --size 960x540 \
  --screenshot-after 8000 /tmp/c.png -- target/release/consoleui --dummy --dummy-launch glxgears \
  --actions "wait:1500,confirm,wait:4000,guide"    # nested compositor + shell; PNG of the composed output
session/consoleui-nested --compositor --dummy      # nested session on our compositor instead of gamescope
```

Compositor options: `--winit`/`--drm` (default: winit when `WAYLAND_DISPLAY`/`DISPLAY` is set),
`--size WxH`, `--no-xwayland`, `--no-shell`, `--screenshot-after MS PNG` (repeatable), the shell command
after `--` or `--shell`. **Never run `--drm` on the host** (it takes over the display); test it in the VM.

Test VM for the DRM backend and the login session (`docs/dev-vm.md`): `dev/vm.sh create` once, then
`dev/vm.sh start [--headless]`. The main checkout is shared at the same path, so toolbox builds run in
the guest; `dev/vm.sh ssh '<cmd>'` runs a command there, `dev/vm.sh screenshot <png>` captures the guest
display, and `dev/vm.sh seat '<cmd>'` runs a command as an active logind session on seat0/VT 7 (log in
`~/seat.log` in the guest), which is what `consoleui-compositor --drm` needs. The guest renders with
llvmpipe only (no gamescope `--backend drm` there); the host GPU is never used. `dev/vm.sh stop` when done.

Quit the running shell with Ctrl+Q. Pressing Enter on a real entry really launches that game or app.
Use `--snapshot` to check UI changes visually instead of screenshotting the desktop: a screen capture
can pick up the user's other windows. Set `XDG_CONFIG_HOME` to a scratch directory when scripted actions
touch the settings page (otherwise they overwrite the user's `~/.config/consoleui/settings.json`), and
keep in mind that UI sounds really play through the speakers unless `"sounds": false` is set there.
Snapshot, `--actions` and `--bench` start only after the boot screen is gone (library loaded + 1.2 s);
with `--actions` the snapshot waits until the script is done (+0.7 s).
The WLAN/Bluetooth pages really change system state on ✕/□/≡ (connect, radio, discovery, pairing).
Use `--fake-network` for scripted tests; without it, only opening the pages is read-only.
A scratch `XDG_DATA_HOME` (to keep the history untouched) also hides `~/.local/share/applications`;
prepend `$HOME/.local/share` to `XDG_DATA_DIRS` to still see those entries.

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
  --dummy-launch glxgears --actions "wait:1500,confirm,wait:4000,toast,wait:1000,guide"
# `toast` is a development step that raises a test toast (shown over the game here)
# inside gamescope: xprop -root -f GAMESCOPECTRL_REQUEST_SCREENSHOT 32c -set GAMESCOPECTRL_REQUEST_SCREENSHOT 2
#   writes /tmp/gamescope.png; type 2 = full composition (type 1 omits overlays)
```

## Architecture

Cargo workspace (edition 2024, `default-members` = the shell):

- `crates/core` (`consoleui-core`): UI-free logic with unit tests. New logic belongs here when it can
  be tested without Slint.
  - `AppEntry` (in `library.rs`) is the common model for every source. Its `id` is prefixed by source
    (`steam:<appid>`, `desktop:<desktop-file-id>`, `dummy:<n>`), and that ID is also the key in the history.
    `Artwork` holds only paths; images are loaded in the shell. `title`/`description` are the
    current-language texts; `names` (`Names`: Fixed / Dummy / `Arc<DesktopNames>` with only the supported
    languages' `.desktop` variants) keeps their sources, and `localize_entries` re-resolves them when
    `Shell::apply_language` switches the language (followed by a re-sort of entries and tiles).
  - `discover_library()` = `steam::discover` (own KeyValues parser in `vdf.rs`) + `desktop::discover`
    (own `.desktop` parser, category allow/deny lists, `icons.rs` lookup). Then `apply_history` +
    `sort_by_recent` (most recent first, then alphabetical).
  - `launch::spawn` starts detached processes (desktop apps in a `systemd-run --user --scope` unit when the
    user bus exists) and returns a `ProcessHandle` that can `stop()`; Steam games are stopped via
    `steam::stop_game` (SIGTERM to processes with `SteamAppId=<id>`). `History` persists to
    `$XDG_DATA_HOME/consoleui/history.json`, `Collection` (pinned/hidden entry IDs, `collection.rs`)
    to `collection.json` next to it.
  - `Action` / `Carousel` (row) / `Grid` (library) handle input-agnostic navigation; `osk.rs` is the
    on-screen keyboard; `search.rs` ranks titles (`normalize` folds case and umlauts, also used for A–Z
    sorting); `library_view` applies the library filter (incl. `Hidden`) and sort, `home_view` builds the
    home row (pinned first, then recents); hidden entries are left out of home, library and search.
  - `input.rs` has the device-free gamepad logic
    (`DirectionRepeat` for hold-to-repeat, `Stick` with hysteresis, `glyph_style` by device name).
  - `running::RunningApp` decides when the launch overlay closes: process exit for commands,
    `steam::running_games()` for Steam games (scans `/proc` for processes with `SteamAppId=<id>`;
    `RunningAppID` in `~/.steam/registry.vdf` is updated by Steam only minutes later), with a minimum hold
    time and a 2 s grace period without game processes.
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
  - `game_options.rs` (Story 7.10): per-entry nested gamescope options (`GameOptions`: resolution,
    upscaler, FPS limit, HDR, fullscreen); `wrap()` puts `gamescope <args> --` in front of
    `Launch::Command` launches (Steam entries are not supported, Steam starts the game itself),
    `GameOptionsPanel` is the panel state, `GameOptionsStore` persists to `game_options.json` next to
    `collection.json`. `launch::plan` removes `ENABLE_GAMESCOPE_WSI` for a nested gamescope.
  - `i18n.rs` (Story 6.6): `Language` (from `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` or the settings
    row), `tr`/`trf`/`trn`/`tr_noop`, backed by the embedded German catalog (see "Translations" below).
  - `protocol.rs`: the shell ↔ compositor messages (`ShellMessage` / `CompositorMessage`), one JSON
    object per line tagged with `"type"`, `LineReader` and `encode`; unknown types and malformed lines
    are skipped and new fields are optional, so both sides can evolve independently.
- `crates/compositor` (binary `consoleui-compositor`, workspace member but not a default member; needs
  the dev container's `-devel` packages; Smithay pinned to git `24647fc8d001`): the session compositor of
  Epic 7. `winit.rs` (nested) and `drm.rs` (DRM/KMS + libinput + libseat, single GPU and output, accepts
  a software EGL device) sit behind `backend::Backend`. `state.rs` holds the globals and protocol
  handlers (dmabuf, drm-syncobj, presentation-time, relative-pointer, pointer-constraints, …).
  `policy.rs` is the Smithay-free, unit-tested window policy (owner shell/app/unknown, front, stacking,
  keyboard focus; every toplevel is fullscreen, an app comes to the front once its window shows
  content). `xwayland.rs` runs `X11Wm` and maps windows to apps via `_NET_WM_PID` or the X-Resource
  client PID plus `consoleui_core::ownership`. `control.rs` is the control socket
  (`$XDG_RUNTIME_DIR/consoleui-<wayland-socket>.sock`), `child.rs` starts the shell with
  `WAYLAND_DISPLAY`, `DISPLAY` and `CONSOLEUI_COMPOSITOR`, `render.rs` builds frame elements and the
  offscreen PNG screenshot.
- `crates/shell` (binary `consoleui`): Slint UI plus glue.
  - `src/compositor.rs`: counterpart of `gamescope.rs`, active when `CONSOLEUI_COMPOSITOR` is set (takes
    precedence over gamescope). Sends `Hello`/`Launched`/`Exited`/`Focus`/`Overlay`;
    `Shell::poll_compositor` (50 ms timer) turns `WindowMapped`/`AppGone` into `has_window`.
  - `ui/main.slint`: `AppWindow`, the home screen. All sizes scale from a 1920×1080 reference via the
    `s` factor; tiles are positioned absolutely and animated (`animate x, y, width, height`).
  - `ui/theme.slint`: the `Theme` global with design tokens (colors, durations). Use it instead of
    hard-coded values.
  - `ui/types.slint`: `NavAction` enum and `AppTile` struct shared with Rust.
  - `src/shell.rs`: `Shell` owns entries plus a parallel `tiles` vector, all `running` apps (each with a
    gamescope app ID, a stop handle and an `Owner`), the `foreground` app, the three tile models (home =
    `home_entries` from `home_view` + library tile, library view, search results), the `Collection`,
    the current `Screen`, focus, history, the hero loader, the tracked running app, the open context menu
    and the quit confirmation. `Shell::apply(action)` is the single entry point for input; it dispatches
    by screen and ends with `sync()` (screen, focus, hero, hints). `refresh()` rebuilds the models after a launch, an app exit or a re-sort. The hero background uses two
    image layers (`hero-a`/`hero-b` + `hero-front-a`) for crossfades; a launch moves the entry to index 0
    under the fullscreen launch overlay.
  - `src/hero.rs`: `Heroes` decodes hero images on two worker threads (downscaled to max 1920×1080,
    LRU cache of 12, neighbours of the focused tile prefetched, stale jobs dropped). A self-stopping
    `Timer::single_shot` chain polls results only while jobs are open; the old hero stays until the new
    one is ready, then the layers flip. `ui/hero.slint` (`HeroLayer`): parallax (`hero-parallax` -1…1,
    from the focused column, 0 when animations are off) and a settle-zoom on fade-in; both animate only
    after a change, so a static screen does not redraw.
  - `src/tiles.rs`: `AppEntry` → `AppTile` (loads cover, icon and logo, overrides the accent from the cover).
    Icons that Slint cannot load (no file extension) are sniffed with `consoleui_core::image_format`.
  - `src/loader.rs`: background thread that discovers the library and decodes covers/logos with the
    `image` crate into `SharedPixelBuffer`s (Slint `Image`s are created on the UI thread in `tiles.rs`);
    `main.rs` polls the channel, calls `Shell::set_library` and then hides the boot screen.
  - `src/audio.rs`: persistent `pw-cat --raw` process fed by a writer thread. `Shell::apply` compares a
    `Feedback` snapshot before/after each action to pick the sound.
  - `src/grab.rs`: while the quick menu is open, `EVIOCGRAB` on the controller's evdev nodes (matched by
    gilrs `os_name`) and read them directly so the game in front gets no input.
  - `src/gamepad.rs`: gilrs polling → `Action`s (only devices with an SDL mapping); PS/Guide → `Action::Guide`.
  - `src/gamescope.rs`: X11 control of gamescope (tagging windows, baselayer, preparing the overlay and
    notification windows).
  - `src/quick.rs`: quick menu ("Steuerzentrale") items and input handling. In gamescope it is shown in
    `OverlayWindow` (`ui/overlay.slint`, transparent, gets `STEAM_OVERLAY` and is remapped before first
    use); otherwise it is the `QuickMenu` layer inside `AppWindow`.
  - `src/context.rs`: context menu (Menu/Tab on a focused entry in home and library: pin/unpin, hide,
    show again, gamescope options, quit, and the library sort). Shown as the `ContextMenu` layer
    (`ui/contextmenu.slint`) in `AppWindow`; its hints come from `Shell::update_hints`. "Gamescope
    options…" opens the `GameOptionsPanel` layer (`ui/gameoptions.slint`, reuses `SettingRow`);
    `Shell::launch` wraps the launch with the gamescope root size or the window size as output.
  - Toasts (`Shell::toast`): while an app is in front in gamescope they go to `NotificationWindow`
    (`ui/notification.slint`): `STEAM_OVERLAY` without `STEAM_INPUT_FOCUS`, 1 px narrower than the root
    window, so gamescope treats it as its notification window (always on top, never focused). gamescope
    ignores its position and scales it to the output, hence the fixed near-fullscreen size with the toast
    drawn top-right. Otherwise the toast is a layer in `AppWindow`.
  - `src/main.rs`: wiring and timers: gamepad poll (8 ms), status/clock (1 s), app watcher (500 ms), bench.
  - `ui/statusbar.slint`: the right side of the top bar; icons in `ui/icons/*.svg` are tinted with `colorize`.
  - `ui/home.slint`, `ui/library.slint`, `ui/search.slint`: one component per `Screen`, all stacked in
    `main.slint` and cross-faded. Each exposes `focus-x/-y/-size` so the launch overlay starts on the right
    tile. `library.slint` computes `columns` from the window width; Rust reads it for grid navigation.
  - `ui/theme.slint`: `Theme.accent` and all durations are `in-out` and set from Rust (`Shell::apply_settings`),
    for the main window, the overlay and the notification window separately (globals are per component instance).
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
shortcuts (WASD, Q/E, F, /). gilrs reads devices directly, so `Shell::accepts_gamepad` decides which
gamepad input reaches the shell: everything while the quick menu is open, the Guide button always; in
gamescope otherwise only while the shell is in front (`shell_in_front`; gamescope does not hand the X11
focus back after the overlay, so `window-active` is unreliable there and `restore_keyboard_focus` fixes
it for the keyboard), on the desktop only while the window is focused (`window-active`). `--actions`
script steps bypass the gating.

Translations: English is the source language; the only catalog is `lang/de/LC_MESSAGES/consoleui.po`
(gettext, no `msgctxt`, plural rule `n != 1`). Slint uses it as bundled translations (`build.rs`,
`@tr("Text")`, `@tr("Filter: {}", v)`, `@tr("{n} item" | "{n} items" % count)`); Rust embeds the same
file via `consoleui_core::i18n` (`tr("Text")`, `trf("Connected to {}", &[&name])`,
`trn("{n} x", "{n} xs", n, &[])`, `tr_noop` in `const` tables). Arguments must be string literals. For a
new string add `msgid`/`msgstr` to the catalog and run `cargo test -p consoleui-core i18n`
(`catalog_matches_sources` fails on missing, untranslated or stale entries and placeholder mismatches).
One msgid has exactly one German text; use a different English text if the German must differ.
`Shell::apply_language` switches the language at runtime (the on-screen keyboard is QWERTZ for German,
QWERTY for English).

License: GPL-3.0-or-later (`LICENSE`, `license.workspace` in every crate). Code may be taken from
Smithay's anvil/smallvil (MIT) and niri (GPL-3.0-or-later) with attribution in a comment. cosmic-comp is
GPL-3.0-only: copying from it would pin the project to GPL-3.0-only, so only take ideas from it.

If the GPU renderer fails (e.g. the NVIDIA driver needs a reset after a gamescope crash),
`SLINT_BACKEND=winit-software` still renders `--snapshot` PNGs.

Note: `[profile.dev]` uses `opt-level = 1` and dependencies use `opt-level = 3`, so debug builds
render smoothly.

The git repository root is this `ConsoleUI/` directory.
