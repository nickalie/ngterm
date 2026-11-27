//! A remote window's structural actions on their way to its server (upstream's `RemoteCatalogActions`): each one goes
//! under an operation id and stays marked until its receipt settles it. Applied settles once the catalog reaches the
//! receipt's revision; no change, a refusal or a fence settles at once. A reply this machine never got is settled only
//! by `catalog op`, never by what a snapshot shows, and is never sent again; one the server itself cannot answer stays
//! unknown until the user dismisses it or its row goes.
//!
//! Event-driven, as the rest of the remote window is: the ssh runs go off the main thread, and
//! [`RemoteActions::poll`] takes their answers on the remote tick.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ngterm_proto::{CatalogReceipt, ControlRemoteOperation, ControlResult, Response};
use serde_json::Value;
use uuid::Uuid;

use super::operation::{RemoteOperation, RemoteOperationTarget};
use super::stream::{CatalogState, CatalogStream};
use crate::catalog::frames::CatalogSnapshot;
use crate::catalog::operations::{CatalogPosition, outcome};
use crate::pane_env::uuid_string;
use crate::remote::origin_replies::COMPANION_DEADLINE;
use crate::remote::peer::RemotePeer;
use crate::remote::ssh;
use crate::remote::tree::RemoteCommandRunner;
use crate::remote::viewer::{Answer, run_off_thread, take_answer};

/// What the window's connection offers an action: its server, whether the stream is up, and the catalog it shows.
#[derive(Clone, Copy)]
pub(crate) struct Link<'a> {
    pub host: &'a str,
    pub connected: bool,
    pub catalog: Option<&'a CatalogSnapshot>,
}

/// What `stream` offers an action.
pub(crate) fn link(stream: &CatalogStream) -> Link<'_> {
    Link {
        host: stream.host(),
        connected: *stream.state() == CatalogState::Connected,
        catalog: stream.catalog(),
    }
}

/// An operation with what it was decided on: the catalog position and, for a pane action, the session's panes as the
/// window showed them, so a change that arrives before it is sent cannot retarget it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub operation: RemoteOperation,
    sent: CatalogPosition,
    panes: Option<Vec<Uuid>>,
    /// The session's scratch identity, or `none`, when the operation replaces it.
    scratch: Option<String>,
    /// The window's selected row when the operation was asked for.
    pub selection: Option<Uuid>,
}

/// Who asked for an action, which decides where its answer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asker {
    /// A control request, which waits for the reply.
    Control,
    /// The window's own controls, which report a refusal over the row.
    Gui,
}

/// A send whose answer came.
#[derive(Debug)]
pub(crate) struct Finished {
    pub op_id: String,
    pub operation: RemoteOperation,
    pub asker: Asker,
    pub response: Response,
    /// The row the send created, to select once the window shows it unless the selection moved from the baseline.
    pub created: Option<(Uuid, Option<Uuid>)>,
}

/// An operation the window shows on its row: still pending, or with an outcome this machine could not learn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOperationMark {
    pub op_id: String,
    pub label: &'static str,
    pub target: Option<RemoteOperationTarget>,
    pub unknown: bool,
}

impl From<&RemoteOperationMark> for ControlRemoteOperation {
    fn from(mark: &RemoteOperationMark) -> Self {
        let (session, workspace) = match mark.target {
            Some(RemoteOperationTarget::Session(id)) => (Some(uuid_string(id)), None),
            Some(RemoteOperationTarget::Workspace(id)) => (None, Some(uuid_string(id))),
            None => (None, None),
        };
        Self {
            id: mark.op_id.clone(),
            action: mark.label.to_owned(),
            session,
            workspace,
            state: if mark.unknown { "unknown" } else { "pending" }.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Sending,
    Awaiting(CatalogPosition),
    Lookup,
    Unknown,
}

#[derive(Debug)]
struct Entry {
    operation: RemoteOperation,
    sent: CatalogPosition,
    order: u64,
    state: State,
}

impl Entry {
    const fn holds_repeat(&self) -> bool {
        matches!(self.state, State::Sending | State::Awaiting(_))
    }

    const fn unknown(&self) -> bool {
        matches!(self.state, State::Lookup | State::Unknown)
    }
}

/// Where the lookups of lost replies stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Batch {
    #[default]
    Idle,
    Running,
    /// A snapshot arrived while the batch ran: ask about every lost reply again.
    RunningAgain,
}

struct Outgoing {
    op_id: String,
    answer: Answer,
    asker: Asker,
    select: bool,
    selection: Option<Uuid>,
}

/// One remote window's actions.
#[derive(Default)]
pub(crate) struct RemoteActions {
    entries: HashMap<String, Entry>,
    sends: Vec<Outgoing>,
    lookup: Option<(String, Answer)>,
    /// The lost replies this batch of lookups already asked about.
    asked: HashSet<String>,
    batch: Batch,
    next_order: u64,
    stopped: bool,
    changed: bool,
    replies: HashMap<String, Response>,
}

impl std::fmt::Debug for RemoteActions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActions")
            .field("entries", &self.entries)
            .field("sends", &self.sends.len())
            .finish_non_exhaustive()
    }
}

/// The answer of an action whose reply this machine never got (upstream's string).
pub(crate) fn lost_reply(host: &str, label: &str) -> String {
    format!("{host} did not answer {label}; its outcome is unknown until {host} can be asked")
}

/// The refusal of `command` on a window that is not remote (upstream's `remoteWindowOnly`).
pub(crate) fn remote_window_only(command: &str) -> String {
    format!("{command} acts only on a remote window")
}

/// The refusal while the window's stream is down.
pub(crate) fn not_connected(host: &str) -> String {
    format!("{host} is not connected")
}

impl RemoteActions {
    /// Every operation still pending or unknown, oldest first.
    pub(crate) fn marks(&self) -> Vec<RemoteOperationMark> {
        let mut entries: Vec<(&String, &Entry)> = self.entries.iter().collect();
        entries.sort_by_key(|(_, entry)| entry.order);
        entries
            .into_iter()
            .map(|(op_id, entry)| RemoteOperationMark {
                op_id: op_id.clone(),
                label: entry.operation.label(),
                target: entry.operation.target(),
                unknown: entry.unknown(),
            })
            .collect()
    }

    /// Row `id`'s read-back: `unknown` when one of its operations' outcome was lost, `pending` while one is on its
    /// way, `None` otherwise.
    pub(crate) fn pending_state(&self, id: Uuid) -> Option<&'static str> {
        let target = Some(RemoteOperationTarget::Session(id));
        let mut row = self.entries.values().filter(|entry| entry.operation.target() == target);
        let first = row.next()?;
        if first.unknown() || row.any(Entry::unknown) {
            return Some("unknown");
        }
        Some("pending")
    }

    /// Whether an ssh run is out.
    pub(crate) fn busy(&self) -> bool {
        !self.sends.is_empty() || self.lookup.is_some()
    }

    /// Whether the marks changed since the last call.
    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The reply of control request `op_id`, once it came.
    pub(crate) fn take_reply(&mut self, op_id: &str) -> Option<Response> {
        self.replies.remove(op_id)
    }

    /// Captures what `operation` is decided on, when it was asked for; refused while the window is not connected.
    pub(crate) fn prepare(
        &self,
        link: Link<'_>,
        operation: RemoteOperation,
        selection: Option<Uuid>,
    ) -> Result<Prepared, String> {
        let host = link.host;
        let catalog = link
            .catalog
            .filter(|_| link.connected && !self.stopped)
            .ok_or_else(|| not_connected(host))?;
        let session = match operation.target() {
            Some(RemoteOperationTarget::Session(id)) if operation.pins_panes() || operation.replaces_scratch() => {
                let session = catalog
                    .workspaces
                    .iter()
                    .flat_map(|workspace| &workspace.sessions)
                    .find(|session| session.id() == id)
                    .ok_or_else(|| format!("no such session on {host}"))?;
                Some(session)
            }
            _ => None,
        };
        let panes = match session.filter(|_| operation.pins_panes()) {
            Some(session) if session.panes.is_empty() => return Err(format!("the session has ended on {host}")),
            Some(session) => Some(session.panes.iter().map(|pane| pane.identity).collect()),
            None => None,
        };
        let scratch = session.filter(|_| operation.replaces_scratch()).map(|session| {
            session
                .fields
                .scratch
                .as_ref()
                .map_or_else(|| "none".to_owned(), |scratch| uuid_string(scratch.pane.identity))
        });
        Ok(Prepared {
            operation,
            sent: CatalogPosition::new(&catalog.epoch, catalog.rev),
            panes,
            scratch,
            selection,
        })
    }

    /// Runs `prepared` on the server: its operation id, or the refusal when nothing was sent, the window being down or
    /// the same action on the same row still pending. `select` selects the row it creates once the window shows it.
    pub(crate) fn send(
        &mut self,
        link: Link<'_>,
        runner: &Arc<dyn RemoteCommandRunner>,
        prepared: Prepared,
        asker: Asker,
        select: bool,
    ) -> Result<String, String> {
        let operation = prepared.operation;
        if self.stopped || !link.connected {
            return Err(not_connected(link.host));
        }
        let key = operation.key();
        if self
            .entries
            .values()
            .any(|entry| entry.operation.key() == key && entry.holds_repeat())
        {
            return Err(format!("{} is still pending on {}", operation.label(), link.host));
        }
        let mut words = operation.words();
        if let Some(panes) = &prepared.panes {
            let panes: Vec<String> = panes.iter().copied().map(uuid_string).collect();
            words.extend(["--expect-panes".to_owned(), panes.join(",")]);
        }
        if let Some(scratch) = &prepared.scratch {
            words.extend(["--expect-scratch".to_owned(), scratch.clone()]);
        }
        let op_id = uuid_string(Uuid::new_v4());
        words.extend([
            "--op-id".to_owned(),
            op_id.clone(),
            "--epoch".to_owned(),
            prepared.sent.epoch.clone(),
        ]);
        let answer = command(link.host, &words, &operation.operands())
            .and_then(|argv| run_off_thread(runner, argv, COMPANION_DEADLINE))
            .ok_or_else(|| format!("no connection to {}", link.host))?;
        self.next_order += 1;
        self.entries.insert(
            op_id.clone(),
            Entry {
                operation,
                sent: prepared.sent,
                order: self.next_order,
                state: State::Sending,
            },
        );
        self.changed = true;
        self.sends.push(Outgoing {
            op_id: op_id.clone(),
            answer,
            asker,
            select,
            selection: prepared.selection,
        });
        Ok(op_id)
    }

    /// Takes the answers that came: each send settles and is handed back, and each lookup settles its operation.
    pub(crate) fn poll(&mut self, link: Link<'_>, runner: &Arc<dyn RemoteCommandRunner>) -> Vec<Finished> {
        let mut finished = Vec::new();
        let sends = std::mem::take(&mut self.sends);
        for send in sends {
            match take_answer(&send.answer) {
                Some(result) => finished.extend(self.answered(link, runner, send, &result.stdout)),
                None => self.sends.push(send),
            }
        }
        if let Some((op_id, answer)) = self.lookup.take() {
            match take_answer(&answer) {
                Some(result) => {
                    self.looked_up(link, &op_id, &result.stdout);
                    self.pump(link, runner);
                }
                None => self.lookup = Some((op_id, answer)),
            }
        }
        finished
    }

    fn answered(
        &mut self,
        link: Link<'_>,
        runner: &Arc<dyn RemoteCommandRunner>,
        send: Outgoing,
        stdout: &str,
    ) -> Option<Finished> {
        let operation = self.entries.get(&send.op_id)?.operation.clone();
        let reply: Option<Response> = serde_json::from_str(stdout.trim()).ok();
        let typed = reply.as_ref().and_then(typed_result);
        let receipt = typed
            .as_ref()
            .and_then(|result| result.receipt.clone())
            .filter(|receipt| receipt.op_id == send.op_id);
        let (response, created) = if let (Some(reply), Some(receipt)) = (reply, receipt) {
            self.settle(link, runner, &send.op_id, Some(&receipt));
            let created = typed
                .and_then(|result| result.id)
                .and_then(|id| Uuid::parse_str(&id).ok())
                .filter(|_| send.select && reply.ok)
                .map(|id| (id, send.selection));
            (reply, created)
        } else {
            self.settle(link, runner, &send.op_id, None);
            (Response::failure(lost_reply(link.host, operation.label())), None)
        };
        if send.asker == Asker::Control {
            self.replies.insert(send.op_id.clone(), response.clone());
        }
        Some(Finished {
            op_id: send.op_id,
            operation,
            asker: send.asker,
            response,
            created,
        })
    }

    fn looked_up(&mut self, link: Link<'_>, op_id: &str, stdout: &str) {
        let receipt = serde_json::from_str::<Response>(stdout.trim())
            .ok()
            .as_ref()
            .and_then(typed_result)
            .and_then(|result| result.receipt)
            .filter(|receipt| receipt.op_id == op_id);
        let looking = self
            .entries
            .get(op_id)
            .is_some_and(|entry| entry.state == State::Lookup);
        if let Some(receipt) = receipt.filter(|_| looking) {
            self.settle_receipt(link, op_id, &receipt);
        }
    }

    /// Drops every mark whose outcome this machine could not learn; answers how many.
    pub(crate) fn dismiss_unknown(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, entry| !entry.unknown());
        let dismissed = before - self.entries.len();
        self.changed |= dismissed > 0;
        dismissed
    }

    /// Ends every lookup: the window is closing.
    pub(crate) fn stop(&mut self) {
        self.stopped = true;
        self.lookup = None;
        self.batch = Batch::Idle;
    }

    /// The catalog moved: settles what its new revision answers. A new snapshot also asks about every lost reply, and
    /// sends a receipt of an epoch the catalog is not in to a lookup.
    pub(crate) fn catalog_moved(&mut self, link: Link<'_>, runner: &Arc<dyn RemoteCommandRunner>, snapshot: bool) {
        let Some(catalog) = link.catalog else {
            return;
        };
        let mut gone = Vec::new();
        for (op_id, entry) in &mut self.entries {
            match &entry.state {
                State::Awaiting(target) if target.epoch == catalog.epoch => {
                    if catalog.rev >= target.rev {
                        gone.push(op_id.clone());
                    }
                }
                State::Awaiting(_) => {
                    if snapshot {
                        entry.state = State::Lookup;
                        self.changed = true;
                    }
                }
                State::Lookup | State::Unknown => {
                    if entry
                        .operation
                        .target()
                        .is_some_and(|target| !contains(catalog, target))
                    {
                        gone.push(op_id.clone());
                    }
                }
                State::Sending => {}
            }
        }
        for op_id in gone {
            self.entries.remove(&op_id);
            self.changed = true;
        }
        if snapshot {
            self.start_lookups(link, runner, true);
        }
    }

    fn settle(
        &mut self,
        link: Link<'_>,
        runner: &Arc<dyn RemoteCommandRunner>,
        op_id: &str,
        receipt: Option<&CatalogReceipt>,
    ) {
        if self.settle_receipt(link, op_id, receipt.unwrap_or(&lost())) {
            self.start_lookups(link, runner, false);
        }
    }

    /// Settles `op_id` by `receipt`; true when its reply turned out lost and a lookup must ask about it.
    fn settle_receipt(&mut self, link: Link<'_>, op_id: &str, receipt: &CatalogReceipt) -> bool {
        let Some(entry) = self.entries.get_mut(op_id) else {
            return false;
        };
        self.changed = true;
        let state = match receipt.outcome.as_str() {
            outcome::NOOP | outcome::REFUSED | outcome::FENCED => None,
            outcome::UNKNOWN => Some(State::Unknown),
            outcome::APPLIED => match (receipt.rev, link.catalog) {
                // a marker answered before its command finished carries no revision: ask again later
                (None, _) => Some(State::Lookup),
                (Some(_), None) => return false,
                (Some(rev), Some(catalog)) => {
                    let target = CatalogPosition::new(&receipt.epoch, rev);
                    if catalog.epoch == target.epoch {
                        (catalog.rev < rev).then_some(State::Awaiting(target))
                    } else if target.epoch == entry.sent.epoch {
                        // the catalog already left the epoch this receipt belongs to, and that epoch never returns
                        Some(State::Lookup)
                    } else {
                        Some(State::Awaiting(target))
                    }
                }
            },
            _ => Some(State::Lookup),
        };
        match state {
            None => {
                self.entries.remove(op_id);
                false
            }
            Some(state) => {
                let lookup = state == State::Lookup;
                entry.state = state;
                lookup
            }
        }
    }

    /// Asks about every lost reply once per batch. A reply lost while a batch runs joins it; a `round` (a new snapshot)
    /// asks about every one again, so a lookup that failed is asked again only then.
    fn start_lookups(&mut self, link: Link<'_>, runner: &Arc<dyn RemoteCommandRunner>, round: bool) {
        if self.stopped || !link.connected || !self.entries.values().any(|entry| entry.state == State::Lookup) {
            return;
        }
        if self.batch != Batch::Idle {
            if round {
                self.batch = Batch::RunningAgain;
            }
            return;
        }
        self.batch = Batch::Running;
        self.asked.clear();
        self.pump(link, runner);
    }

    /// Starts the batch's next lookup, the oldest lost reply it has not asked about.
    fn pump(&mut self, link: Link<'_>, runner: &Arc<dyn RemoteCommandRunner>) {
        while self.batch != Batch::Idle && self.lookup.is_none() {
            if self.stopped || !link.connected {
                self.batch = Batch::Idle;
                return;
            }
            if self.batch == Batch::RunningAgain {
                self.batch = Batch::Running;
                self.asked.clear();
            }
            let next = self
                .entries
                .iter()
                .filter(|(op_id, entry)| entry.state == State::Lookup && !self.asked.contains(*op_id))
                .min_by_key(|(_, entry)| entry.order)
                .map(|(op_id, entry)| (op_id.clone(), entry.sent.clone()));
            let Some((op_id, sent)) = next else {
                self.batch = Batch::Idle;
                return;
            };
            self.asked.insert(op_id.clone());
            let rev = sent.rev.to_string();
            let words = [
                "catalog",
                "op",
                op_id.as_str(),
                "--epoch",
                sent.epoch.as_str(),
                "--rev",
                rev.as_str(),
            ];
            let answer = command(link.host, &words, &[] as &[String])
                .and_then(|argv| run_off_thread(runner, argv, COMPANION_DEADLINE));
            if let Some(answer) = answer {
                self.lookup = Some((op_id, answer));
            }
        }
    }
}

/// The receipt a lost reply settles by: the lookup is what tells.
fn lost() -> CatalogReceipt {
    CatalogReceipt {
        op_id: String::new(),
        epoch: String::new(),
        outcome: "lost".to_owned(),
        rev: None,
        error: None,
    }
}

fn typed_result(reply: &Response) -> Option<ControlResult> {
    reply
        .result
        .clone()
        .and_then(|result: Value| serde_json::from_value(result).ok())
}

/// The ssh line that runs `ngtermctl WORDS -- OPERANDS` on `host`'s `ngterm-headless`.
fn command<S: AsRef<str>, O: AsRef<str>>(host: &str, words: &[S], operands: &[O]) -> Option<Vec<String>> {
    ssh::control_command_with_operands(host, RemotePeer::Ngterm, words, operands).ok()
}

fn contains(catalog: &CatalogSnapshot, target: RemoteOperationTarget) -> bool {
    match target {
        RemoteOperationTarget::Session(id) => catalog
            .workspaces
            .iter()
            .any(|workspace| workspace.sessions.iter().any(|session| session.id() == id)),
        RemoteOperationTarget::Workspace(id) => catalog.workspaces.iter().any(|workspace| workspace.id == id),
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
