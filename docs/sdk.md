# Building apps for ConsoleUI

An app built on the SDK looks like the rest of the console, follows the user's accent color and animation
settings, and runs in two places without any change: in a window on any Linux desktop and fullscreen in the
console session. The status of the SDK and what is still missing is in `BACKLOG.md`, Epic 8.

The API is not stable yet, everything is version 0.x and may change.

## Start

```bash
dev/new-app.sh my-app ~/Projects/my-app
cd ~/Projects/my-app
cargo run
```

The crates are not published, so a new app points at this checkout with a path dependency. Slint needs the
same `-devel` packages as the shell.

## Showing up on the console

The shell finds apps through their `.desktop` file (`~/.local/share/applications` or
`/usr/share/applications`), shows them fullscreen and keeps the Guide button. It normally lists only games and
media apps; these keys make an app part of the console:

| Key | Meaning |
|---|---|
| `X-ConsoleUI=true` | Listed whatever `Categories` says. With `NoDisplay=true` it stays out of desktop menus |
| `X-ConsoleUI-Kind` | `game` or `app`, for the library filter |
| `X-ConsoleUI-Cover` | Tile image; the tiles are square, a portrait image is cropped |
| `X-ConsoleUI-Hero` | Background behind the home screen, 16:9 |
| `X-ConsoleUI-Logo` | Title logo over the background |

Artwork is an absolute path to a PNG, or an icon name. Flatpak only exports icons, so there use an icon name that
starts with the app's ID, such as `org.example.App-cover`.

## What is in the template

| File | Purpose |
|---|---|
| `src/page.rs` | The app's logic without any UI, with tests. Every input ends up in `Page::apply(Action)` |
| `src/main.rs` | Connects the window, the input and the page |
| `ui/main.slint` | The window, built from the design system |
| `build.rs` | Compiles the UI with `consoleui_sdk::build::compile` |
| `my-app.desktop` | How the shell and desktops find the app |

## What the SDK offers

- **Design system:** `import { Button, SettingList, Dialog, Theme, ... } from "@consoleui/std.slint";`. The
  `gallery` crate shows every component: `cargo run -p consoleui-gallery`.
- **`Controls`:** controllers and keyboard as one stream of `Event`s. Controller presses only arrive while the
  window has the focus, and never the Guide button.
- **`Hints`:** the content of the hint bar. The glyphs follow the device used last.
- **`Sounds`:** the shell's UI sounds, silent if the user turned them off.
- **`Session`:** where the app runs and the settings of the active profile.
- **`toast(title, text)`:** a toast from the shell, also over a game. Off if the user turned notifications off.
- **`bridge!`:** `nav_action`, `hint_model`, `controller_style` and `apply_theme` for the app's generated types.
- **`snapshot_from_args`:** `--snapshot out.png` to check the UI without a screen capture.

Depend on `consoleui-sdk` and `slint` only; the crates behind it can change without notice.

## The shell's service

A running shell answers on the session bus as `org.consoleui.Shell1` at `/org/consoleui/Shell1`, usable from any
language and from a sandbox (Flatpak: `--talk-name=org.consoleui.Shell1`). `Session::detect` and `toast` use it,
without a shell they fall back to the profile files and to doing nothing. Values are read once at the start;
an app does not notice later changes yet.

| Member | Type | Meaning |
|---|---|---|
| `AccentColor` | property, `s` | `#rrggbb` of the active profile |
| `Animations`, `Sounds` | properties, `b` | The user's settings |
| `ProfileName` | property, `s` | Name of the active profile |
| `Toast(title, text)` | method, `ss` | Shows a toast |

## Design rules

1. **The gamepad comes first.** Everything works with a controller, with the keyboard, and with the mouse on a desktop.
2. **Confirm is ✕ (A), back is ○ (B).** Back always goes one step back. On the first page it leaves the app,
   after asking if something could be lost.
3. **The Guide button belongs to the system.** It opens the quick menu; an app never sees it.
4. **Always show the hint bar,** with what the buttons do right now.
5. **One thing has the focus,** and it is always visible. Use the focus look of the design system.
6. **Use the tokens.** Colors, radii and durations come from `Theme`; never hard-code them, so accent color and
   "animations off" work.
7. **Scale with the window.** Sizes are for 1920×1080, multiplied by `s`: the same layout on a TV and in a small
   desktop window. Readable from the couch: no text below 18 px at that size.
8. **Quit by closing the window.** The shell notices and comes back; an app does not start other apps or
   manage windows.
9. **Keep logic out of the UI,** so it can be tested without a window.

## License

The crates are GPL-3.0-or-later, so an app that links them has to be too, for now (Story 8.1).
