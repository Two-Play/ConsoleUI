//! XWayland (Story 7.3): Smithay's X11 window manager for X11 games and apps. Top-level X11
//! windows are made fullscreen like Wayland ones, dialogs (transient windows) are centered, and
//! override-redirect windows (menus, tooltips, splash screens) keep their own position and are
//! drawn above their owner. Every window goes through the same policy as Wayland windows; its
//! process comes from `_NET_WM_PID` or, for clients that do not set it (e.g. `glxgears`), from
//! the X server itself (X-Resource `QueryClientIds`).
//!
//! The selection and window manager glue follows anvil (`anvil/src/shell/x11.rs`, `state.rs`,
//! MIT, Copyright (c) 2017 Victor Berger).

use std::os::fd::OwnedFd;
use std::process::Stdio;

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata, request_data_device_client_selection,
    set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
    set_primary_selection,
};
use smithay::wayland::selection::{SelectionSource, SelectionTarget};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::{Reorder, ResizeEdge, WmWindowType, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler};
use tracing::{info, warn};

use crate::child;
use crate::focus::FocusTarget;
use crate::state::State;

/// Starts XWayland; the shell is started once it is ready, so it gets `DISPLAY`. Without
/// XWayland (failed to start) the shell still starts, without `DISPLAY`.
pub fn start(state: &mut State, shell: Option<Vec<String>>) {
    let spawned = XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        std::iter::empty::<String>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    );
    let (xwayland, client) = match spawned {
        Ok(spawned) => spawned,
        Err(err) => {
            warn!("cannot start XWayland: {err}");
            child::start_shell_or_stop(state, shell.as_deref(), None);
            return;
        }
    };
    let inserted = state.loop_handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready { x11_socket, display_number } => {
            let dh = state.display_handle.clone();
            match X11Wm::start_wm(state.loop_handle.clone(), &dh, x11_socket, client.clone()) {
                Ok(wm) => {
                    info!("XWayland ready on DISPLAY=:{display_number}");
                    state.xwm = Some(wm);
                    state.xdisplay = Some(display_number);
                    child::start_shell_or_stop(state, shell.as_deref(), Some(display_number));
                }
                Err(err) => {
                    warn!("cannot start the X11 window manager: {err}");
                    child::start_shell_or_stop(state, shell.as_deref(), None);
                }
            }
        }
        XWaylandEvent::Error => {
            warn!("XWayland crashed on startup");
            if state.shell.is_none() {
                child::start_shell_or_stop(state, shell.as_deref(), None);
            }
        }
    });
    if let Err(err) = inserted {
        warn!("cannot watch XWayland: {err}");
    }
}

/// Dialogs (transient for another window, or typed as dialog/splash) keep their size.
fn is_dialog(surface: &X11Surface) -> bool {
    surface.is_transient_for().is_some()
        || matches!(surface.window_type(), Some(WmWindowType::Dialog | WmWindowType::Splash | WmWindowType::Utility))
}

/// Fullscreen for top-level windows, centered for dialogs; override-redirect windows place
/// themselves.
pub fn fit_x11_window(state: &mut State, window: &Window, surface: &X11Surface, size: Size<i32, Logical>) {
    if surface.is_override_redirect() {
        return;
    }
    let geometry = if is_dialog(surface) {
        let own = surface.geometry().size;
        let own = Size::from((own.w.clamp(1, size.w), own.h.clamp(1, size.h)));
        Rectangle::new(Point::from(((size.w - own.w) / 2, (size.h - own.h) / 2)), own)
    } else {
        if let Err(err) = surface.set_fullscreen(true) {
            warn!("X11 window {}: cannot set fullscreen: {err}", surface.window_id());
        }
        Rectangle::from_size(size)
    };
    if let Err(err) = surface.configure(geometry) {
        warn!("X11 window {}: cannot configure: {err}", surface.window_id());
    }
    state.space.map_element(window.clone(), geometry.loc, false);
}

pub fn raise(state: &mut State, surface: &X11Surface) {
    if let Some(xwm) = state.xwm.as_mut()
        && let Err(err) = xwm.raise_window(surface)
    {
        warn!("cannot raise X11 window {}: {err}", surface.window_id());
    }
}

pub fn new_wayland_selection(state: &mut State, target: SelectionTarget, source: Option<SelectionSource>) {
    if let Some(xwm) = state.xwm.as_mut()
        && let Err(err) = xwm.new_selection(target, source.map(|source| source.mime_types()))
    {
        warn!("cannot set the X11 selection: {err}");
    }
}

pub fn send_selection_to_x11(state: &mut State, target: SelectionTarget, mime_type: String, fd: OwnedFd) {
    if let Some(xwm) = state.xwm.as_mut()
        && let Err(err) = xwm.send_selection(target, mime_type, fd)
    {
        warn!("cannot send the selection to X11: {err}");
    }
}

impl State {
    fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space.elements().find(|w| w.x11_surface() == Some(surface)).cloned()
    }

    /// The process of an X11 window: `_NET_WM_PID`, else the X client's PID from the server.
    fn x11_pid(surface: &X11Surface) -> Option<u32> {
        surface.pid().or_else(|| surface.get_client_pid().ok()).filter(|&pid| pid != 0)
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// The X11 window got its Wayland surface; it may already have content.
    fn surface_associated(&mut self, _xwm: XwmId, _wl_surface: WlSurface, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.window_committed(&window);
        }
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("X11 event without window manager")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(err) = surface.set_mapped(true) {
            warn!("cannot map X11 window {}: {err}", surface.window_id());
            return;
        }
        let pid = Self::x11_pid(&surface);
        info!("X11 window {} {:?} (class {:?}, pid {pid:?})", surface.window_id(), surface.title(), surface.class());
        let title = surface.title();
        self.add_window(Window::new_x11_window(surface), pid, false, &title);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let location = surface.geometry().loc;
        let pid = Self::x11_pid(&surface);
        let title = surface.title();
        let window = Window::new_x11_window(surface);
        self.space.map_element(window.clone(), location, false);
        self.add_window(window, pid, true, &title);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.remove_window(&window);
        }
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.remove_window(&window);
        }
    }

    /// Top-level windows stay fullscreen whatever they ask for; dialogs may change their size.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geometry = surface.geometry();
        if let Some(w) = w {
            geometry.size.w = w as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h as i32;
        }
        match self.x11_window(&surface) {
            Some(window) if !is_dialog(&surface) => {
                if let Some(size) = self.output_size() {
                    fit_x11_window(self, &window, &surface, size);
                }
            }
            Some(window) => {
                let _ = surface.configure(geometry);
                if let Some(size) = self.output_size() {
                    fit_x11_window(self, &window, &surface, size);
                }
            }
            // Not mapped yet: let it have its size, it becomes fullscreen when it maps.
            None => {
                let _ = surface.configure(geometry);
            }
        }
        self.apply_policy();
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // Override-redirect windows move themselves.
        if surface.is_override_redirect()
            && let Some(window) = self.x11_window(&surface)
        {
            self.space.relocate_element(&window, geometry.loc);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let (Some(window), Some(size)) = (self.x11_window(&surface), self.output_size()) {
            fit_x11_window(self, &window, &surface, size);
            self.apply_policy();
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        // Console policy: stays fullscreen.
        self.fullscreen_request(_xwm, surface);
    }

    fn maximize_request(&mut self, xwm: XwmId, surface: X11Surface) {
        self.fullscreen_request(xwm, surface);
    }

    fn resize_request(&mut self, _xwm: XwmId, _surface: X11Surface, _button: u32, _edges: ResizeEdge) {}

    fn move_request(&mut self, _xwm: XwmId, _surface: X11Surface, _button: u32) {}

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        // Only the focused X11 client may read the clipboard.
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| matches!(focus, FocusTarget::X11(surface) if surface.xwm_id() == Some(xwm)))
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let result = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}"))
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}"))
            }
        };
        if let Err(err) = result {
            warn!("cannot pass the Wayland selection to X11: {err}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&self.display_handle, &self.seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        warn!("the X11 window manager lost its connection");
        self.xwm = None;
    }
}
