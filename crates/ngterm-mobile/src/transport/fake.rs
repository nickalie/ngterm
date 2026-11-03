//! The scripted in-memory transport: a machine is refused or connects, at once or once the test releases it
//! ([`FakeTransport::hold`]), a command with a script answers it and ends (a sequence of scripts answers its runs in
//! turn, [`FakeTransport::script_sequence`]), and any other command's channel is handed to the test
//! ([`FakeTransport::take_opened`]) to drive by hand. [`FakeTransport::drop_connections`] loses a machine's
//! connections as a network gone away would.
//!
//! The state is shared behind a mutex, so a test keeps a clone of the [`FakeTransport`] it moved onto the actor and
//! inspects what the core did through it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ngterm_term::GridSize;
use tokio::sync::{mpsc, oneshot, watch};

use super::russh::CONNECTION_LOST;
use super::{Channel, ChannelEnd, ChannelOutput, ChannelPeer, Connection, Transport, TransportError};
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
    refusals: HashMap<String, TransportError>,
    held: HashSet<String>,
    /// The connects waiting on a held host, by host.
    gates: Vec<(String, oneshot::Sender<()>)>,
    /// Each command's answers in turn; the last one answers every later run.
    scripts: HashMap<String, VecDeque<Script>>,
    connects: Vec<String>,
    /// The key each connect signed in with, in order.
    keys: Vec<String>,
    closed: Vec<String>,
    execs: Vec<Exec>,
    opened: VecDeque<Opened>,
    /// Every connection made, by its number, until it is closed or lost.
    links: HashMap<u64, Link>,
    last_link: u64,
}

/// One connection's shared part: what closing or losing it ends.
#[derive(Debug)]
struct Link {
    machine_id: String,
    /// Set when it is closed or lost, which resolves [`Connection::lost`].
    gone: watch::Sender<bool>,
    /// The channels opened with no script, held weakly so a peer the test drops still reads as gone.
    channels: Vec<mpsc::WeakSender<ChannelOutput>>,
}

impl State {
    /// Ends connection `id`, if still up: its channels end as lost for `reason`, and later execs fail.
    fn end_link(&mut self, id: u64, reason: &str) -> bool {
        let Some(link) = self.links.remove(&id) else {
            return false;
        };
        link.gone.send_replace(true);
        // A channel whose reader is a full queue behind keeps its place and misses the end: ending can't wait.
        for channel in link.channels {
            if let Some(output) = channel.upgrade() {
                let _ = output.try_send(ChannelOutput::End(ChannelEnd::Lost(reason.to_owned())));
            }
        }
        true
    }
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

    /// Fails every connect to `host` as [`TransportError::Unreachable`] with `reason`.
    pub fn refuse(&self, host: &str, reason: &str) {
        self.lock()
            .refusals
            .insert(host.to_owned(), TransportError::Unreachable(reason.to_owned()));
    }

    /// Fails every connect to `host` as [`TransportError::Refused`] with `reason`, as a changed host key does.
    pub fn refuse_for_good(&self, host: &str, reason: &str) {
        self.lock()
            .refusals
            .insert(host.to_owned(), TransportError::Refused(reason.to_owned()));
    }

    /// Accepts connects to `host` again.
    pub fn accept(&self, host: &str) {
        self.lock().refusals.remove(host);
    }

    /// Keeps connects to `host` pending until [`FakeTransport::release`].
    pub fn hold(&self, host: &str) {
        self.lock().held.insert(host.to_owned());
    }

    /// Lets the connects to `host` held so far finish, and later ones finish at once.
    pub fn release(&self, host: &str) {
        let mut state = self.lock();
        state.held.remove(host);
        let (released, held) = std::mem::take(&mut state.gates)
            .into_iter()
            .partition(|(gate, _)| gate == host);
        state.gates = held;
        drop(state);
        for (_, gate) in released {
            // A connect dropped while held has nothing to finish.
            let _ = gate.send(());
        }
    }

    /// Answers every exec of exactly `command` with `script`.
    pub fn script(&self, command: &str, script: Script) {
        self.script_sequence(command, vec![script]);
    }

    /// Answers the runs of exactly `command` with `scripts` in turn, the last one answering every later run; an empty
    /// sequence removes the command's script.
    pub fn script_sequence(&self, command: &str, scripts: Vec<Script>) {
        let mut state = self.lock();
        if scripts.is_empty() {
            state.scripts.remove(command);
        } else {
            state.scripts.insert(command.to_owned(), scripts.into());
        }
    }

    /// The machine ids connected so far, in order, refused and held ones included.
    #[must_use]
    pub fn connects(&self) -> Vec<String> {
        self.lock().connects.clone()
    }

    /// The key ids the connects so far signed in with, in order, refused and held ones included.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.lock().keys.clone()
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

    /// Loses every connection of `machine_id` still up: their channels end as lost ([`CONNECTION_LOST`]) and their
    /// [`Connection::lost`] resolves, as when the network goes away.
    pub fn drop_connections(&self, machine_id: &str) {
        let mut state = self.lock();
        let ids: Vec<u64> = state
            .links
            .iter()
            .filter(|(_, link)| link.machine_id == machine_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            state.end_link(id, CONNECTION_LOST);
        }
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
        state.keys.push(machine.key_id.clone());
        let connected = if let Some(refusal) = state.refusals.get(&machine.host) {
            Err(refusal.clone())
        } else {
            state.last_link += 1;
            let id = state.last_link;
            let (gone, lost) = watch::channel(false);
            let link = Link {
                machine_id: machine.id.clone(),
                gone,
                channels: Vec::new(),
            };
            state.links.insert(id, link);
            Ok(FakeConnection {
                id,
                machine_id: machine.id.clone(),
                state: Arc::clone(&self.state),
                lost,
            })
        };
        let gate = state.held.contains(&machine.host).then(|| {
            let (open, gate) = oneshot::channel();
            state.gates.push((machine.host.clone(), open));
            gate
        });
        async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            connected
        }
    }
}

/// A fake machine's connection.
#[derive(Debug)]
pub struct FakeConnection {
    /// Its number among the transport's connections.
    id: u64,
    machine_id: String,
    state: Arc<Mutex<State>>,
    lost: watch::Receiver<bool>,
}

impl Connection for FakeConnection {
    async fn exec(&self, command: &str, pty: Option<GridSize>) -> Result<Channel, TransportError> {
        let script = {
            let mut state = lock(&self.state);
            if !state.links.contains_key(&self.id) {
                return Err(TransportError::Closed);
            }
            state.execs.push(Exec {
                machine_id: self.machine_id.clone(),
                command: command.to_owned(),
                pty,
            });
            state.scripts.get_mut(command).and_then(|scripts| {
                if scripts.len() > 1 {
                    scripts.pop_front()
                } else {
                    scripts.front().cloned()
                }
            })
        };
        let (channel, peer) = Channel::pair();
        let Some(script) = script else {
            let mut state = lock(&self.state);
            if let Some(link) = state.links.get_mut(&self.id) {
                link.channels.push(peer.output.downgrade());
            }
            state.opened.push_back(Opened {
                machine_id: self.machine_id.clone(),
                command: command.to_owned(),
                pty,
                peer,
            });
            return Ok(channel);
        };
        // A script is three chunks at most, within the channel's depth: delivered before the reader exists.
        if !script.stdout.is_empty() {
            peer.data(script.stdout).await;
        }
        if !script.stderr.is_empty() {
            peer.stderr(script.stderr).await;
        }
        peer.end(script.end).await;
        Ok(channel)
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        if state.end_link(self.id, &TransportError::Closed.to_string()) {
            state.closed.push(self.machine_id.clone());
        }
    }

    async fn lost(&self) {
        // The sender goes with the connection's link: set when closed or lost, dropped when this connection is.
        let _ = self.lost.clone().wait_for(|gone| *gone).await;
    }
}

impl Drop for FakeConnection {
    fn drop(&mut self) {
        lock(&self.state).links.remove(&self.id);
    }
}
