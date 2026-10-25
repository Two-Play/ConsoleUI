# Spike 7.1 – Smithay compositor (hybrid architecture)

Tested on 2026-10-25 on Fedora 44 (kernel 7.2.11), KDE Plasma Wayland, NVIDIA RTX 3090 with the
proprietary driver 615.78.08 (open kernel module), Rust 1.99.1. Smithay git main at
`24647fc8d00131eb7f388efed26e35ea316b0018` (2026-10-20, crate version 0.7.0, MSRV 1.87), cloned
outside the repository. Everything ran nested: anvil with its **winit** backend as a window on the
KDE desktop. Nothing ran on a TTY, the udev/DRM backend was only built and read.

## Results

1. **Building anvil works**, but the udev backend needs `-devel` packages that are not installed.
   With only runtime libraries present, anvil links against unversioned symlinks plus minimal
   `.pc` files in a scratch directory (all `-sys` crates involved ship pre-generated bindings, so
   no headers are needed). For 7.2 install the real packages instead:
   `sudo dnf install libinput-devel mesa-libgbm-devel libseat-devel libdrm-devel libdisplay-info-devel libei-devel`.
   Without `gbm.h`, Smithay's build script also disables `gbm_bo_create_with_modifiers2` /
   `gbm_bo_get_fd_for_plane`, so the shim build is fine for a spike only.
2. **Our shell runs natively on Wayland inside anvil** (`DISPLAY` unset, so no X11 fallback):
   `xdg_toplevel`, server-side decorations, dmabuf buffers from the NVIDIA GL driver,
   `wp_presentation` feedback (141 `presented` events in ~3 s), `wp_fifo_v1` / `wp_commit_timing`.
   `--actions` and `--snapshot` work as on the desktop.
3. **X11 clients run through anvil's XWayland** (`X11Wm`): `glxinfo` (direct rendering on the
   RTX 3090), `glxgears` at ~110 fps (host refresh), `xprop -root` shows `_NET_SUPPORTED`,
   `_NET_CLIENT_LIST`. The shell launched `glxgears` (`--dummy-launch`) into anvil's XWayland, the
   game window came on top with focus, and after the game exited the shell showed "Zuletzt
   gespielt: heute" again. **`glxgears` has no `_NET_WM_PID`**, so window → app mapping cannot rely
   on that atom alone (see 7.3 below).
4. **Nested gamescope as a client:**
   - **Wayland path:** gamescope's own Wayland backend (`--backend wayland`) refuses anvil-winit:
     `xdg_backend: Couldn't create Wayland objects.` The check in `WaylandBackend.cpp` (3.16.29)
     requires `zwp_relative_pointer_manager_v1`, which anvil only exposes with the udev backend
     (`HAS_RELATIVE_MOTION`). gamescope then silently falls back to its SDL backend on Wayland
     (`SDL_VIDEODRIVER=wayland`, because `wp_presentation` exists) – that **worked**: FSR-upscaled
     `glxgears` in a 1280×720 window, 60 fps inside gamescope.
     With the global forced on (spike patch), gamescope logs `Initted Wayland backend` and renders
     into anvil (tested on lavapipe, see 6.). **So our compositor must expose relative-pointer and
     pointer-constraints** (both exist in Smithay).
   - **X11 path** (`--backend sdl`, only `DISPLAY` set): gamescope started and `glxgears` inside it ran
     (230 fps, i.e. not paced by a visible surface), but nothing appeared in anvil's output and
     `_NET_CLIENT_LIST` was empty. When gamescope was stopped (SIGINT at 12:32:21), the kernel logged
     **Xid 51 (BAD_TSG) from `gamescope-wl`, then Xid 154 "recovery action PF FLR"**. Since then every
     new Vulkan device / GL buffer allocation on the GPU fails (`vkCreateDevice` →
     `ERROR_INITIALIZATION_FAILED`, anvil `GL_OUT_OF_MEMORY` + segfault in `libnvidia-eglcore`). The
     KDE session keeps running; **a reboot is needed**. The timing points to this run of mine as the
     trigger (gamescope SDL/X11 nested in anvil-winit's XWayland on NVIDIA). A contributing factor
     cannot be ruled out: my frame-dump patch made the NVIDIA EGL window surface of anvil fail with
     `EGL_BAD_ALLOC` after each dump (anvil recovered each time).
   - `SDL_VIDEODRIVER=wayland gamescope --backend sdl` failed with `vkCreateDevice failed (-3)` and a
     gamescope segfault at 12:30:01, i.e. *before* the Xid. Unclear whether this was an early symptom.
5. **The shell's fullscreen request is not honoured by anvil**: Slint sends
   `set_fullscreen(nil)` before the first commit, anvil only fullscreens mapped windows, configures
   without the fullscreen state, and winit then sends `unset_fullscreen()`. Our compositor will make
   the shell fullscreen by policy anyway (7.2), but it must handle `set_fullscreen` before the first
   buffer.
6. After the GPU fault, the remaining tests ran on Mesa llvmpipe/lavapipe (`__EGL_VENDOR_LIBRARY_FILENAMES`
   = Mesa, `LIBGL_ALWAYS_SOFTWARE=1`, `gamescope --prefer-vk-device 10005:0000`): anvil, shell,
   shell → `glxgears` launch and gamescope's Wayland backend work there. gamescope's SDL backend
   cannot run on lavapipe (`Missing required extension: VK_KHR_present_id / present_wait`). Note:
   gamescope has file capabilities, so the Vulkan loader ignores `VK_ICD_FILENAMES`; only
   `--prefer-vk-device` selects lavapipe.
7. **Smithay covers everything 7.2–7.8 needs**; the gaps are tearing control (7.9) and any shell ↔
   compositor protocol (our own). Details below.

**Recommendation:** go ahead with 7.2 as a separate crate `crates/compositor` built on Smithay
(MIT, like anvil), start from anvil's `udev.rs` rather than from a GPL compositor, and treat
"nested gamescope on NVIDIA" as a risk to verify early (after the reboot) before 7.10.

## Re-run after the reboot (done)

After the reboot, anvil (same commit, built in the `dev/toolbox.sh` container with the real `-devel`
packages, so the udev backend is included) ran nested on the host with the NVIDIA EGL renderer, and
the nested gamescope ran on the NVIDIA GPU. `journalctl -k | grep -i xid` stayed empty throughout.

| Test | Result |
|---|---|
| E: `gamescope --backend sdl` with `SDL_VIDEODRIVER=wayland` in anvil | works: `xdg_toplevel` configured by anvil, ~900 buffers attached and frame callbacks answered in 8 s, glxgears inside at 60 fps |
| `gamescope` with the auto backend | the Wayland backend fails ("Couldn't create Wayland objects", relative-pointer missing in anvil-winit) and gamescope silently falls back to **headless** – no window. 7.2 must offer relative-pointer/pointer-constraints |
| D: `gamescope --backend sdl` with `SDL_VIDEODRIVER=x11` on anvil's XWayland `:1` | works this time: the window is in `_NET_CLIENT_LIST`, glxgears inside at ~105 fps, **no Xid**. The earlier fault most likely came from the frame-dump patch (`EGL_BAD_ALLOC` on anvil's surface) together with SIGINT, not from this path itself |
| C: gamescope's native Wayland backend | not re-run (needs the relative-pointer patch, which was lost with the scratchpad) |

Conclusion: nested gamescope on NVIDIA under a Smithay compositor works through SDL on both Wayland
and X11. Our compositor should still expose relative-pointer so gamescope's own Wayland backend can be
used and the auto backend does not fall back to headless.

## Details

### Build (question 1)

| Item | Value |
|---|---|
| Smithay | git main `24647fc8d001`, 0.7.0 |
| Rust | rustc 1.99.1 (Smithay MSRV 1.87, edition 2024) |
| anvil features | default = `egl, winit, x11, udev, xwayland, libei` |
| Build time | ~2 min release (after deps) |

`pkg-config` on this machine: `wayland-server/-client 1.26.0`, `xkbcommon 1.13.1`, `libudev 259`,
`egl 1.5`, `glesv2 3.2`, `pixman-1 0.46.2`, `vulkan 1.4.341`, `xcb 1.17.0` are present.
Missing (runtime libs exist, `-devel` not installed): `libinput` (libinput 1.31.3), `gbm`
(mesa-libgbm 26.2.5), `libseat` (libseat 0.9.3), `libdrm` (2.4.134), `libdisplay-info` (0.3.0),
`libei-1.0` (1.6.0). `Xwayland` 24.1.14 is installed. Note: `seatd` is not installed; libseat uses
logind, which is what a display-manager session has.

Even the winit/x11-only build needs `-lgbm` (`backend_x11` pulls in `backend_gbm`).

### Nested runs (questions 2 and 3)

anvil prints `Listening on wayland socket name="wayland-1"`; the XWayland display is not logged,
it is the `:N` argument of the `Xwayland` child process. anvil killed with SIGTERM leaves
`/tmp/.X11-unix/XN` and `/tmp/.XN-lock` behind (no signal handling) – our compositor must clean up.

Globals of anvil-winit (from `wayland-info`): `wl_compositor, wl_subcompositor, wl_shm,
wl_data_device_manager, wl_seat, wl_output, xdg_wm_base, zxdg_decoration_manager_v1,
zxdg_output_manager_v1, zwlr_layer_shell_v1, zwp_linux_dmabuf_v1 (v6), wl_drm,
wl_eglstream_display, wp_presentation, wp_viewporter, wp_fractional_scale_manager_v1,
wp_single_pixel_buffer_manager_v1, wp_fifo_manager_v1, wp_commit_timing_manager_v1,
xdg_activation_v1, zwp_pointer_constraints_v1, zwp_pointer_gestures_v1,
zwp_keyboard_shortcuts_inhibit_manager_v1, zwp_text_input_manager_v3, zwp_input_method_manager_v2,
zwp_virtual_keyboard_manager_v1, zwp_tablet_manager_v2, zwp_primary_selection_device_manager_v1,
zwlr_data_control_manager_v1, zxdg_exporter_v2/importer_v2, wp_security_context_manager_v1,
ext_image_copy_capture_manager_v1, ext_output_image_capture_source_manager_v1, wl_fixes`.
Missing for games: `zwp_relative_pointer_manager_v1` (only with udev), `wp_tearing_control_v1`,
`wp_color_manager_v1`, `wp_content_type_v1` (Smithay has the module, anvil does not use it).

anvil advertises `ext_image_copy_capture` but fails every frame ("Anvil doesn't implement actual
capture"), so `grim`-style capture does not work. For PNGs of the composed output I patched anvil
(scratchpad only): when the file named by `$ANVIL_DUMP` exists, the winit loop renders with
age 0 and reads the framebuffer back with `ExportMem::copy_framebuffer` (rows flipped). This is
also the core of story 7.12.

| Test | Result |
|---|---|
| `wayland-info`, `xdpyinfo`, `glxinfo -B` | OK; GL in XWayland = NVIDIA, direct rendering |
| `eglgears_wayland`, `glxgears` (6 s each) | OK, 110 fps |
| shell `--windowed --dummy --actions right,right,down --snapshot` | OK, exit 0, PNG written |
| shell without `--windowed` | maps as a normal window, see result 5 |
| shell `--dummy-launch glxgears --actions wait:500,confirm` | game on top with `_NET_WM_STATE_FOCUSED`, `_NET_ACTIVE_WINDOW` stays 0 (anvil's WM does not set it); after `pkill glxgears` the shell is back |
| gamescope `--backend wayland` (anvil-winit) | falls back to SDL/Wayland, works |
| gamescope auto backend | same as above |
| gamescope `--backend sdl`, X11 only | not visible; Xid 51/154 on exit (see results) |
| gamescope `--backend sdl` + `SDL_VIDEODRIVER=wayland` | `vkCreateDevice failed (-3)`, segfault |
| gamescope `--backend wayland` + relative-pointer global, lavapipe | `Initted Wayland backend`, renders (own CSD title bar) |

### What 7.2 needs from anvil's udev backend (`anvil/src/udev.rs`, ~1700 lines)

- **Session:** `LibSeatSession::new()` → `(session, notifier)`; the notifier delivers
  `SessionEvent::PauseSession/ActivateSession` (VT switch): suspend/resume libinput, pause/activate
  `DrmOutputManager`. VT switching itself is `session.change_vt(n)` on Ctrl+Alt+Fn.
- **GPU:** `primary_gpu(seat)` / `all_gpus`, `DrmNode`; `GpuManager<GbmGlesBackend<GlesRenderer>>`
  (multi-GPU ready); per device `DrmDevice::new(fd)`, `GbmDevice`, `GbmAllocator` with
  `RENDERING | SCANOUT`, `DrmOutputManager` → one `DrmCompositor` per CRTC.
- **Outputs/hotplug:** `UdevBackend` events (added/changed/removed) + `smithay_drm_extras::DrmScanner`
  for connectors; `libdisplay-info` for EDID names. That also covers 7.11.
- **Input:** `Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>`,
  `udev_assign_seat`, `LibinputInputBackend` as a calloop source → the same `process_input_event`
  the winit backend uses. Gamepads are *not* libinput devices; gilrs in the shell keeps working
  (evdev access through the logind seat ACLs, as today).
- **Rendering loop:** `render_surface` builds the element list, `DrmCompositor::render_frame` with
  `FrameFlags::DEFAULT` (direct scanout allowed, `ANVIL_DISABLE_DIRECT_SCANOUT` turns it off),
  `queue_frame`, then on VBlank `frame_submitted` and presentation feedback.
- **Extras we can drop at first:** DRM leasing (VR), the debug/fps overlay, renderdoc, pixman.
- What anvil does **not** have and we need: a policy layer (shell always fullscreen, one app in
  front, overlays above), signal handling/cleanup, a control socket for the shell.

### Smithay coverage for 7.2–7.9 (question 4)

| Story | Smithay | Notes |
|---|---|---|
| 7.2 DRM/KMS, libinput, libseat | `backend::session::libseat::LibSeatSession`, `backend::udev::UdevBackend`, `backend::libinput::LibinputInputBackend`, `backend::drm::{DrmDevice, DrmNode, output::DrmOutputManager, compositor::DrmCompositor}`, `backend::allocator::gbm::{GbmDevice, GbmAllocator}`, `backend::renderer::{gles::GlesRenderer, multigpu::GpuManager}` | anvil shows all of it; NVIDIA works with GBM since driver 495 but needs a test on real hardware |
| 7.3/7.4 XWayland | `xwayland::{XWayland, XWaylandEvent}`, `xwayland::xwm::{X11Wm, XwmHandler, X11Surface, WmWindowType}`; `X11Surface::is_override_redirect()`, `pid()` (`_NET_WM_PID`), `class()`, `startup_id()`, `set_fullscreen()`, `set_activated()`, `configure()`; `X11Wm::raise_window`, `update_stacking_order_*`; `wayland::xwayland_shell`, `wayland::xwayland_keyboard_grab` | `pid()` is `None` for clients like `glxgears`; fall back to the X client's socket peer (XRes `QueryClientIds`, as gamescope does) or the cgroup of the Xwayland client – `consoleui_core::ownership` stays the decision logic |
| 7.5 Focus/input | `input::Seat`, `input::keyboard::{KeyboardHandle::set_focus, set_grab, input_intercept, KeyboardGrab}`, `input::pointer::PointerGrab`, `wayland::keyboard_shortcuts_inhibit` | the compositor decides who gets keyboard focus; "game gets nothing while the quick menu is open" = keep focus on the shell surface (no EVIOCGRAB needed for keyboard/mouse). Gamepads: the game reads evdev directly, so `grab.rs` (or revoking the game's evdev access) is still needed for controllers |
| 7.6 Overlays | `wayland::shell::wlr_layer::{WlrLayerShellState, LayerSurface}`, `desktop::{LayerMap, layer_map_for_output}` | **Slint (1.18) has no layer-shell support**, so for our shell the compositor should place the shell's own extra toplevels (quick menu, toast) by app_id/title or by a request over the control socket; layer-shell stays useful for third-party tools |
| 7.7 Transitions | `backend::renderer::element::{Element, RenderElement, texture, utils::RescaleRenderElement/RelocateRenderElement}`, custom GLES shaders (`GlesRenderer::compile_custom_pixel_shader`) | animating a game surface is just scaling its render element |
| 7.8 Direct scanout | `DrmCompositor` with `FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT(_ANY)`, `ALLOW_OVERLAY_PLANE_SCANOUT`, `ALLOW_CURSOR_PLANE_SCANOUT`; `wayland::dmabuf` feedback with scanout tranches (`DmabufFeedbackBuilder`, see anvil `get_surface_dmabuf_feedback`) | automatic when the fullscreen game buffer is plane-compatible; falls back to composition when an overlay element is on top |
| 7.9 VRR / tearing | `DrmSurface/DrmCompositor::vrr_supported()` → `VrrSupport`, `use_vrr(bool)`; `wayland::presentation` (presentation-time), `wayland::fifo`, `wayland::commit_timing`, `wayland::content_type` | **no `wp_tearing_control_v1` module and no async page flip in `DrmCompositor`** at this commit – tearing needs own work or an upstream contribution |
| other | `wayland::relative_pointer`, `wayland::pointer_constraints` (needed by gamescope's Wayland backend and by games), `wayland::drm_syncobj` (explicit sync, important on NVIDIA), `wayland::session_lock`, `wayland::idle_inhibit`, `wayland::image_copy_capture` | |

### Proposed structure for 7.2 (question 5)

- **New crate `crates/compositor`** (binary `consoleui-compositor`, not in `default-members`
  at first), depending on `smithay` (git or 0.7, features `backend_drm, backend_gbm,
  backend_libinput, backend_udev, backend_session_libseat, backend_egl, renderer_gl, xwayland,
  desktop, wayland_frontend`, plus `backend_winit` for development) and on `consoleui-core`.
  Two backends from day one: `--winit` (nested, for development and CI-like tests) and `--drm`
  (session). Policy code (which surface is in front, focus rules, overlay stacking, app mapping)
  lives in testable modules without Smithay types where possible, like `consoleui-core` today.
- **Startup:** the compositor creates the Wayland socket and XWayland, then spawns
  `consoleui` itself with `WAYLAND_DISPLAY`, `DISPLAY` and `CONSOLEUI_COMPOSITOR=<control socket path>`.
  Apps launched by the shell inherit both displays, so games keep running through XWayland and
  native Wayland apps work too. The session file `consoleui-smithay.desktop` sits next to the
  gamescope session; `session/consoleui-nested` gets a `--compositor` variant that runs
  `consoleui-compositor --winit`.
- **Detection in the shell:** `CONSOLEUI_COMPOSITOR` set → new `Compositor` mode next to
  `CONSOLEUI_GAMESCOPE=steam`. `gamescope.rs` and `grab.rs` stay for the fallback session.
- **Shell ↔ compositor protocol:** a Unix socket with line-delimited JSON (serde types in
  `consoleui-core`, e.g. `protocol.rs`), not a custom Wayland protocol – Slint/winit does not expose
  its Wayland connection for extra protocols, and the shell already has a polling loop. Messages:
  - shell → compositor: `Launched { app_id, pid, scope }` (the compositor maps windows by PID /
    cgroup / `SteamAppId` with `consoleui_core::ownership`), `Focus { app_id | Shell }`,
    `Overlay { open: bool }` (quick menu: keep keyboard focus on the shell, draw the shell's
    overlay toplevel above the game), `Toast { visible }`, later `Transition { from_rect }` for 7.7.
  - compositor → shell: `WindowMapped { app_id, title }`, `AppGone { app_id }`,
    `GuidePressed` (if the compositor ever sees the Guide key on a keyboard), `FocusChanged`.
  This replaces the `STEAM_GAME` atoms, `GAMESCOPECTRL_BASELAYER_APPID`, the overlay/notification
  window tricks and `restore_keyboard_focus`.
- **SP after the spike** (backlog values in brackets):
  7.2 **13** (13) – anvil gives a template, but NVIDIA/GBM on real hardware and session handling
  are still the main risk; 7.3 **13** (13) – PID-less X11 windows need XRes/cgroup mapping;
  7.4 13 (13); 7.5 **5** (8) – keyboard/pointer focus is fully in our hands, controller grab stays
  as today; 7.6 **5** (8) – no layer-shell from Slint, but placing the shell's own toplevels is
  simple once the control socket exists; 7.7 8; 7.8 **5** (8) – `DrmCompositor` already does plane
  assignment; 7.9 **13** (8) – no tearing-control in Smithay. 7.10 needs relative-pointer +
  pointer-constraints in the compositor (cheap) and a check that nested gamescope does not fault
  the NVIDIA GPU. 7.1–7.6 stays at about 60 SP.

### niri and cosmic-comp

- **niri** (`GPL-3.0-or-later`, Smithay git) and **cosmic-comp** (`GPL-3.0-only`) are both
  Smithay compositors and show production patterns: niri's TTY backend (session pause/resume,
  output config, `DrmCompositor` with direct scanout, VRR toggles), its XWayland via
  xwayland-satellite, and cosmic-comp's XWayland integration and its handling of fullscreen games.
- **Licensing:** ConsoleUI has **no license file and no `license` field** in `Cargo.toml` yet.
  Copying code from niri or cosmic-comp would make the compositor (and in practice the project)
  GPL-3.0. Reading them for ideas is fine; code we copy should come from Smithay/anvil/smallvil
  (MIT). Decide the project license before 7.2 starts.

## Commands that were run

```bash
S=/tmp/claude-1000/-home-philippe-Dokumente-Entwicklung-Rust-ConsoleUI/<session>/scratchpad
git clone --depth 50 https://github.com/Smithay/smithay.git $S/smithay
pkg-config --modversion libinput gbm libseat libdrm libdisplay-info libei-1.0   # missing
rpm -q libinput-devel mesa-libgbm-devel libseat-devel libdrm-devel libdisplay-info-devel libei-devel
# $S/libshim: lib<name>.so -> /usr/lib64/lib<name>.so.N symlinks + minimal .pc files
SHIM=1 sh $S/build-anvil.sh ""                       # LIBRARY_PATH/PKG_CONFIG_PATH = shim; cargo build --release
sh $S/run-anvil.sh sh $S/t1-basic.sh                 # wayland-info, xdpyinfo, glxinfo, glxgears, eglgears_wayland, xprop
sh $S/run-anvil.sh sh $S/t2-shell.sh                 # shell native Wayland + WAYLAND_DEBUG, shell launches glxgears
ANVIL_BIN=$S/anvil-dump ANVIL_DUMP=$S/dump-request sh $S/run-anvil.sh sh $S/t3-gamescope.sh   # gamescope paths (triggered the Xid)
# after the GPU fault, software rendering only:
ANVIL_FORCE_RELATIVE_POINTER=1 ANVIL_BIN=$S/anvil-dump ANVIL_DUMP=$S/dump-request \
  sh $S/swenv.sh sh $S/run-anvil.sh sh $S/t4-full.sh
gamescope --prefer-vk-device 10005:0000 --backend wayland -W 1280 -H 720 -w 640 -h 360 -F fsr -- glxgears
journalctl -k | grep -i -E 'xid|NVRM'
```

`run-anvil.sh` starts anvil, waits for the socket and the XWayland display, runs the test with
`WAYLAND_DISPLAY`/`DISPLAY` set and stops anvil. The shell ran with a scratch `XDG_CONFIG_HOME`
(`{"sounds": false}`) and `XDG_DATA_HOME`. No screenshots of the desktop were taken; images came
from the shell's `--snapshot` and from the anvil frame dump.

## Not tested

- anvil or any compositor on a TTY / DRM (by design); libseat/logind session, VT switching,
  direct scanout, VRR – only read in the code.
- Nested gamescope on the NVIDIA GPU after the fault (blocked until reboot), HDR, `-e` mode inside
  our compositor, real games, Steam.
- Gamepad input through the compositor (gilrs reads evdev directly, unaffected).
- niri and cosmic-comp were not built; only their licenses were checked.
