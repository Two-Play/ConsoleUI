//! The shell as the input method of our compositor (`zwp_input_method_v2`, announced by the
//! compositor's feature `text_input`): apps tell the compositor when a text field has the focus
//! (`zwp_text_input_v3`), the compositor passes it on, and the shell shows its on-screen keyboard
//! and types into the field with `commit_string`.
//!
//! The Wayland connection runs on a thread of its own, because Slint owns the one of the windows;
//! this module is its handle: events come in through [`InputMethod::poll`], requests go out
//! through [`InputMethod::commit`] and [`InputMethod::delete_before`].

use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::ContentPurpose;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::{self, ZwpInputMethodV2};

/// What the app's text field is like
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextField {
    /// A password or PIN: the keyboard hides what is typed
    pub secret: bool,
    /// Digits only (number, phone, PIN)
    pub numeric: bool,
    /// The text before the cursor, if the app tells (never for secrets)
    pub before_cursor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A text field has the focus (or its content changed)
    Active(TextField),
    /// No text field has the focus any more
    Inactive,
}

enum Request {
    Commit(String),
    DeleteBefore(u32),
}

pub struct InputMethod {
    events: Receiver<Event>,
    requests: Sender<Request>,
}

impl InputMethod {
    /// Connects to the compositor and binds the input method. `None` when it does not offer one
    /// (older compositor, or the shell is not allowed to bind it).
    pub fn start() -> Option<Self> {
        let connection = Connection::connect_to_env().map_err(|err| eprintln!("Input method: {err}")).ok()?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&connection).map_err(|err| eprintln!("Input method: {err}")).ok()?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat =
            globals.bind(&qh, 1..=1, ()).map_err(|err| eprintln!("Input method: no seat ({err})")).ok()?;
        let manager: ZwpInputMethodManagerV2 =
            globals.bind(&qh, 1..=1, ()).map_err(|err| eprintln!("Input method not offered: {err}")).ok()?;
        let method = manager.get_input_method(&seat, &qh, ());

        let (events_tx, events) = mpsc::channel();
        let (requests, requests_rx) = mpsc::channel();
        let mut state = State {
            events: events_tx,
            serial: 0,
            pending: Pending::default(),
            active: false,
            field: TextField::default(),
            unavailable: false,
        };
        std::thread::spawn(move || {
            loop {
                loop {
                    match requests_rx.try_recv() {
                        Ok(Request::Commit(text)) if state.active => {
                            method.commit_string(text);
                            method.commit(state.serial);
                        }
                        Ok(Request::DeleteBefore(bytes)) if state.active => {
                            method.delete_surrounding_text(bytes, 0);
                            method.commit(state.serial);
                        }
                        Ok(_) => {}
                        Err(TryRecvError::Empty) => break,
                        // The shell is gone
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                if queue.dispatch_pending(&mut state).is_err() || queue.flush().is_err() || state.unavailable {
                    return;
                }
                if let Some(guard) = queue.prepare_read() {
                    let mut poll =
                        libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
                    // SAFETY: one valid pollfd for the duration of the call
                    let ready = unsafe { libc::poll(&mut poll, 1, 50) };
                    if ready > 0 && guard.read().is_err() {
                        return;
                    }
                }
            }
        });
        Some(Self { events, requests })
    }

    /// Events since the last call
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Types text into the field that has the focus.
    pub fn commit(&self, text: impl Into<String>) {
        let _ = self.requests.send(Request::Commit(text.into()));
    }

    /// Deletes `bytes` bytes (whole characters) before the cursor.
    pub fn delete_before(&self, bytes: u32) {
        let _ = self.requests.send(Request::DeleteBefore(bytes));
    }
}

/// What the compositor sends between two `done` events; it counts once the `done` arrives.
#[derive(Default)]
struct Pending {
    active: bool,
    before_cursor: String,
    secret: bool,
    numeric: bool,
}

struct State {
    events: Sender<Event>,
    /// Number of `done` events, as the protocol wants it in `commit`
    serial: u32,
    pending: Pending,
    active: bool,
    field: TextField,
    unavailable: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpInputMethodManagerV2,
        _: <ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.pending = Pending { active: true, ..Pending::default() },
            zwp_input_method_v2::Event::Deactivate => state.pending.active = false,
            zwp_input_method_v2::Event::SurroundingText { text, cursor, .. } => {
                let end = (cursor as usize).min(text.len());
                state.pending.before_cursor = text.get(..end).unwrap_or_default().to_owned();
            }
            zwp_input_method_v2::Event::ContentType { purpose, .. } => {
                let purpose = match purpose {
                    WEnum::Value(purpose) => Some(purpose),
                    WEnum::Unknown(_) => None,
                };
                state.pending.secret = matches!(purpose, Some(ContentPurpose::Password | ContentPurpose::Pin));
                state.pending.numeric = matches!(
                    purpose,
                    Some(ContentPurpose::Digits | ContentPurpose::Number | ContentPurpose::Phone | ContentPurpose::Pin)
                );
            }
            zwp_input_method_v2::Event::Done => {
                state.serial = state.serial.wrapping_add(1);
                let pending = &state.pending;
                let field = TextField {
                    secret: pending.secret,
                    numeric: pending.numeric,
                    before_cursor: if pending.secret { String::new() } else { pending.before_cursor.clone() },
                };
                if pending.active && (!state.active || state.field != field) {
                    state.field = field.clone();
                    let _ = state.events.send(Event::Active(field));
                } else if !pending.active && state.active {
                    let _ = state.events.send(Event::Inactive);
                }
                state.active = pending.active;
            }
            zwp_input_method_v2::Event::Unavailable => state.unavailable = true,
            _ => {}
        }
    }
}
