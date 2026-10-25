//! Keyboard/pointer/touch focus target: a Wayland surface or an X11 window.
//!
//! Adapted from Smithay's anvil (`anvil/src/focus.rs`, MIT, Copyright (c) 2017 Victor Berger),
//! reduced to the two surface kinds a console shell needs.

use std::borrow::Cow;
use std::sync::Arc;

use smithay::backend::input::{InputTime, KeyState};
use smithay::input::Seat;
use smithay::input::dnd::{DndFocus, OfferData, Source};
use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
    PointerTarget, RelativeMotionEvent,
};
use smithay::input::touch::{DownEvent, FrameMarker, OrientationEvent, ShapeEvent, TouchTarget, UpEvent};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Point, Serial};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::WlOfferData;
use smithay::xwayland::X11Surface;
use smithay::xwayland::xwm::XwmOfferData;

use crate::state::State;

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // short-lived, cloned rarely; boxing buys nothing here
pub enum FocusTarget {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl FocusTarget {
    fn keyboard(&self) -> &dyn KeyboardTarget<State> {
        match self {
            FocusTarget::Wayland(surface) => surface,
            FocusTarget::X11(surface) => surface,
        }
    }

    fn pointer(&self) -> &dyn PointerTarget<State> {
        match self {
            FocusTarget::Wayland(surface) => surface,
            FocusTarget::X11(surface) => surface,
        }
    }

    fn touch(&self) -> &dyn TouchTarget<State> {
        match self {
            FocusTarget::Wayland(surface) => surface,
            FocusTarget::X11(surface) => surface,
        }
    }
}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        match self {
            FocusTarget::Wayland(surface) => surface.alive(),
            FocusTarget::X11(surface) => surface.alive(),
        }
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            FocusTarget::Wayland(surface) => Some(Cow::Borrowed(surface)),
            FocusTarget::X11(surface) => surface.wl_surface().map(Cow::Owned),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            FocusTarget::Wayland(surface) => surface.same_client_as(object_id),
            FocusTarget::X11(surface) => surface.same_client_as(object_id),
        }
    }
}

impl From<WlSurface> for FocusTarget {
    fn from(surface: WlSurface) -> Self {
        FocusTarget::Wayland(surface)
    }
}

impl From<X11Surface> for FocusTarget {
    fn from(surface: X11Surface) -> Self {
        FocusTarget::X11(surface)
    }
}

impl KeyboardTarget<State> for FocusTarget {
    fn enter(&self, seat: &Seat<State>, data: &mut State, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        self.keyboard().enter(seat, data, keys, serial)
    }
    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        self.keyboard().leave(seat, data, serial)
    }
    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
        self.keyboard().key(seat, data, key, state, serial, time)
    }
    fn modifiers(&self, seat: &Seat<State>, data: &mut State, modifiers: ModifiersState, serial: Serial) {
        self.keyboard().modifiers(seat, data, modifiers, serial)
    }
}

impl PointerTarget<State> for FocusTarget {
    fn enter(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        self.pointer().enter(seat, data, event)
    }
    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        self.pointer().motion(seat, data, event)
    }
    fn relative_motion(&self, seat: &Seat<State>, data: &mut State, event: &RelativeMotionEvent) {
        self.pointer().relative_motion(seat, data, event)
    }
    fn button(&self, seat: &Seat<State>, data: &mut State, event: &ButtonEvent) {
        self.pointer().button(seat, data, event)
    }
    fn axis(&self, seat: &Seat<State>, data: &mut State, frame: AxisFrame) {
        self.pointer().axis(seat, data, frame)
    }
    fn frame(&self, seat: &Seat<State>, data: &mut State) {
        self.pointer().frame(seat, data)
    }
    fn gesture_swipe_begin(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeBeginEvent) {
        self.pointer().gesture_swipe_begin(seat, data, event)
    }
    fn gesture_swipe_update(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeUpdateEvent) {
        self.pointer().gesture_swipe_update(seat, data, event)
    }
    fn gesture_swipe_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureSwipeEndEvent) {
        self.pointer().gesture_swipe_end(seat, data, event)
    }
    fn gesture_pinch_begin(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchBeginEvent) {
        self.pointer().gesture_pinch_begin(seat, data, event)
    }
    fn gesture_pinch_update(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchUpdateEvent) {
        self.pointer().gesture_pinch_update(seat, data, event)
    }
    fn gesture_pinch_end(&self, seat: &Seat<State>, data: &mut State, event: &GesturePinchEndEvent) {
        self.pointer().gesture_pinch_end(seat, data, event)
    }
    fn gesture_hold_begin(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldBeginEvent) {
        self.pointer().gesture_hold_begin(seat, data, event)
    }
    fn gesture_hold_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldEndEvent) {
        self.pointer().gesture_hold_end(seat, data, event)
    }
    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial, time: InputTime) {
        self.pointer().leave(seat, data, serial, time)
    }
}

impl TouchTarget<State> for FocusTarget {
    fn down(&self, seat: &Seat<State>, data: &mut State, event: &DownEvent) {
        self.touch().down(seat, data, event)
    }
    fn up(&self, seat: &Seat<State>, data: &mut State, event: &UpEvent) {
        self.touch().up(seat, data, event)
    }
    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &smithay::input::touch::MotionEvent) {
        TouchTarget::motion(self.touch(), seat, data, event)
    }
    fn frame(&self, seat: &Seat<State>, data: &mut State, marker: FrameMarker) {
        TouchTarget::frame(self.touch(), seat, data, marker)
    }
    fn cancel(&self, seat: &Seat<State>, data: &mut State, marker: FrameMarker) {
        self.touch().cancel(seat, data, marker)
    }
    fn shape(&self, seat: &Seat<State>, data: &mut State, event: &ShapeEvent) {
        self.touch().shape(seat, data, event)
    }
    fn orientation(&self, seat: &Seat<State>, data: &mut State, event: &OrientationEvent) {
        self.touch().orientation(seat, data, event)
    }
    fn last_frame(&self, seat: &Seat<State>, data: &mut State) -> Option<FrameMarker> {
        self.touch().last_frame(seat, data)
    }
}

/// Drag and drop between Wayland and X11 clients (required by Smithay's X11 window manager).
#[allow(dead_code)] // the X11 variant is used with XWayland (Story 7.3)
pub enum FocusOfferData<S: Source> {
    Wayland(WlOfferData<S>),
    X11(XwmOfferData<S>),
}

impl<S: Source> OfferData for FocusOfferData<S> {
    fn disable(&self) {
        match self {
            FocusOfferData::Wayland(data) => data.disable(),
            FocusOfferData::X11(data) => data.disable(),
        }
    }
    fn drop(&self) {
        match self {
            FocusOfferData::Wayland(data) => data.drop(),
            FocusOfferData::X11(data) => data.drop(),
        }
    }
    fn validated(&self) -> bool {
        match self {
            FocusOfferData::Wayland(data) => data.validated(),
            FocusOfferData::X11(data) => data.validated(),
        }
    }
}

impl DndFocus<State> for FocusTarget {
    type OfferData<S>
        = FocusOfferData<S>
    where
        S: Source;

    fn enter<S: Source>(
        &self,
        data: &mut State,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<FocusOfferData<S>> {
        match self {
            FocusTarget::Wayland(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial).map(FocusOfferData::Wayland)
            }
            // X11 drag and drop needs the X11 window manager (Story 7.3).
            FocusTarget::X11(_) => None,
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut FocusOfferData<S>>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        match (self, offer) {
            (FocusTarget::Wayland(surface), Some(FocusOfferData::Wayland(offer))) => {
                DndFocus::motion(surface, data, Some(offer), seat, location, time)
            }
            (FocusTarget::Wayland(surface), None) => DndFocus::motion(surface, data, None::<&mut WlOfferData<S>>, seat, location, time),
            _ => {}
        }
    }

    fn leave<S: Source>(&self, data: &mut State, offer: Option<&mut FocusOfferData<S>>, seat: &Seat<State>) {
        match (self, offer) {
            (FocusTarget::Wayland(surface), Some(FocusOfferData::Wayland(offer))) => {
                DndFocus::leave(surface, data, Some(offer), seat)
            }
            (FocusTarget::Wayland(surface), None) => DndFocus::leave(surface, data, None::<&mut WlOfferData<S>>, seat),
            _ => {}
        }
    }

    fn drop<S: Source>(&self, data: &mut State, offer: Option<&mut FocusOfferData<S>>, seat: &Seat<State>) {
        match (self, offer) {
            (FocusTarget::Wayland(surface), Some(FocusOfferData::Wayland(offer))) => {
                DndFocus::drop(surface, data, Some(offer), seat)
            }
            (FocusTarget::Wayland(surface), None) => DndFocus::drop(surface, data, None::<&mut WlOfferData<S>>, seat),
            _ => {}
        }
    }
}
