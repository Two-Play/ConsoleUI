//! Compositor state: Wayland globals, windows, seat, the window policy and the control socket.
//!
//! The protocol handler setup follows Smithay's anvil (`anvil/src/state.rs`, `shell/*.rs`, MIT,
//! Copyright (c) 2017 Victor Berger); the window management is our own (see `policy.rs`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use consoleui_core::keymap::{self, Keymap};
use consoleui_core::ownership;
use consoleui_core::protocol::{self, CompositorMessage, ShellMessage, Target, WindowRole};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{
    PopupKind, PopupManager, Space, Window, WindowSurface, find_popup_root_surface, get_popup_toplevel_coords,
};
use smithay::input::dnd::{DndGrabHandler, GrabType, Source};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorImageStatus, PointerHandle};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::utils::{Clock, Logical, Monotonic, Point, Rectangle, SERIAL_COUNTER, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, add_blocker,
    add_pre_commit_hook, get_parent, is_sync_subsurface, with_states,
};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf};
use smithay::wayland::drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::pointer_constraints::{
    PointerConstraintsHandler, PointerConstraintsState, with_pointer_constraint,
};
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};
use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::X11Wm;
use tracing::{debug, info, warn};

use crate::backdrop::Backdrops;
use crate::backend::Backend;
use crate::blur::BlurCache;
use crate::child::ShellProcess;
use crate::control::{ClientId as ControlClient, Control};
use crate::focus::FocusTarget;
use crate::policy::{Layer, Policy, Role, WindowId};
use crate::render::{Blur, Drawn};
use crate::transition::Transitions;

#[derive(Debug, Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// A screenshot request (CLI or control socket); `reply` gets `ScreenshotDone`.
pub struct ScreenshotRequest {
    pub path: PathBuf,
    pub reply: Option<ControlClient>,
}

#[allow(dead_code)]
pub struct Globals {
    xdg_decoration_state: XdgDecorationState,
    output_manager_state: OutputManagerState,
    presentation_state: PresentationState,
    pointer_constraints_state: PointerConstraintsState,
    relative_pointer_state: RelativePointerManagerState,
    viewporter_state: ViewporterState,
    single_pixel_buffer_state: SinglePixelBufferState,
}

pub struct State {
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub loop_signal: LoopSignal,
    pub socket_name: String,
    pub clock: Clock<Monotonic>,
    pub running: bool,
    /// Exit code of the compositor (the shell's, once it has exited)
    pub exit_code: i32,

    pub backend: Backend,
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub output: Option<Output>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub syncobj_state: Option<DrmSyncobjState>,

    /// Globals without per-request state on our side; kept so they stay alive
    #[allow(dead_code)]
    pub globals: Globals,

    pub seat: Seat<State>,
    pub pointer: PointerHandle<State>,
    pub cursor_status: CursorImageStatus,
    /// Keys whose press the compositor handled itself (their release is not forwarded either)
    pub suppressed_keys: Vec<u32>,
    /// The keyboard should go from the shell to an app, but keys are still held: it moves once
    /// they are released, so the app does not get the release of a key the shell got pressed
    /// (Story 7.5).
    pub focus_deferred: bool,
    /// Keymap detected from the system (`XKB_DEFAULT_*`, localectl, /etc files)
    pub system_keymap: Keymap,
    /// Keymap the seat keyboard uses now (the system one or the shell's choice)
    pub keymap: Keymap,

    pub policy: Policy,
    /// What was in front when the policy was last applied (front changes start transitions)
    pub front: Target,
    /// Animated front changes (Story 7.7)
    pub transitions: Transitions,
    /// Blurred backdrops under the overlay and notification windows (Story 7.13)
    pub backdrops: Backdrops,
    pub blur_cache: BlurCache,
    /// Backdrop blur enabled (`--no-blur`, `CONSOLEUI_NO_BLUR`); without it the shell is not told
    /// about the feature and keeps its opaque glass
    pub blur: bool,
    pub control: Control,
    pub shell: Option<ShellProcess>,
    pub screenshots: Vec<ScreenshotRequest>,

    pub xwayland_shell_state: XWaylandShellState,
    pub primary_selection_state: PrimarySelectionState,
    pub xwm: Option<X11Wm>,
    /// XWayland display number (`DISPLAY=:N`)
    pub xdisplay: Option<u32>,
}

impl State {
    pub fn new(
        display: smithay::reexports::wayland_server::Display<State>,
        handle: LoopHandle<'static, State>,
        loop_signal: LoopSignal,
        backend: Backend,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let dh = display.handle();
        let clock = Clock::new();

        let source = ListeningSocketSource::new_auto()?;
        let socket_name = source.socket_name().to_string_lossy().into_owned();
        handle.insert_source(source, |stream, _, state| {
            if let Err(err) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                warn!("cannot add Wayland client: {err}");
            }
        })?;
        handle.insert_source(
            smithay::reexports::calloop::generic::Generic::new(
                display,
                Interest::READ,
                smithay::reexports::calloop::Mode::Level,
            ),
            |_, display, state| {
                // Safety: the display is never dropped while the source exists.
                unsafe {
                    display.get_mut().dispatch_clients(state)?;
                }
                Ok(smithay::reexports::calloop::PostAction::Continue)
            },
        )?;
        info!("listening on Wayland socket {socket_name}");

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, clock.id() as u32);
        // gamescope's Wayland backend refuses to run without these two (docs/spike-smithay.md).
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, backend.seat_name());
        let pointer = seat.add_pointer();
        let (system_keymap, source) = keymap::detect();
        info!("keyboard layout {} (from {source:?})", system_keymap.describe());
        let keymap = match seat.add_keyboard(xkb_config(&system_keymap), 200, 25) {
            Ok(_) => system_keymap.clone(),
            Err(err) => {
                warn!("keymap {system_keymap:?} does not compile ({err}), using the default");
                seat.add_keyboard(XkbConfig::default(), 200, 25)?;
                Keymap::default()
            }
        };

        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let mut control = Control::listen(&runtime_dir.join(format!("consoleui-{socket_name}.sock")), &handle)?;
        control.set_greeting(keymap_message(&keymap));

        let dh_for_x = dh.clone();
        Ok(State {
            display_handle: dh,
            loop_handle: handle,
            loop_signal,
            socket_name,
            clock,
            running: true,
            exit_code: 0,
            backend,
            space: Space::default(),
            popups: PopupManager::default(),
            output: None,
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            syncobj_state: None,
            globals: Globals {
                xdg_decoration_state,
                output_manager_state,
                presentation_state,
                pointer_constraints_state,
                relative_pointer_state,
                viewporter_state,
                single_pixel_buffer_state,
            },
            seat,
            pointer,
            cursor_status: CursorImageStatus::default_named(),
            suppressed_keys: Vec::new(),
            focus_deferred: false,
            system_keymap,
            keymap,
            policy: Policy::default(),
            front: Target::Shell,
            transitions: Transitions::default(),
            backdrops: Backdrops::default(),
            blur_cache: BlurCache::default(),
            blur: true,
            control,
            shell: None,
            screenshots: Vec::new(),
            xwayland_shell_state: XWaylandShellState::new::<Self>(&dh_for_x),
            primary_selection_state: PrimarySelectionState::new::<Self>(&dh_for_x),
            xwm: None,
            xdisplay: None,
        })
    }

    // --- Output ------------------------------------------------------------------------------

    /// Takes over the (single) output and tells clients and the shell about its size.
    pub fn set_output(&mut self, output: Output) {
        if let Some(old) = self.output.take() {
            self.space.unmap_output(&old);
        }
        self.space.map_output(&output, (0, 0));
        self.output = Some(output);
        self.output_changed();
    }

    /// The output mode changed (window resized, other mode): refit every window.
    pub fn output_changed(&mut self) {
        let Some(size) = self.output_size() else { return };
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for window in windows {
            self.fit_window(&window);
        }
        self.apply_policy();
        let message = CompositorMessage::Output { width: size.w as u32, height: size.h as u32 };
        self.control.set_greeting(message.clone());
        self.control.broadcast(&message);
    }

    pub fn output_size(&self) -> Option<Size<i32, Logical>> {
        let output = self.output.as_ref()?;
        self.space.output_geometry(output).map(|geometry| geometry.size)
    }

    /// Fullscreen for top-level windows, centered for dialogs (toplevels with a parent).
    fn fit_window(&mut self, window: &Window) {
        let Some(size) = self.output_size() else { return };
        match window.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                if toplevel.parent().is_none() {
                    toplevel.with_pending_state(|state| {
                        state.states.set(xdg_toplevel::State::Fullscreen);
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.size = Some(size);
                        state.bounds = Some(size);
                    });
                    self.space.map_element(window.clone(), (0, 0), false);
                } else {
                    toplevel.with_pending_state(|state| state.bounds = Some(size));
                    let geometry = window.geometry();
                    let location = Point::from(((size.w - geometry.size.w) / 2, (size.h - geometry.size.h) / 2));
                    self.space.map_element(window.clone(), location, false);
                }
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            WindowSurface::X11(surface) => crate::xwayland::fit_x11_window(self, window, surface, size),
        }
    }

    // --- Windows and policy ------------------------------------------------------------------

    pub fn window_id(window: &Window) -> WindowId {
        window.id() as WindowId
    }

    pub fn find_window(&self, id: WindowId) -> Option<Window> {
        self.space.elements().find(|w| Self::window_id(w) == id).cloned()
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space.elements().find(|w| w.wl_surface().is_some_and(|s| &*s == surface)).cloned()
    }

    /// The process behind a Wayland client.
    pub fn client_pid(&self, surface: &WlSurface) -> Option<u32> {
        let client = self.display_handle.get_client(surface.id()).ok()?;
        let credentials = client.get_credentials(&self.display_handle).ok()?;
        u32::try_from(credentials.pid).ok()
    }

    /// Adds a mapped window to the policy and the space.
    pub fn add_window(&mut self, window: Window, pid: Option<u32>, popup: bool, title: &str) {
        let id = Self::window_id(&window);
        if !popup {
            self.space.map_element(window.clone(), (0, 0), false);
            self.fit_window(&window);
        }
        let events = self.policy.add_window(id, pid, popup, title, &ownership::read_process);
        debug!("window {id} added (pid {pid:?}, popup {popup}, {title:?}) as {:?}", self.policy.role(id));
        self.after_policy_change(events);
    }

    pub fn remove_window(&mut self, window: &Window) {
        let id = Self::window_id(window);
        self.space.unmap_elem(window);
        let events = self.policy.remove_window(id);
        self.after_policy_change(events);
    }

    /// A window committed: once it has a buffer, the policy may bring it to the front.
    pub fn window_committed(&mut self, window: &Window) {
        let Some(surface) = window.wl_surface() else { return };
        let has_buffer = with_renderer_surface_state(&surface, |state| state.buffer().is_some()).unwrap_or(false);
        let id = Self::window_id(window);
        if has_buffer && !self.policy.is_visible(id) {
            // Apply the policy even without events: the shell's first buffer changes no front
            // window, but its window only now becomes the keyboard focus target.
            let events = self.policy.show(id);
            self.after_policy_change(events);
        }
    }

    /// Sends the policy's messages to the shell and applies stacking, keyboard and pointer focus.
    pub fn after_policy_change(&mut self, events: Vec<CompositorMessage>) {
        for event in &events {
            info!("policy: {event:?}");
            self.control.broadcast(event);
        }
        self.track_front();
        self.apply_policy();
        self.refresh_pointer_focus();
    }

    /// Starts a transition when the window in front changed (not for the quick menu, which only
    /// stacks the shell's overlay above the app).
    fn track_front(&mut self) {
        let front = self.policy.front();
        if front != self.front {
            let old = std::mem::replace(&mut self.front, front);
            if !self.policy.overlay_open() {
                let leaving = match old {
                    Target::App { app_id } => self.policy.app_has_content(app_id),
                    Target::Shell => false,
                };
                let output = self.output_size().map(|s| (s.w as f64, s.h as f64)).unwrap_or((1.0, 1.0));
                self.transitions.front_changed(old, front, leaving, output, self.clock.now().into());
            }
        }
        self.transitions.settle(front);
    }

    /// The windows of the next frame, front to back, with the transition's effects (Story 7.7).
    pub fn frame_windows(&mut self) -> Vec<Drawn> {
        let now: Duration = self.clock.now().into();
        let output = self.output_size().map(|s| (s.w as f64, s.h as f64)).unwrap_or((1.0, 1.0));
        let animating = self.transitions.tick(now);
        let mut bottom_up: Vec<(Role, bool, Drawn)> = Vec::new();
        for window in self.space.elements() {
            let id = Self::window_id(window);
            let role = self.policy.role(id).unwrap_or(Role::Unknown);
            let normal = self.policy.layer(id).is_none_or(|layer| layer == Layer::Normal);
            if animating && normal && self.transitions.hidden(role) {
                continue;
            }
            let effect = if animating && normal { self.transitions.effect(role, now, output) } else { None };
            let location = self.space.element_location(window).unwrap_or_default() - window.geometry().loc;
            let blur = self.policy.layer(id).and_then(|layer| match layer {
                Layer::Normal => None,
                Layer::Overlay => Some(WindowRole::Overlay),
                Layer::Notification => Some(WindowRole::Notification),
            });
            let blur = blur.and_then(|role| {
                let (regions, alpha) = self.backdrops.get(role).visible(now)?;
                Some(Blur { role, regions: regions.to_vec(), alpha })
            });
            bottom_up.push((role, normal, Drawn { window: window.clone(), location, effect, blur }));
        }
        // During a return the app being left shrinks above the shell (below the shell's overlays).
        if let Some(app_id) = self.transitions.raised().filter(|_| animating) {
            let (raised, mut rest): (Vec<_>, Vec<_>) =
                bottom_up.into_iter().partition(|(role, normal, _)| *normal && *role == Role::App(app_id));
            let at = rest.iter().rposition(|(role, normal, _)| *normal && *role == Role::Shell).map_or(0, |i| i + 1);
            rest.splice(at..at, raised);
            bottom_up = rest;
        }
        bottom_up.into_iter().rev().map(|(_, _, drawn)| drawn).collect()
    }

    fn focus_target(window: &Window) -> FocusTarget {
        match window.underlying_surface() {
            WindowSurface::Wayland(toplevel) => FocusTarget::Wayland(toplevel.wl_surface().clone()),
            WindowSurface::X11(surface) => FocusTarget::X11(surface.clone()),
        }
    }

    /// The policy role of the window behind a keyboard focus target.
    fn focus_role(&self, target: &FocusTarget) -> Option<crate::policy::Role> {
        let window = self.space.elements().find(|w| &Self::focus_target(w) == target)?;
        self.policy.role(Self::window_id(window))
    }

    pub fn apply_policy(&mut self) {
        let order = self.policy.stacking();
        let focus = self.policy.keyboard_focus();
        let current: Vec<WindowId> = self.space.elements().map(Self::window_id).collect();
        let wanted: Vec<WindowId> = order.iter().copied().filter(|id| current.contains(id)).collect();
        if current != wanted {
            for id in &wanted {
                if let Some(window) = self.find_window(*id) {
                    self.space.raise_element(&window, false);
                }
            }
        }
        for window in self.space.elements() {
            let active = Some(Self::window_id(window)) == focus;
            if window.set_activated(active)
                && let Some(toplevel) = window.toplevel()
                && toplevel.is_initial_configure_sent()
            {
                toplevel.send_pending_configure();
            }
        }

        let target = focus.and_then(|id| self.find_window(id)).map(|window| Self::focus_target(&window));
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let current = keyboard.current_focus();
        if current != target {
            // Back from the shell (quick menu closed with a key that is still held): wait for the
            // release, otherwise the app would see it (`on_key` applies the policy again).
            let from_shell = current.as_ref().and_then(|c| self.focus_role(c)) == Some(crate::policy::Role::Shell);
            let to_shell = focus.and_then(|id| self.policy.role(id)) == Some(crate::policy::Role::Shell);
            if from_shell && !to_shell && !keyboard.pressed_keys().is_empty() {
                debug!("keyboard focus -> {focus:?} once all keys are released");
                self.focus_deferred = true;
                return;
            }
            self.focus_deferred = false;
            debug!("keyboard focus -> {:?}", focus);
            if let Some(FocusTarget::X11(surface)) = &target {
                crate::xwayland::raise(self, surface);
            }
            keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
        }
    }

    // --- Control socket ----------------------------------------------------------------------

    pub fn handle_shell_message(&mut self, client: ControlClient, message: ShellMessage) {
        let owner = message.launched_owner();
        let events = match message {
            ShellMessage::Hello { version, pid } => {
                if version != protocol::VERSION {
                    warn!("shell speaks protocol {version}, compositor {}", protocol::VERSION);
                }
                let mut features = vec![
                    protocol::FEATURE_OVERLAYS.to_owned(),
                    protocol::FEATURE_GUIDE_KEY.to_owned(),
                    protocol::FEATURE_TRANSITIONS.to_owned(),
                ];
                if self.blur {
                    features.push(protocol::FEATURE_BLUR.to_owned());
                }
                self.control.send_to(client, &CompositorMessage::Welcome { version: protocol::VERSION, features });
                self.policy.set_shell_pid(pid, &ownership::read_process)
            }
            ShellMessage::Launched { app_id, .. } => {
                info!("shell launched app {app_id} ({owner:?})");
                self.policy.launched(app_id, owner, &ownership::read_process)
            }
            ShellMessage::Exited { app_id } => self.policy.exited(app_id),
            ShellMessage::Focus { target, transition } => {
                self.transitions.request(target, transition, self.clock.now().into());
                self.policy.set_target(target)
            }
            ShellMessage::Overlay { open } => {
                info!("overlay {}", if open { "open" } else { "closed" });
                self.policy.set_overlay(open)
            }
            ShellMessage::WindowRole { title, role } => {
                info!("shell window {title:?} has the role {role:?}");
                self.policy.set_window_role(&title, role)
            }
            ShellMessage::Blur { role, regions, fade_ms } => {
                debug!("blur under the {role:?} window: {regions:?}");
                let fade = Duration::from_millis(fade_ms.min(2000) as u64);
                self.backdrops.get_mut(role).set(regions, fade, self.clock.now().into());
                Vec::new()
            }
            ShellMessage::Screenshot { path } => {
                self.screenshots.push(ScreenshotRequest { path: path.into(), reply: Some(client) });
                Vec::new()
            }
            ShellMessage::KeyboardLayout { layout } => {
                self.set_keyboard_layout(layout.as_deref());
                Vec::new()
            }
        };
        self.after_policy_change(events);
    }

    /// Switches the seat keyboard to `layout` (`None`: the system layout) at runtime; clients get
    /// the new keymap through `wl_keyboard.keymap` (XWayland passes it on to X11 games).
    fn set_keyboard_layout(&mut self, layout: Option<&str>) {
        let wanted = self.system_keymap.with_layout(layout);
        if wanted != self.keymap
            && let Some(keyboard) = self.seat.get_keyboard()
        {
            match keyboard.set_xkb_config(self, xkb_config(&wanted)) {
                Ok(()) => {
                    info!("keyboard layout -> {}", wanted.describe());
                    self.keymap = wanted;
                }
                Err(err) => {
                    warn!("keyboard layout {layout:?} does not compile ({err}), keeping {}", self.keymap.describe())
                }
            }
        }
        let message = keymap_message(&self.keymap);
        self.control.set_greeting(message.clone());
        self.control.broadcast(&message);
    }

    /// Takes a screenshot `delay` after now (`--screenshot-after`).
    pub fn schedule_screenshot(&mut self, delay: Duration, path: PathBuf) {
        let timer = smithay::reexports::calloop::timer::Timer::from_duration(delay);
        let result = self.loop_handle.insert_source(timer, move |_, _, state| {
            state.screenshots.push(ScreenshotRequest { path: path.clone(), reply: None });
            smithay::reexports::calloop::timer::TimeoutAction::Drop
        });
        if let Err(err) = result {
            warn!("cannot schedule screenshot: {err}");
        }
    }

    pub fn screenshot_done(&mut self, request: ScreenshotRequest, result: Result<(), String>) {
        match &result {
            Ok(()) => info!("screenshot saved: {}", request.path.display()),
            Err(err) => warn!("screenshot {} failed: {err}", request.path.display()),
        }
        if let Some(client) = request.reply {
            let path = request.path.to_string_lossy().into_owned();
            self.control.send_to(client, &CompositorMessage::ScreenshotDone { path, error: result.err() });
        }
    }
}

// --- Protocol handlers -----------------------------------------------------------------------

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &state.compositor_state;
        }
        &client.get_data::<ClientState>().expect("client without state").compositor_state
    }

    /// Waits for dmabufs to be ready (implicit fence or explicit-sync acquire point) before the
    /// commit is applied, like anvil.
    fn new_surface(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            let (acquire_point, dmabuf) = with_states(surface, |data| {
                let acquire = data.cached_state.get::<DrmSyncobjCachedState>().pending().acquire_point.clone();
                let dmabuf =
                    data.cached_state.get::<SurfaceAttributes>().pending().buffer.as_ref().and_then(|assignment| {
                        match assignment {
                            BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                            _ => None,
                        }
                    });
                (acquire, dmabuf)
            });
            let Some(dmabuf) = dmabuf else { return };
            let Some(client) = surface.client() else { return };
            if let Some(acquire_point) = acquire_point
                && let Ok((blocker, source)) = acquire_point.generate_blocker()
            {
                let client = client.clone();
                let inserted = state.loop_handle.insert_source(source, move |_, _, state| {
                    let dh = state.display_handle.clone();
                    state.client_compositor_state(&client).blocker_cleared(state, &dh);
                    Ok(())
                });
                if inserted.is_ok() {
                    add_blocker(surface, blocker);
                    return;
                }
            }
            if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                let inserted = state.loop_handle.insert_source(source, move |_, _, state| {
                    let dh = state.display_handle.clone();
                    state.client_compositor_state(&client).blocker_cleared(state, &dh);
                    Ok(())
                });
                if inserted.is_ok() {
                    add_blocker(surface, blocker);
                }
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                if let Some(toplevel) = window.toplevel() {
                    let initial_sent = with_states(&root, |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .is_some_and(|data| data.lock().unwrap().initial_configure_sent)
                    });
                    // The initial configure carries the fullscreen state set in `new_toplevel`
                    // or `fullscreen_request`, so a request before the first commit is honoured.
                    if !initial_sent {
                        toplevel.send_configure();
                    } else if toplevel.parent().is_some() {
                        self.fit_window(&window);
                    }
                }
                self.window_committed(&window);
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
            && let Err(err) = popup.send_configure()
        {
            warn!("initial popup configure failed: {err}");
        }
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // No configure here: the initial configure goes out on the first commit.
        let pid = self.client_pid(surface.wl_surface());
        let title = with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().unwrap().title.clone())
                .unwrap_or_default()
        });
        surface.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
        let window = Window::new_wayland_window(surface);
        self.add_window(window, pid, false, &title);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.remove_window(&window);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let title = with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().unwrap().title.clone())
                .unwrap_or_default()
        });
        let events = self.policy.set_title(Self::window_id(&window), &title);
        self.after_policy_change(events);
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.fit_window(&window);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            warn!("cannot track popup: {err}");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // Popup grabs (menus that close on an outside click) are not needed by the shell or
        // games; see Story 7.4 for desktop apps.
    }

    /// Everything is fullscreen anyway; the request is answered with the current state. Setting
    /// the state before the first commit is what fixes the spike's "ignored fullscreen" issue.
    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.fit_window(&window);
        }
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        // Console policy: top-level windows stay fullscreen. The protocol still wants a reply.
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }
}

impl State {
    /// Keeps popups inside the output (from smallvil, MIT).
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };
        let Some(window) = self.window_for_surface(&root) else { return };
        let Some(output) = self.output.as_ref() else { return };
        let (Some(output_geometry), Some(window_geometry)) =
            (self.space.output_geometry(output), self.space.element_geometry(&window))
        else {
            return;
        };
        let mut target: Rectangle<i32, Logical> = output_geometry;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geometry.loc;
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl XdgDecorationHandler for State {
    /// Server side, and the server draws nothing: fullscreen windows have no decorations.
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        self.new_decoration(toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.new_decoration(toplevel);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = FocusTarget;
    type PointerFocus = FocusTarget;
    type TouchFocus = FocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, target: Option<&FocusTarget>) {
        let client = target
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, leds: smithay::input::keyboard::LedState) {
        self.backend.update_led_state(leds);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        target: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        crate::xwayland::new_wayland_selection(self, target, source);
    }

    fn send_selection(
        &mut self,
        target: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        crate::xwayland::send_selection_to_x11(self, target, mime_type, fd);
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DndGrabHandler for State {}

impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        _seat: Seat<Self>,
        _serial: Serial,
        _type: GrabType,
    ) {
        // No drag and drop on a console.
        source.cancel();
    }
}

impl OutputHandler for State {}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let imported = self.backend.renderer().is_some_and(|renderer| renderer.import_dmabuf(&dmabuf, None).is_ok());
        if imported {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.syncobj_state.as_mut()
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // Games lock the pointer for mouse look; grant it while the surface has the pointer.
        if pointer.current_focus().and_then(|focus| focus.wl_surface().map(|s| s.into_owned())).as_ref()
            == Some(surface)
        {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }
}

smithay::delegate_dispatch2!(State);

/// The xkb names of a [`Keymap`]; empty fields fall back to `XKB_DEFAULT_*` and then xkbcommon's
/// defaults.
fn xkb_config(keymap: &Keymap) -> XkbConfig<'_> {
    XkbConfig {
        rules: &keymap.rules,
        model: &keymap.model,
        layout: &keymap.layout,
        variant: &keymap.variant,
        options: (!keymap.options.is_empty()).then(|| keymap.options.clone()),
    }
}

fn keymap_message(keymap: &Keymap) -> CompositorMessage {
    CompositorMessage::Keymap { layout: keymap.layout.clone(), variant: keymap.variant.clone() }
}
