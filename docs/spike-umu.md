# Spike 10.1 – Proton outside Steam with umu-launcher

Goal: find out whether the game manager (Epic 10) can run Windows games through `umu-run` from our session,
and decide how the app is built and shipped.

Tested with umu-launcher 1.4.4 (the zipapp Lutris keeps in `~/.local/share/lutris/runtime/umu/umu-run`),
UMU-Proton 10.0-4 and the Steam Linux Runtime 3 "sniper" (`~/.local/share/umu/steamrt3`), on the host desktop
(KDE, RTX 3090) and in the test VM (Fedora 44, llvmpipe/lavapipe) under `consoleui-compositor --drm`.

## Test program

`dev/winprobe/probe.c` is a stand-in Windows game: a borderless window the size of the screen that clears with
Direct3D 11 and writes the D3D adapter, frame counts, focus changes, key presses and XInput buttons to
`probe.log` in its working directory. `dev/winprobe/build.sh` builds `target/winprobe/probe.exe` with MinGW in
the dev container (`mingw64-gcc` is now in `dev/toolbox.sh`). Run it like a game:

```bash
WINEPREFIX=/tmp/pfx GAMEID=umu-default PROTONPATH=~/.local/share/Steam/compatibilitytools.d/UMU-Proton-10.0-4 \
  UMU_RUNTIME_UPDATE=0 umu-run target/winprobe/probe.exe
```

In the VM the runtime and Proton were copied (reflinks) into `target/umu-test/` with a launch script, so the
guest found them on the shared checkout; the prefix stayed on the guest's disk.

## Results

1. **It runs.** On the host DXVK reached the NVIDIA GPU (`adapter NVIDIA GeForce RTX 3090`). In the VM DXVK ran
   on lavapipe at about 100 fps. A new prefix takes about 20 s (wineboot in the VM); with an existing prefix the
   window shows content 10 s after the launch in the VM.
2. **Under our compositor it behaves like any app.** The shell starts it in its `systemd-run --scope` unit
   (`Launch::Command`), the compositor maps the XWayland window to that app through the scope
   (`App(…) "winprobe" shows content`), it comes to the front, the quick menu opens over it, and real key events
   (`dev/vm.sh keys`) reach the game before and after the quick menu but not while it is open.
3. **Quitting** through the quick menu (`systemctl --user stop` of the scope) ends everything: umu, pressure-vessel,
   Proton, wineserver and the game; nothing is left. The game gets SIGTERM, not a window close: it cannot save.
4. **Ambient capabilities break bubblewrap.** A session started on a TTY (here `dev/vm.sh seat`, i.e.
   `systemd-run -p TTYPath=… -p StandardInput=tty`) gives its processes the ambient capability
   `CAP_WAKE_ALARM`. Every child inherits it, and bubblewrap refuses: "Unexpected capabilities but not setuid,
   old file caps config?". That hit umu's own `srt-bwrap` and the system's `bwrap` 0.12 alike, so Flatpak apps
   started from the shell in such a session fail the same way. The session through SDDM did not have it.
   **Fixed:** the compositor and the shell clear their ambient set at start
   (`prctl(PR_CAP_AMBIENT_CLEAR_ALL)`); afterwards the same launch works without any workaround.
5. **umu's environment.** umu sets `SteamAppId`, `SteamGameId` and `STEAM_COMPAT_APP_ID` from `GAMEID`:
   `umu-default` gives `default`/`0` (no number, so `ownership` and `steam::running_games` ignore it), but an ID
   from the umu database like `umu-1091500` gives `SteamAppId=1091500`, the Steam ID of that game. The shell
   finds the game's windows through its scope first, but `steam::running_games()` and `steam::stop_game()` would
   take such a process for the Steam game with that ID. Fixed in 10.2: processes with `UMU_ID` are
   skipped there and in `ownership`.
6. **Where umu comes from.** Not in Fedora's own repositories (only third-party `terra`); Lutris downloads the
   upstream zipapp into its runtime folder, Heroic's Flatpak bundles it. umu downloads the runtime (`steamrt3`,
   about 800 MB) and, without `PROTONPATH`, the newest UMU-Proton (about 1.4 GB) by itself;
   `UMU_RUNTIME_UPDATE=0` skips the update check.
7. **Nested gamescope (7.10) could not be tested:** gamescope's Wayland backend fails with a protocol error in
   the VM even with `true` as the child (the known VM limit in `docs/dev-vm.md`), and the host was busy.

## Decisions

- **The app:** `crates/games`, binary `consoleui-games`, an app on the SDK like the store, not a default member;
  the model and the parsers in `consoleui-core` (`games.rs`), as planned in 10.2.
- **Shipped on the host, not as a Flatpak.** The game manager starts installers and games from any folder, needs
  the GPU, all input devices, 32-bit libraries and bubblewrap inside the sandbox (Heroic's Flatpak needs
  `devices=all`, `multiarch`, `per-app-dev-shm` and many folders for the same). Games must also run in the shell's
  scope, not inside an app's sandbox. It is installed with the shell (`session/install.sh`).
- **Launching:** the desktop entry the app writes for a game runs `consoleui-games --run <id>`, which sets the
  environment and replaces itself with `umu-run` (`exec`), so the PID stays the one the shell's scope tracks.
- **umu-run:** `umu-run` from `PATH` (distribution package), else the one Lutris keeps, else the app downloads the
  upstream zipapp release into `$XDG_DATA_HOME/consoleui/umu` (with a checksum check; part of 10.5).
- **GAMEID:** the umu database ID when the game is known (it brings its fixes), otherwise `umu-default`.

## Not tested

- A nested gamescope around `umu-run` (7.10) and the gamescope session: on the host, after the user agrees.
- Controllers in the game: XInput through Proton's SDL, and that the shell's grab (`EVIOCGRAB`) keeps them away
  while the quick menu is open (in the VM only the keyboard was tested).
- Real games, an installer, umu downloading Proton and the runtime (all runs used local copies, no network).
- Performance on the host under our compositor.

## Follow-ups

- **Quit gently first:** for Windows games a close request to the window (WM_CLOSE through XWayland's
  `WM_DELETE_WINDOW`), and SIGTERM only after a few seconds. Add to 10.4 or as its own story.
