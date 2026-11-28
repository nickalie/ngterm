//! A remote window's shared ssh connections (upstream's `SshConnectionPool.swift`): its ssh channels, panes, streams
//! and one-shot commands, ride `ControlMaster` connections, at most [`CHANNELS_PER_CONNECTION`] per master, under stock
//! sshd's `MaxSessions 10`. Each channel holds a lease for its lifetime, and a slot's master starts once, before its
//! users connect, so a reconnect costs one handshake per master, not one per channel.
//!
//! [`SshConnectionPool::ready`] blocks, so it runs on a thread of its own: a lease starts it in the background, and a
//! one-shot command waits for it before its ssh runs. Callers that arrive during a start share that one start.
//!
//! Unix only: Windows' OpenSSH has no connection sharing, so a Windows viewer's windows run without a pool.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use uuid::Uuid;

use crate::command_restore::shell_quoted_line;
use crate::pane_env::uuid_string;
use crate::remote::client::{LinkKey, PresentationLink, PresentationTransport};
use crate::remote::ssh;
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner, validate};
use crate::remote::viewer::{Answer, run_off_thread};
use crate::remote::viewer_run::{KEEP_ALIVE, NO_KEEP_ALIVE};

/// How many channels one master carries.
pub const CHANNELS_PER_CONNECTION: usize = 8;

/// How long a master's check, start or exit may take (upstream's runner deadline).
const COMMAND_DEADLINE: Duration = Duration::from_secs(15);

/// One remote window's pool. Clones share it.
#[derive(Clone)]
pub struct SshConnectionPool {
    shared: Arc<Shared>,
}

struct Shared {
    host: String,
    prefix: String,
    runner: Arc<dyn RemoteCommandRunner>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    slots: Vec<Slot>,
    closed: bool,
    background: Vec<JoinHandle<()>>,
}

struct Slot {
    path: String,
    users: HashSet<String>,
    preparing: Option<Arc<Start>>,
}

/// One master's check-then-start, which every caller arriving meanwhile waits on.
#[derive(Default)]
struct Start {
    done: Mutex<Option<bool>>,
    finished: Condvar,
}

impl Start {
    fn wait(&self) -> bool {
        let mut done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(started) = *done {
                return started;
            }
            done = self.finished.wait(done).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn finish(&self, started: bool) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) = Some(started);
        self.finished.notify_all();
    }
}

impl std::fmt::Debug for SshConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnectionPool")
            .field("host", &self.shared.host)
            .field("load", &self.load())
            .finish_non_exhaustive()
    }
}

impl SshConnectionPool {
    /// A pool for `host` whose master sockets live under `root`; `key` names its owner, so two windows or two state
    /// directories never share a master.
    #[must_use]
    pub fn new(host: &str, root: &Path, key: &str, runner: Arc<dyn RemoteCommandRunner>) -> Self {
        Self {
            shared: Arc::new(Shared {
                host: host.to_owned(),
                prefix: format!("{}/{}", root.display(), hash(key)),
                runner,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// The host every master connects to.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.shared.host
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The control path `user` rides: the slot it already holds, else the first with room, else a new one. Each lease
    /// also makes sure that master is up, in the background, as a pane's command cannot wait for it, and a reattach
    /// re-leases the slot whose master may have died.
    #[expect(
        clippy::must_use_candidate,
        reason = "a lease is also taken for its side effect alone, the master's start (upstream's @discardableResult)"
    )]
    pub fn lease(&self, user: &str) -> String {
        let mut state = self.state();
        let index = state
            .slots
            .iter()
            .position(|slot| slot.users.contains(user))
            .or_else(|| {
                state
                    .slots
                    .iter()
                    .position(|slot| slot.users.len() < CHANNELS_PER_CONNECTION)
            })
            .unwrap_or_else(|| {
                let path = format!("{}-{}", self.shared.prefix, state.slots.len());
                state.slots.push(Slot {
                    path,
                    users: HashSet::new(),
                    preparing: None,
                });
                state.slots.len() - 1
            });
        let slot = &mut state.slots[index];
        slot.users.insert(user.to_owned());
        let path = slot.path.clone();
        if !state.closed {
            let pool = self.clone();
            let ready_path = path.clone();
            let spawned = std::thread::Builder::new()
                .name("ngterm.ssh-pool".to_owned())
                .spawn(move || {
                    let _started = pool.ready(&ready_path);
                });
            if let Ok(handle) = spawned {
                state.background.retain(|handle| !handle.is_finished());
                state.background.push(handle);
            }
        }
        path
    }

    /// A channel for one command: the caller releases the returned user when it exits.
    #[must_use]
    pub fn lease_once(&self) -> (String, String) {
        let user = format!("once:{}", Uuid::new_v4());
        let path = self.lease(&user);
        (user, path)
    }

    pub fn release(&self, user: &str) {
        for slot in &mut self.state().slots {
            slot.users.remove(user);
        }
    }

    /// Keeps the users starting with `kind` that are in `users` and releases the rest of that kind.
    pub fn retain(&self, users: &HashSet<String>, kind: &str) {
        for slot in &mut self.state().slots {
            slot.users
                .retain(|user| !user.starts_with(kind) || users.contains(user));
        }
    }

    /// Waits until `path`'s master answers, starting it when it does not; callers arriving meanwhile share that one
    /// start. False when there is no master, and the caller then fails its attempt rather than connect on its own.
    #[must_use]
    pub fn ready(&self, path: &str) -> bool {
        let start = {
            let mut state = self.state();
            if state.closed || validate(&self.shared.host).is_err() {
                return false;
            }
            let Some(slot) = state.slots.iter_mut().find(|slot| slot.path == path) else {
                return false;
            };
            if let Some(start) = &slot.preparing {
                let start = Arc::clone(start);
                drop(state);
                return start.wait();
            }
            let start = Arc::new(Start::default());
            slot.preparing = Some(Arc::clone(&start));
            start
        };
        let started = self.check_or_start(path);
        start.finish(started);
        let mut state = self.state();
        if let Some(slot) = state.slots.iter_mut().find(|slot| slot.path == path)
            && slot
                .preparing
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &start))
        {
            slot.preparing = None;
        }
        started
    }

    fn check_or_start(&self, path: &str) -> bool {
        let host = &self.shared.host;
        let run = |argv: Vec<String>| self.shared.runner.run(&argv, COMMAND_DEADLINE).status == 0;
        if run(check_command(host, path)) {
            return !self.state().closed;
        }
        if self.state().closed {
            return false;
        }
        let started = run(start_command(host, path));
        if self.state().closed {
            // the pool closed while this master was starting; its exit command already ran and missed it
            if started {
                run(exit_command(host, path));
            }
            return false;
        }
        started
    }

    /// Drops every lease and returns the commands that end the pool's masters.
    #[must_use]
    pub fn close(&self) -> Vec<Vec<String>> {
        let mut state = self.state();
        state.closed = true;
        let commands = state
            .slots
            .iter()
            .map(|slot| exit_command(&self.shared.host, &slot.path))
            .collect();
        state.slots.clear();
        commands
    }

    /// Each slot's lease count, in slot order.
    #[must_use]
    pub fn load(&self) -> Vec<usize> {
        self.state().slots.iter().map(|slot| slot.users.len()).collect()
    }

    /// Waits for the master checks the leases started.
    pub fn settle(&self) {
        let handles = std::mem::take(&mut self.state().background);
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// The pool user a pane's attach holds.
pub(crate) fn pane_user(identity: Uuid) -> String {
    format!("pane:{}", uuid_string(identity))
}

/// The pool user a row's presentation stream holds.
pub(crate) fn present_user(session: Uuid) -> String {
    format!("present:{}", uuid_string(session))
}

/// A transport whose streams ride `user`'s channel of `pool` (upstream's `PooledTransport`): each open leases it again,
/// which brings back a master that died, and the stream's ssh runs once that master answers. A master that cannot be
/// started ends the stream unopened, and the stream's own retry asks again.
pub(crate) struct PooledTransport<'a> {
    pub(crate) pool: &'a SshConnectionPool,
    pub(crate) user: String,
    pub(crate) inner: &'a dyn PresentationTransport,
}

impl PresentationTransport for PooledTransport<'_> {
    fn open(&self, argv: &[String], key: LinkKey) -> Box<dyn PresentationLink> {
        let path = self.pool.lease(&self.user);
        let argv = ssh::waiting_for_connection(&ssh::pooled(argv.to_vec(), &path), self.pool.host(), &path);
        self.inner.open(&argv, key)
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

/// The status of a pooled command that was never sent: its master did not come up. No process exits with it.
pub(crate) const NOT_SENT: i32 = i32::MIN;

/// Runs `argv`, an ssh invocation built on [`ssh_arguments`](crate::remote::tree::ssh_arguments), through `runner` on a
/// thread of its own; the slot fills when it is done. With `pool`, it rides a channel of its own once the masters at
/// `first` and its own answer, and without them it is never sent ([`NOT_SENT`]). `None` when no thread starts.
pub(crate) fn run_pooled_off_thread(
    pool: Option<&SshConnectionPool>,
    first: &[String],
    runner: &Arc<dyn RemoteCommandRunner>,
    argv: Vec<String>,
    deadline: Duration,
) -> Option<Answer> {
    let Some(pool) = pool else {
        return run_off_thread(runner, argv, deadline);
    };
    let (user, path) = pool.lease_once();
    let answer: Answer = Arc::new(Mutex::new(None));
    let (pool, runner, slot, first) = (pool.clone(), Arc::clone(runner), Arc::clone(&answer), first.to_vec());
    let spawned = std::thread::Builder::new()
        .name("ngterm.remote-command".to_owned())
        .spawn(move || {
            let ready = first.iter().all(|path| pool.ready(path)) && pool.ready(&path);
            let result = if ready {
                runner.run(&ssh::pooled(argv, &path), deadline)
            } else {
                RemoteCommandResult {
                    status: NOT_SENT,
                    stdout: String::new(),
                    stderr: format!("no connection to {}", pool.host()),
                }
            };
            pool.release(&user);
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        });
    spawned.is_ok().then_some(answer)
}

/// `ssh -O check` on the master at `path`.
#[must_use]
pub fn check_command(host: &str, path: &str) -> Vec<String> {
    control_operation("check", host, path)
}

/// `ssh -O exit` on the master at `path`.
#[must_use]
pub fn exit_command(host: &str, path: &str) -> Vec<String> {
    control_operation("exit", host, path)
}

fn control_operation(operation: &str, host: &str, path: &str) -> Vec<String> {
    let destination = crate::remote::peer::ssh_destination(host);
    [
        "ssh",
        "-O",
        operation,
        "-o",
        &format!("ControlPath={path}"),
        destination,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Starts a background master at `path`. Its stdio goes nowhere, or the master would keep the runner's pipes open for
/// as long as it lives. It runs only after a failed check, so a socket at `path` is a dead master's: left there, `ssh
/// -M` finds the path taken and connects without sharing. A channel joining the master keeps the master's keep-alive,
/// never its own, so the master gets the keep-alive when the user's config sets none.
#[must_use]
pub fn start_command(host: &str, path: &str) -> Vec<String> {
    let control_path = format!("ControlPath={path}");
    let connect_timeout = format!("ConnectTimeout={}", crate::remote::connect_timeout::seconds(host));
    let host = crate::remote::peer::ssh_destination(host);
    let options = [
        "-o",
        "BatchMode=yes",
        "-o",
        connect_timeout.as_str(),
        "-o",
        "ControlPersist=60",
        "-o",
        control_path.as_str(),
    ];
    let log = format!("{path}.log");
    let mut master = vec!["ssh", "-f", "-N", "-M", "-E", log.as_str()];
    master.extend(options);
    let ssh = format!("{} $ka {}", shell_quoted_line(&master), shell_quoted_line(&[host]));
    // -N rides into the check: a `Match sessiontype` block answers the master's own session type
    let mut probe = vec!["ssh", "-G", "-N"];
    probe.extend(options);
    probe.push(host);
    let keep_alive = format!(
        "ka=; {} 2>/dev/null | grep -qx {} && ka={}",
        shell_quoted_line(&probe),
        shell_quoted_line(&[NO_KEEP_ALIVE]),
        shell_quoted_line(&[KEEP_ALIVE.join(" ")])
    );
    let script = format!(
        "rm -f {}; {keep_alive}; exec {ssh} </dev/null >/dev/null 2>&1",
        shell_quoted_line(&[path])
    );
    ["/bin/sh", "-c", script.as_str()].map(str::to_owned).to_vec()
}

/// Where the pools' master sockets live (upstream's `sshPoolRoot`): under `/tmp`, as a socket path caps near 104
/// bytes, private to this user; `None` on Windows, whose OpenSSH shares no connections.
#[must_use]
pub fn default_root() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        Some(PathBuf::from(format!("/tmp/ngterm-ssh-{uid}")))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Makes `root`, private to this user; false when it cannot be made.
pub(crate) fn prepare_root(root: &Path) -> bool {
    if std::fs::create_dir_all(root).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).is_ok()
    }
    #[cfg(not(unix))]
    true
}

/// FNV-1a: stable across runs and platforms, and short enough for a socket path.
#[must_use]
pub fn hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
