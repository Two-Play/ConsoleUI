# Spike 11.1 – A web view for links

Goal: find out how ConsoleUI shows a web page for a link (Epic 11), and whether that needs a web view of its own or
whether an existing browser can be steered for the console instead.

Tested in the test VM (Fedora 44, 4 cores, 6 GB, llvmpipe/lavapipe) under `consoleui-compositor --drm`, with the
shell and without it (`--no-shell`). Candidates:

- **WebKitGTK 6** (`webkitgtk6.0` 2.54.2, Fedora package) in a small Rust program of our own, `dev/webprobe`
  (`webkit6` crate 0.6.1): a fullscreen window with only the web view, commands on stdin.
- **Chromium 157** in app mode (`--app=<url> --start-fullscreen`, Wayland with `--enable-wayland-ime
  --wayland-text-input-version=3`), steered over the Chrome DevTools Protocol (`--remote-debugging-port`).
- **Firefox 160** in kiosk mode (`--kiosk --profile <dir>`).
- WPE WebKit / cog: **not packaged in Fedora** (neither `wpewebkit` nor `cog`); building it ourselves or a Flatpak would
  be needed, so it was dropped.

## Test program

`dev/webprobe/build.sh` builds `target/webprobe/release/webprobe` in the dev container (`webkitgtk6.0-devel` is now in
`dev/toolbox.sh`). `webprobe [--data-dir DIR] <url>` logs timings and page events to stderr and reads commands on stdin
(`open <url>`, `back`, `forward`, `reload`, `zoom <factor>`, `scroll <dy>`, `quit`); in the VM a FIFO fed them.
`dev/webprobe/test.html` has an autofocused text field, links (one with `target=_blank`) and a long page;
`dev/webprobe/pss.sh <pattern>` adds up the memory (PSS) of the matching processes. webkit6 0.6.1 does not enable the
gtk4 feature it needs (`gtk::Accessible`), so the probe enables `gtk4/v4_18` itself.

## Results

| | WebKitGTK 6 (own frame) | Chromium `--app` | Firefox `--kiosk` |
|---|---|---|---|
| Launch → first content (shell launch, test page) | 0.5 s | 0.8 s | 1.5 s |
| Memory (PSS, all processes) | 327 MB test page, 675 MB Wikipedia | 556 MB Wikipedia | 964 MB test page |
| Browser chrome on screen | none (we draw everything) | none | none |
| Shell's floating keyboard in a web form (text-input-v3) | works | works (with the two flags) | works |
| Steering (back, reload, zoom, scroll, open) | direct API calls | DevTools Protocol (WebSocket) | WebDriver BiDi (WebSocket), not tried |
| Popups, permission requests, TLS errors, downloads | our callbacks (`create`, `permission-request`, ...) | the browser's own windows and bubbles | the browser's own dialogs |
| Data per profile (11.6) | a `NetworkSession` per data directory | `--user-data-dir` | `--profile` |
| Widevine (Netflix, Disney+, ...) | no | not in Fedora's build (Google Chrome has it) | can download it |

1. **All three run under our compositor like any app** and come to the front; the Chromium and WebKit windows were
   mapped through the shell's scope like any other launch. Memory figures are with software rendering and include
   the GPU process; on the host they will be lower, the order stays.
2. **The on-screen keyboard works in all three** without code of their own: ✕ on the focused field opens the shell's
   floating keyboard (typing into apps) and the letters reach the page. The keyboard's own preview shows the text one
   or two letters behind what the field has (in all three, so on our side: `src/shell/typing.rs`).
3. **WebKitGTK:** a local page in 0.5 s, Wikipedia in 5 s on llvmpipe; `zoom`, `back` and `open` work directly;
   `scroll` through JavaScript did nothing after a zoom change (to look at in 11.4, WebKit has no scroll API in GTK 4,
   synthetic wheel events are the other way). `target=_blank` and `window.open` are caught in `create` and loaded in
   the same view, permission requests are answered by us (denied in the probe).
4. **Chromium:** the DevTools endpoint answers (`/json/list` lists the page with its title and URL), so back, reload,
   zoom and scroll are possible from the shell; all of it over a WebSocket and JSON with a debug port open on
   localhost, which any local process can use.
5. **Firefox** starts slowest and needs the most memory, and its own dialogs (permissions, "restore session", updates)
   are made for the mouse.

## Answer: a web view of our own or an existing browser?

Nobody builds a browser engine; the question is who owns the window around it. Steering an existing browser in kiosk or
app mode works (the keyboard too) and brings Widevine with Firefox or Google Chrome, but the browser's dialogs, popups
and new windows stay its own and look nothing like the console, the control runs through a debug port, and its startup
and memory are higher. A frame of our own around WebKitGTK is a few hundred lines, starts fastest, needs the least
memory, and every popup, permission, certificate error and download goes through our code and the design system. Its
one gap is DRM video.

## Decisions

- **The web view is our own frame around WebKitGTK 6:** `crates/web`, binary `consoleui-web`, a GTK 4 program (not
  Slint, the page is the window) with the distribution's `webkitgtk6.0`; the shell draws the bar and dialogs in its
  overlay window (11.5) and steers it over D-Bus (`org.consoleui.Web1`: open, back, forward, reload, zoom, scroll,
  pointer; signals for title, URL, progress, security, requests).
- **Streaming with DRM goes to a real browser:** a page that needs Widevine (`EME` request) offers "Open in Firefox"
  (or Google Chrome if installed), started like an app in kiosk mode. That keeps the existing browser for what only it
  can do, without making it the web view.
- **Not WPE/cog** (not packaged), **not Chromium `--app`** as the web view (its own popups and windows, debug port).

## Not tested

- On the host GPU and in the gamescope session (the VM has no gamescope there); video playback and its frame rate.
- The controller: pointer, scroll and zoom from the gamepad (11.4); the shell's grab while the bar is open.
- What Chromium `--app` does with `target=_blank` (expected: a normal browser window) and Firefox's WebDriver BiDi.
- Widevine itself (no streaming account in the VM).

## Follow-ups

- The floating keyboard's preview lags behind the field in all three engines (`src/shell/typing.rs`, surrounding
  text); worth fixing on its own, it also affects apps.
