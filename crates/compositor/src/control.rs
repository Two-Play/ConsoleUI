//! Control socket for the shell (`CONSOLEUI_COMPOSITOR`): a Unix socket with line-delimited JSON,
//! message types in `consoleui_core::protocol`. Several clients may connect (the shell plus e.g.
//! a test script); compositor messages go to all of them.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use consoleui_core::protocol::{self, CompositorMessage, LineReader, ShellMessage};
use tracing::{debug, info, warn};

use crate::state::State;

pub type ClientId = u64;

struct Client {
    id: ClientId,
    /// Write side (the read side lives in the event source)
    stream: UnixStream,
    outgoing: Vec<u8>,
}

pub struct Control {
    path: PathBuf,
    clients: Vec<Client>,
    next_id: ClientId,
    /// Sent to every new client (e.g. the output size)
    greeting: Vec<CompositorMessage>,
}

impl Control {
    /// Listens on `path` (an old socket file there is replaced).
    pub fn listen(path: &Path, handle: &LoopHandle<'static, State>) -> io::Result<Self> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        handle
            .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state| {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => state.control_accept(stream),
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        Err(err) => {
                            warn!("control socket: accept failed: {err}");
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            })
            .map_err(|err| io::Error::other(err.to_string()))?;
        info!("control socket at {}", path.display());
        Ok(Self { path: path.to_owned(), clients: Vec::new(), next_id: 1, greeting: Vec::new() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a shell is connected (it may then claim reserved keys like Guide).
    pub fn has_clients(&self) -> bool {
        !self.clients.is_empty()
    }

    /// Replaces the greeting entry of the same kind (e.g. a new output size).
    pub fn set_greeting(&mut self, message: CompositorMessage) {
        self.greeting.retain(|m| std::mem::discriminant(m) != std::mem::discriminant(&message));
        self.greeting.push(message);
    }

    pub fn broadcast(&mut self, message: &CompositorMessage) {
        debug!("control: -> {message:?}");
        let line = protocol::encode(message);
        for client in &mut self.clients {
            client.outgoing.extend_from_slice(line.as_bytes());
        }
        self.flush();
    }

    pub fn send_to(&mut self, id: ClientId, message: &CompositorMessage) {
        let line = protocol::encode(message);
        if let Some(client) = self.clients.iter_mut().find(|c| c.id == id) {
            client.outgoing.extend_from_slice(line.as_bytes());
        }
        self.flush();
    }

    /// Writes what the non-blocking sockets did not take yet; drops clients that are gone.
    pub fn flush(&mut self) {
        self.clients.retain_mut(|client| {
            while !client.outgoing.is_empty() {
                match client.stream.write(&client.outgoing) {
                    Ok(0) => return false,
                    Ok(n) => {
                        client.outgoing.drain(..n);
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return false,
                }
            }
            true
        });
    }

    fn remove(&mut self, id: ClientId) {
        self.clients.retain(|c| c.id != id);
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl State {
    fn control_accept(&mut self, stream: UnixStream) {
        let Ok(write_side) = stream.try_clone() else { return };
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let id = self.control.next_id;
        self.control.next_id += 1;
        let mut reader = LineReader::default();
        let source = Generic::new(stream, Interest::READ, Mode::Level);
        let inserted = self.loop_handle.insert_source(source, move |_, stream, state| {
            let mut buffer = [0u8; 4096];
            loop {
                match (&**stream).read(&mut buffer) {
                    Ok(0) => {
                        debug!("control: client {id} disconnected");
                        state.control.remove(id);
                        return Ok(PostAction::Remove);
                    }
                    Ok(n) => reader.push(&buffer[..n]),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        state.control.remove(id);
                        return Ok(PostAction::Remove);
                    }
                }
            }
            while let Some(message) = reader.next_message::<ShellMessage>() {
                debug!("control: <- {message:?}");
                state.handle_shell_message(id, message);
            }
            Ok(PostAction::Continue)
        });
        if inserted.is_err() {
            return;
        }
        let _ = write_side.set_nonblocking(true);
        debug!("control: client {id} connected");
        self.control.clients.push(Client { id, stream: write_side, outgoing: Vec::new() });
        for message in self.control.greeting.clone() {
            self.control.send_to(id, &message);
        }
    }
}
