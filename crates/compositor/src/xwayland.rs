//! XWayland integration (Story 7.3). Until then the compositor only serves Wayland clients.

use std::os::fd::OwnedFd;

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use smithay::wayland::selection::{SelectionSource, SelectionTarget};
use smithay::xwayland::X11Surface;

use crate::state::State;

pub fn fit_x11_window(_state: &mut State, _window: &Window, _surface: &X11Surface, _size: Size<i32, Logical>) {}

pub fn raise(_state: &mut State, _surface: &X11Surface) {}

pub fn commit(_state: &mut State, _surface: &WlSurface) {}

pub fn new_wayland_selection(_state: &mut State, _target: SelectionTarget, _source: Option<SelectionSource>) {}

pub fn send_selection_to_x11(_state: &mut State, _target: SelectionTarget, _mime_type: String, _fd: OwnedFd) {}
