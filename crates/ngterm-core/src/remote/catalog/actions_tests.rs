//! Upstream's `RemoteCatalogActionsTests`, over a scripted ssh runner and a catalog the test moves.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ngterm_proto::{CatalogReceipt, ControlRemoteOperation, ControlResult, Response};
use uuid::Uuid;

use super::*;
use crate::catalog::fixture::{pane, pane_with, session_with, snapshot, workspace};
use crate::catalog::frames::{CatalogPane, CatalogScratch, CatalogSession};
use crate::remote::tree::RemoteCommandResult;

type Reply = Box<dyn Fn(&str) -> String + Send>;

/// Answers each command and each `catalog op` from its own queue, an empty line once a queue runs out, and records
/// every line. A lookup can be held until the test opens the gate.
#[derive(Default)]
struct Script {
    commands: Mutex<VecDeque<Reply>>,
    lookups: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<String>>,
    gate: Mutex<Option<Receiver<()>>>,
    held: AtomicBool,
}

impl Script {
    fn plain(argv: &[String]) -> String {
        argv.last()
            .map_or_else(String::new, |line| line.replace(['\'', '\\'], ""))
    }

    fn op_id(line: &str) -> String {
        let marker = ["--op-id ", " catalog op "]
            .iter()
            .find_map(|marker| line.find(marker).map(|at| at + marker.len()));
        marker.map_or_else(String::new, |at| line[at..].chars().take(36).collect())
    }

    fn lookup_calls(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap();
        calls
            .iter()
            .filter(|line| line.contains(" catalog op "))
            .cloned()
            .collect()
    }

    fn command_calls(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap();
        calls
            .iter()
            .filter(|line| !line.contains(" catalog op "))
            .cloned()
            .collect()
    }

    /// Holds the next lookup until the sender is dropped or sends.
    fn hold_next_lookup(&self) -> Sender<()> {
        let (open, gate) = channel();
        *self.gate.lock().unwrap() = Some(gate);
        open
    }
}

impl RemoteCommandRunner for Script {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        let line = Self::plain(argv);
        self.calls.lock().unwrap().push(line.clone());
        let op_id = Self::op_id(&line);
        let queue = if line.contains(" catalog op ") {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                self.held.store(true, Ordering::SeqCst);
                let _ = gate.recv();
            }
            &self.lookups
        } else {
            &self.commands
        };
        let reply = queue.lock().unwrap().pop_front();
        RemoteCommandResult {
            status: 0,
            stdout: reply.map_or_else(String::new, |reply| reply(&op_id)),
            stderr: String::new(),
        }
    }
}

fn reply(outcome: &str, epoch: &str, rev: Option<i64>, id: Option<&str>) -> Reply {
    let (outcome, epoch, id) = (outcome.to_owned(), epoch.to_owned(), id.map(str::to_owned));
    Box::new(move |op_id| {
        let refused = outcome == "refused";
        let receipt = CatalogReceipt {
            op_id: op_id.to_owned(),
            epoch: epoch.clone(),
            outcome: outcome.clone(),
            rev,
            error: refused.then(|| "refused here".to_owned()),
        };
        let result = ControlResult {
            receipt: Some(receipt),
            id: id.clone(),
            ..ControlResult::default()
        };
        let response = Response {
            ok: outcome == "applied" || outcome == "noop",
            result: Some(result),
            error: refused.then(|| "refused here".to_owned()),
        };
        serde_json::to_string(&response).unwrap()
    })
}

fn applied(rev: i64) -> Reply {
    reply("applied", "e1", Some(rev), None)
}

fn lost() -> Reply {
    Box::new(|_| String::new())
}

struct Harness {
    script: Arc<Script>,
    runner: Arc<dyn RemoteCommandRunner>,
    actions: RemoteActions,
    catalog: CatalogSnapshot,
    connected: bool,
    workspace: Uuid,
    session: Uuid,
    panes: Vec<CatalogPane>,
}

impl Harness {
    fn new() -> Self {
        let (workspace, session) = (Uuid::new_v4(), Uuid::new_v4());
        let panes = vec![pane(), pane()];
        let mut harness = Self::with_sessions(workspace, vec![session_with(session, "s", panes.clone())]);
        harness.session = session;
        harness.panes = panes;
        harness
    }

    fn with_sessions(workspace: Uuid, sessions: Vec<CatalogSession>) -> Self {
        let script = Arc::new(Script::default());
        Self {
            runner: script.clone(),
            script,
            actions: RemoteActions::default(),
            catalog: snapshot(vec![self::workspace(workspace, "w", sessions)]),
            connected: true,
            workspace,
            session: Uuid::nil(),
            panes: Vec::new(),
        }
    }

    fn link(&self) -> Link<'_> {
        Link {
            host: "box",
            connected: self.connected,
            catalog: Some(&self.catalog),
        }
    }

    fn queue(&self, reply: Reply) {
        self.script.commands.lock().unwrap().push_back(reply);
    }

    fn queue_lookup(&self, reply: Reply) {
        self.script.lookups.lock().unwrap().push_back(reply);
    }

    fn rename(&self) -> RemoteOperation {
        RemoteOperation::RenameSession {
            session: self.session,
            name: "build".to_owned(),
        }
    }

    /// Prepares and sends `operation`; the refusal when either refused.
    fn start(&mut self, operation: RemoteOperation, select: bool) -> Result<String, String> {
        let link = Link {
            host: "box",
            connected: self.connected,
            catalog: Some(&self.catalog),
        };
        let prepared = self.actions.prepare(link, operation, None)?;
        self.actions.send(link, &self.runner, prepared, Asker::Control, select)
    }

    /// Sends `operation` and waits for its reply, the lookups it starts left running.
    fn send(&mut self, operation: RemoteOperation) -> Response {
        match self.start(operation, false) {
            Ok(op_id) => self.reply_of(&op_id),
            Err(error) => Response::failure(error),
        }
    }

    fn reply_of(&mut self, op_id: &str) -> Response {
        for _ in 0..2000 {
            self.poll();
            if let Some(reply) = self.actions.take_reply(op_id) {
                return reply;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no reply for {op_id}");
    }

    fn poll(&mut self) -> Vec<Finished> {
        let link = Link {
            host: "box",
            connected: self.connected,
            catalog: Some(&self.catalog),
        };
        self.actions.poll(link, &self.runner)
    }

    /// Polls until no ssh run is out.
    fn settle_lookups(&mut self) {
        for _ in 0..2000 {
            self.poll();
            if !self.actions.busy() {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the lookups never settled");
    }

    /// The catalog takes revision `rev`.
    fn advance(&mut self, rev: i64) {
        self.catalog.rev = rev;
        self.moved(false);
    }

    /// A whole snapshot at `epoch` and `rev` arrives.
    fn install(&mut self, epoch: &str, rev: i64) {
        self.catalog.epoch = epoch.to_owned();
        self.catalog.rev = rev;
        self.moved(true);
    }

    fn moved(&mut self, snapshot: bool) {
        let link = Link {
            host: "box",
            connected: self.connected,
            catalog: Some(&self.catalog),
        };
        self.actions.catalog_moved(link, &self.runner, snapshot);
    }

    fn unknown_marks(&self) -> Vec<bool> {
        self.actions.marks().iter().map(|mark| mark.unknown).collect()
    }

    fn wait_until_held(&self) {
        for _ in 0..2000 {
            if self.script.held.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no lookup was held");
    }
}

#[test]
fn an_applied_operation_stays_pending_until_the_catalog_reaches_its_revision() {
    let mut h = Harness::new();
    h.queue(applied(2));
    let response = h.send(h.rename());
    let pending = h.actions.marks();
    h.advance(1);
    let after_unrelated = h.actions.marks().len();
    h.advance(2);
    assert!(response.ok, "{response:?}");
    assert_eq!(pending.iter().map(|mark| mark.unknown).collect::<Vec<_>>(), [false]);
    assert_eq!(pending[0].target, Some(RemoteOperationTarget::Session(h.session)));
    assert_eq!(after_unrelated, 1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_delta_that_arrived_before_the_reply_clears_it_at_once() {
    let mut h = Harness::new();
    h.queue(applied(1));
    let op_id = h.start(h.rename(), false).unwrap();
    h.advance(1);
    let _ = h.reply_of(&op_id);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn an_operation_that_changed_nothing_clears_at_once() {
    for outcome in ["noop", "refused", "fenced"] {
        let mut h = Harness::new();
        h.queue(reply(outcome, "e1", None, None));
        let response = h.send(h.rename());
        assert_eq!(response.ok, outcome == "noop", "{outcome}");
        assert!(h.actions.marks().is_empty(), "{outcome}");
    }
}

#[test]
fn the_same_action_on_the_same_row_is_held_while_pending() {
    let mut h = Harness::new();
    h.queue(applied(5));
    h.queue(reply("noop", "e1", None, None));
    let _ = h.send(h.rename());
    let repeated = h.send(h.rename());
    let other = h.send(RemoteOperation::Swap(h.session));
    assert_eq!(
        repeated.error.as_deref(),
        Some("Rename Session is still pending on box")
    );
    assert!(other.ok, "{other:?}");
    assert_eq!(h.script.command_calls().len(), 2);
}

#[test]
fn a_lost_reply_is_settled_by_a_lookup_where_it_was_sent() {
    let mut h = Harness::new();
    h.catalog.rev = 3;
    h.queue(lost());
    h.queue_lookup(applied(3));
    let response = h.send(h.rename());
    let marked = h.unknown_marks();
    h.settle_lookups();
    assert!(!response.ok);
    assert_eq!(
        response.error.as_deref(),
        Some("box did not answer Rename Session; its outcome is unknown until box can be asked")
    );
    assert_eq!(marked, [true]);
    let lookups = h.script.lookup_calls();
    assert_eq!(lookups.len(), 1);
    assert!(lookups[0].contains("--epoch e1 --rev 3 --json"), "{}", lookups[0]);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_reply_lost_while_a_lookup_runs_is_asked_about_in_the_same_run() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue(lost());
    h.queue_lookup(reply("fenced", "e1", None, None));
    h.queue_lookup(reply("fenced", "e1", None, None));
    let open = h.script.hold_next_lookup();
    let _ = h.send(h.rename());
    h.wait_until_held();
    let _ = h.send(RemoteOperation::Swap(h.session));
    open.send(()).unwrap();
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 2);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_snapshot_arriving_during_a_failing_lookup_asks_again_after_it() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(lost());
    h.queue_lookup(reply("fenced", "e1", None, None));
    let open = h.script.hold_next_lookup();
    let _ = h.send(h.rename());
    h.wait_until_held();
    h.install("e1", 0);
    open.send(()).unwrap();
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 2);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_lookup_that_fails_is_not_retried_until_the_next_snapshot() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(lost());
    let _ = h.send(h.rename());
    h.settle_lookups();
    h.advance(1);
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert_eq!(h.unknown_marks(), [true]);
}

#[test]
fn an_unchanged_snapshot_settles_nothing_and_the_lookup_fences_the_delayed_command() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(lost());
    let _ = h.send(h.rename());
    h.settle_lookups();
    h.queue_lookup(reply("fenced", "e1", None, None));
    h.install("e1", 0);
    let after_snapshot = h.unknown_marks();
    h.settle_lookups();
    h.install("e1", 0);
    h.settle_lookups();
    assert_eq!(after_snapshot, [true]);
    assert!(h.actions.marks().is_empty());
    assert_eq!(h.script.lookup_calls().len(), 2);
}

#[test]
fn a_create_that_committed_and_lost_its_reply_is_applied_by_a_lookup_after_a_restart() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(lost());
    let _ = h.send(RemoteOperation::NewWorkspace {
        name: Some("dev".to_owned()),
    });
    h.settle_lookups();
    h.queue_lookup(reply("applied", "e2", Some(4), None));
    h.install("e2", 4);
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 2);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_command_delayed_across_a_restart_is_refused_and_clears() {
    let mut h = Harness::new();
    h.queue(reply("refused", "e1", None, None));
    let response = h.send(h.rename());
    assert_eq!(response.error.as_deref(), Some("refused here"));
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_receipt_from_a_newer_epoch_waits_for_the_catalog_to_reach_it() {
    let mut h = Harness::new();
    h.queue(reply("applied", "e2", Some(4), None));
    let _ = h.send(h.rename());
    let waiting = h.unknown_marks();
    h.install("e2", 4);
    assert_eq!(waiting, [false]);
    assert!(h.actions.marks().is_empty());
    assert!(h.script.lookup_calls().is_empty());
}

#[test]
fn an_old_epoch_reply_after_the_new_epoch_arrived_is_looked_up() {
    let mut h = Harness::new();
    h.queue(applied(1));
    h.queue_lookup(reply("applied", "e2", Some(0), None));
    let op_id = h.start(h.rename(), false).unwrap();
    h.install("e2", 0);
    let _ = h.reply_of(&op_id);
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_new_epoch_arriving_before_the_revision_sends_the_waiting_receipt_to_a_lookup() {
    let mut h = Harness::new();
    h.queue(applied(5));
    let _ = h.send(h.rename());
    h.queue_lookup(reply("applied", "e2", Some(0), None));
    h.install("e2", 0);
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn an_unknown_outcome_stays_until_dismissed() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(reply("unknown", "e1", None, None));
    let _ = h.send(h.rename());
    h.settle_lookups();
    h.advance(1);
    h.install("e1", 1);
    h.settle_lookups();
    let kept = h.unknown_marks();
    let dismissed = h.actions.dismiss_unknown();
    assert_eq!(kept, [true]);
    assert_eq!(dismissed, 1);
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn an_unknown_outcome_goes_with_its_row() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(reply("unknown", "e1", None, None));
    let _ = h.send(h.rename());
    h.settle_lookups();
    h.catalog.workspaces[0].sessions.clear();
    h.advance(1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn nothing_is_sent_while_the_window_is_not_connected() {
    let mut h = Harness::new();
    h.connected = false;
    let response = h.send(h.rename());
    assert_eq!(response.error.as_deref(), Some("box is not connected"));
    assert!(h.script.calls.lock().unwrap().is_empty());
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_stopped_window_sends_nothing() {
    let mut h = Harness::new();
    h.actions.stop();
    let response = h.send(h.rename());
    assert_eq!(response.error.as_deref(), Some("box is not connected"));
    assert!(h.script.calls.lock().unwrap().is_empty());
}

#[test]
fn a_pane_action_pins_the_panes_the_window_saw() {
    let mut h = Harness::new();
    h.queue(reply("noop", "e1", None, None));
    let _ = h.send(RemoteOperation::Swap(h.session));
    let line = h.script.command_calls().remove(0);
    let pinned = format!(
        "--expect-panes {},{} --op-id",
        uuid_string(h.panes[0].identity),
        uuid_string(h.panes[1].identity)
    );
    assert!(line.contains(&pinned), "{line}");
}

#[test]
fn a_replacing_scratch_carries_the_scratch_the_window_saw() {
    let mut h = Harness::new();
    let scratch = pane_with(Uuid::new_v4(), Some("scratch"));
    h.catalog.workspaces[0].sessions[0].fields.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: false,
    });
    h.queue(reply("noop", "e1", None, None));
    h.queue(reply("noop", "e1", None, None));
    let session = h.session;
    let _ = h.send(RemoteOperation::SetScratch {
        session,
        shown: true,
        command: Some("-htop".to_owned()),
    });
    let _ = h.send(RemoteOperation::SetScratch {
        session,
        shown: false,
        command: None,
    });
    let lines = h.script.command_calls();
    let expected = format!(
        "session scratch on --target {} --command=-htop --expect-scratch {} --op-id",
        uuid_string(session),
        uuid_string(scratch.identity)
    );
    assert!(lines[0].contains(&expected), "{}", lines[0]);
    assert!(!lines[1].contains("--expect-scratch"), "{}", lines[1]);
}

#[test]
fn a_replacing_scratch_on_a_row_without_one_expects_none() {
    let mut h = Harness::new();
    h.queue(reply("noop", "e1", None, None));
    let _ = h.send(RemoteOperation::SetScratch {
        session: h.session,
        shown: true,
        command: Some("htop".to_owned()),
    });
    assert!(h.script.command_calls()[0].contains("--expect-scratch none --op-id"));
}

#[test]
fn a_pane_action_keeps_the_panes_seen_when_it_was_decided() {
    let mut h = Harness::new();
    h.queue(reply("refused", "e1", None, None));
    let prepared = h
        .actions
        .prepare(h.link(), RemoteOperation::Swap(h.session), None)
        .unwrap();
    let replacement = pane();
    h.catalog.workspaces[0].sessions[0].panes[1] = replacement.clone();
    h.advance(2);
    let link = Link {
        host: "box",
        connected: true,
        catalog: Some(&h.catalog),
    };
    let op_id = h
        .actions
        .send(link, &h.runner, prepared, Asker::Control, false)
        .unwrap();
    let _ = h.reply_of(&op_id);
    let sent = h.script.command_calls().remove(0);
    let pinned = format!(
        "--expect-panes {},{} --op-id",
        uuid_string(h.panes[0].identity),
        uuid_string(h.panes[1].identity)
    );
    assert!(sent.contains(&pinned), "{sent}");
    assert!(!sent.contains(&uuid_string(replacement.identity)));
}

#[test]
fn a_pane_action_on_an_ended_row_or_a_missing_one_is_refused_here() {
    let mut h = Harness::new();
    h.catalog.workspaces[0].sessions[0].panes.clear();
    let ended = h.send(RemoteOperation::CloseSplit(h.session));
    let missing = h.send(RemoteOperation::Swap(Uuid::new_v4()));
    assert_eq!(ended.error.as_deref(), Some("the session has ended on box"));
    assert_eq!(missing.error.as_deref(), Some("no such session on box"));
    assert!(h.script.calls.lock().unwrap().is_empty());
}

#[test]
fn a_row_reads_back_pending_then_unknown() {
    let mut h = Harness::new();
    h.queue(applied(3));
    h.queue(lost());
    h.queue_lookup(reply("unknown", "e1", None, None));
    let _ = h.send(h.rename());
    let pending = (h.actions.pending_state(h.session), h.actions.marks().len());
    let _ = h.send(RemoteOperation::Swap(h.session));
    h.settle_lookups();
    assert_eq!(pending, (Some("pending"), 1));
    assert_eq!(h.actions.pending_state(h.session), Some("unknown"));
    assert_eq!(h.unknown_marks(), [false, true]);
    assert_eq!(h.actions.pending_state(Uuid::new_v4()), None);
}

#[test]
fn the_remote_node_lists_workspace_and_targetless_operations() {
    let mut h = Harness::new();
    h.queue(applied(4));
    h.queue(applied(5));
    let workspace = h.workspace;
    let _ = h.send(RemoteOperation::RenameWorkspace {
        workspace,
        name: "ops".to_owned(),
    });
    let _ = h.send(RemoteOperation::NewWorkspace {
        name: Some("dev".to_owned()),
    });
    let operations: Vec<ControlRemoteOperation> = h.actions.marks().iter().map(Into::into).collect();
    let actions: Vec<&str> = operations.iter().map(|operation| operation.action.as_str()).collect();
    assert_eq!(actions, ["Rename Workspace", "New Workspace"]);
    assert_eq!(operations[0].workspace, Some(uuid_string(workspace)));
    assert_eq!(operations[1].workspace, None);
    assert!(
        operations
            .iter()
            .all(|operation| operation.session.is_none() && operation.state == "pending")
    );
}

#[test]
fn every_change_to_the_tracked_operations_is_announced() {
    let mut h = Harness::new();
    h.queue(applied(1));
    let _ = h.send(h.rename());
    assert!(h.actions.take_changed());
    assert!(!h.actions.take_changed());
    h.advance(1);
    assert!(h.actions.take_changed());
    assert!(h.actions.marks().is_empty());
}

#[test]
fn the_created_id_comes_back_with_the_reply_and_is_handed_for_selection() {
    let mut h = Harness::new();
    let created = Uuid::new_v4();
    h.queue(reply("applied", "e1", Some(1), Some(&uuid_string(created))));
    let link = Link {
        host: "box",
        connected: true,
        catalog: Some(&h.catalog),
    };
    let prepared = h
        .actions
        .prepare(link, RemoteOperation::new_session(h.workspace), Some(h.session))
        .unwrap();
    let op_id = h.actions.send(link, &h.runner, prepared, Asker::Gui, true).unwrap();
    let mut finished = Vec::new();
    for _ in 0..2000 {
        finished = h.poll();
        if !finished.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let done = finished.pop().expect("an answer");
    assert_eq!(done.op_id, op_id);
    assert_eq!(done.asker, Asker::Gui);
    assert_eq!(done.created, Some((created, Some(h.session))));
    assert!(h.actions.take_reply(&op_id).is_none(), "a GUI send keeps no reply");
}

#[test]
fn a_send_whose_window_went_down_after_it_was_decided_is_refused() {
    let mut h = Harness::new();
    let prepared = h.actions.prepare(h.link(), h.rename(), None).unwrap();
    let link = Link {
        host: "box",
        connected: false,
        catalog: Some(&h.catalog),
    };
    let refused = h.actions.send(link, &h.runner, prepared, Asker::Control, false);
    assert_eq!(refused, Err("box is not connected".to_owned()));
    assert!(h.actions.marks().is_empty());
    assert!(format!("{:?}", h.actions).starts_with("RemoteActions"));
}

#[test]
fn an_applied_receipt_without_a_revision_is_asked_about_again() {
    let mut h = Harness::new();
    h.queue(reply("applied", "e1", None, None));
    h.queue_lookup(reply("fenced", "e1", None, None));
    let _ = h.send(h.rename());
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert!(h.actions.marks().is_empty());
}

#[test]
fn a_receipt_before_any_catalog_keeps_its_mark() {
    let mut h = Harness::new();
    h.queue(applied(1));
    let op_id = h.start(h.rename(), false).unwrap();
    for _ in 0..2000 {
        let link = Link {
            host: "box",
            connected: true,
            catalog: None,
        };
        if !h.actions.poll(link, &h.runner).is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.actions.take_reply(&op_id).is_some_and(|reply| reply.ok));
    assert_eq!(h.unknown_marks(), [false]);
    let link = Link {
        host: "box",
        connected: true,
        catalog: None,
    };
    h.actions.catalog_moved(link, &h.runner, true);
    assert_eq!(h.unknown_marks(), [false]);
}

#[test]
fn lost_replies_are_not_asked_about_while_the_window_is_down() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(lost());
    let _ = h.send(h.rename());
    h.settle_lookups();
    h.connected = false;
    h.install("e1", 0);
    h.settle_lookups();
    assert_eq!(h.script.lookup_calls().len(), 1);
    assert_eq!(h.unknown_marks(), [true]);
}

#[test]
fn an_unknown_workspace_operation_goes_with_its_workspace() {
    let mut h = Harness::new();
    h.queue(lost());
    h.queue_lookup(reply("unknown", "e1", None, None));
    let _ = h.send(RemoteOperation::DeleteWorkspace(h.workspace));
    h.settle_lookups();
    h.advance(1);
    assert_eq!(h.unknown_marks(), [true]);
    h.catalog.workspaces.clear();
    h.advance(2);
    assert!(h.actions.marks().is_empty());
}
