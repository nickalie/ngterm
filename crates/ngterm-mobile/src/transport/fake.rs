//! The scripted in-memory transport: a machine is refused or connects, a command with a script answers it and ends,
//! and any other command's channel is handed to the test ([`FakeTransport::take_opened`]) to drive by hand.
//!
//! The state is shared behind a mutex, so a test keeps a clone of the [`FakeTransport`] it moved onto the actor and
//! inspects what the core did through it.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ngterm_term::GridSize;

use super::{Channel, ChannelEnd, ChannelPeer, Connection, Transport, TransportError};
use crate::api::Machine;

/// A command's scripted answer: its output, then its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub end: ChannelEnd,
}

impl Script {
    /// A command that prints `stdout` and exits 0.
    #[must_use]
    pub fn ok(stdout: &str) -> Self {
        Self {
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
            end: ChannelEnd::Exit {
                code: Some(0),
                signal: None,
            },
        }
    }

    /// A command that prints `stderr` and exits with `code`.
    #[must_use]
    pub fn failing(code: u32, stderr: &str) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            end: ChannelEnd::Exit {
                code: Some(code),
                signal: None,
            },
        }
    }
}

/// An exec channel the fake opened with no script: the test drives its far end.
#[derive(Debug)]
pub struct Opened {
    pub machine_id: String,
    pub command: String,
    pub pty: Option<GridSize>,
    pub peer: ChannelPeer,
}

/// One exec the core ran, scripted or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    pub machine_id: String,
    pub command: String,
    pub pty: Option<GridSize>,
}

#[derive(Debug, Default)]
struct State {
    refusals: HashMap<String, String>,
    scripts: HashMap<String, Script>,
    connects: Vec<String>,
    closed: Vec<String>,
    execs: Vec<Exec>,
    opened: VecDeque<Opened>,
}

/// The scripted transport; clones share one state.
#[derive(Debug, Clone, Default)]
pub struct FakeTransport {
    state: Arc<Mutex<State>>,
}

impl FakeTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Refuses every connect to `host` with `reason`.
    pub fn refuse(&self, host: &str, reason: &str) {
        self.lock().refusals.insert(host.to_owned(), reason.to_owned());
    }

    /// Accepts connects to `host` again.
    pub fn accept(&self, host: &str) {
        self.lock().refusals.remove(host);
    }

    /// Answers every exec of exactly `command` with `script`.
    pub fn script(&self, command: &str, script: Script) {
        self.lock().scripts.insert(command.to_owned(), script);
    }

    /// The machine ids connected so far, in order, refused ones included.
    #[must_use]
    pub fn connects(&self) -> Vec<String> {
        self.lock().connects.clone()
    }

    /// The machine ids whose connections were closed, in order.
    #[must_use]
    pub fn closed(&self) -> Vec<String> {
        self.lock().closed.clone()
    }

    /// Every exec run so far, in order.
    #[must_use]
    pub fn execs(&self) -> Vec<Exec> {
        self.lock().execs.clone()
    }

    /// The unscripted channels opened since the last call, oldest first.
    #[must_use]
    pub fn take_opened(&self) -> Vec<Opened> {
        self.lock().opened.drain(..).collect()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Transport for FakeTransport {
    type Connection = FakeConnection;

    fn connect(&self, machine: &Machine) -> impl Future<Output = Result<FakeConnection, TransportError>> {
        let mut state = self.lock();
        state.connects.push(machine.id.clone());
        let connected = match state.refusals.get(&machine.host) {
            Some(reason) => Err(TransportError::Unreachable(reason.clone())),
            None => Ok(FakeConnection {
                machine_id: machine.id.clone(),
                state: Arc::clone(&self.state),
                closed: Cell::new(false),
            }),
        };
        std::future::ready(connected)
    }
}

/// A fake machine's connection.
#[derive(Debug)]
pub struct FakeConnection {
    machine_id: String,
    state: Arc<Mutex<State>>,
    closed: Cell<bool>,
}

impl Connection for FakeConnection {
    async fn exec(&self, command: &str, pty: Option<GridSize>) -> Result<Channel, TransportError> {
        if self.closed.get() {
            return Err(TransportError::Closed);
        }
        let script = {
            let mut state = lock(&self.state);
            state.execs.push(Exec {
                machine_id: self.machine_id.clone(),
                command: command.to_owned(),
                pty,
            });
            state.scripts.get(command).cloned()
        };
        let (channel, peer) = Channel::pair();
        match script {
            // A script is three chunks at most, within the channel's depth: delivered before the reader exists.
            Some(script) => {
                if !script.stdout.is_empty() {
                    peer.data(script.stdout).await;
                }
                if !script.stderr.is_empty() {
                    peer.stderr(script.stderr).await;
                }
                peer.end(script.end).await;
            }
            None => lock(&self.state).opened.push_back(Opened {
                machine_id: self.machine_id.clone(),
                command: command.to_owned(),
                pty,
                peer,
            }),
        }
        Ok(channel)
    }

    fn close(&self) {
        if !self.closed.replace(true) {
            lock(&self.state).closed.push(self.machine_id.clone());
        }
    }
}
