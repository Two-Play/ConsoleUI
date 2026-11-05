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
same `-devel` packages as the shell. To show the app in the shell, install its binary and `.desktop` file;
the shell lists it with the other apps, shows it fullscreen and keeps the Guide button.

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
- **`bridge!`:** `nav_action`, `hint_model`, `controller_style` and `apply_theme` for the app's generated types.
- **`snapshot_from_args`:** `--snapshot out.png` to check the UI without a screen capture.

Depend on `consoleui-sdk` and `slint` only; the crates behind it can change without notice.

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
