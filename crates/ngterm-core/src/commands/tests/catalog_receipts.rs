//! Operation receipts on a served catalog (upstream's `ServerActionsReceiptsTests` and the dispatcher cases of
//! `CatalogOperationsTests`): a structural command sent with an `opID` runs once and answers its receipt, a repeat
//! answers the same, a lookup fences an unseen id, and nothing only memory holds is handed out.

use ngterm_proto::command::{CatalogCmd, SessionCmd, WorkspaceCmd};
use std::path::{Path, PathBuf};

use ngterm_proto::{AppIdentity, Args, CatalogReceipt, Command, Request, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog_served::served;
use super::overlay_page::fixture_with_host;
use super::{Fixture, answered_id, args, wire};
use crate::catalog::OPERATIONS_FILE_NAME;
use crate::catalog::fixture::CATALOG;
use crate::catalog::operations::{CatalogOperationTable, CatalogPosition, outcome, receipt};
use crate::dispatch::{Dispatched, OPERATION_RECEIPTS_ERROR};
use crate::persist::{AppSettings, LibraryPersistence};
use crate::{Core, CoreConfig, CoreState};

fn epoch(fixture: &Fixture) -> String {
    fixture.core.remote.respawns.epoch().to_owned()
}

fn rev(fixture: &Fixture) -> i64 {
    fixture
        .core
        .catalog
        .served
        .as_ref()
        .map_or(0, |served| served.hub.rev())
}

/// `cmd` sent with `op` at the catalog's epoch.
fn serve(fixture: &mut Fixture, cmd: Command, target: Option<&str>, base: Args, op: &str) -> Response {
    let epoch = epoch(fixture);
    serve_at(fixture, cmd, target, base, op, Some(&epoch))
}

fn serve_at(
    fixture: &mut Fixture,
    cmd: Command,
    target: Option<&str>,
    mut base: Args,
    op: &str,
    at: Option<&str>,
) -> Response {
    base.op_id = Some(op.to_owned());
    base.epoch = at.map(str::to_owned);
    let response = fixture.send(cmd, target, base);
    let _ = fixture.core.commit();
    response
}

fn lookup(fixture: &mut Fixture, op: &str, sent: Option<CatalogPosition>) -> Response {
    let args = args(|a| {
        a.epoch = sent.as_ref().map(|sent| sent.epoch.clone());
        a.rev = sent.map(|sent| sent.rev);
    });
    fixture.send(Command::Catalog(CatalogCmd::Op), Some(op), args)
}

fn receipt_of(response: &Response) -> Option<CatalogReceipt> {
    serde_json::from_value(response.result.as_ref()?.get("receipt")?.clone()).ok()
}

fn session_count(fixture: &Fixture) -> usize {
    fixture.store().sessions().count()
}

fn named(name: &str) -> Args {
    args(|a| a.name = Some(name.to_owned()))
}

fn mode(mode: &str) -> Args {
    args(|a| a.mode = Some(mode.to_owned()))
}

const SESSION_NEW: Command = Command::Session(SessionCmd::New);

/// Upstream's `everyStructuralCommandAnswersAppliedAtTheRevisionItsChangeReached`.
#[test]
fn every_structural_command_answers_applied_at_the_revision_its_change_reached() {
    let mut f = served(0);
    let created = serve(&mut f, SESSION_NEW, None, Args::default(), "new");
    let id = wire(answered_id(&created).expect("a session"));
    let workspace = serve(&mut f, Command::Workspace(WorkspaceCmd::New), None, named("dev"), "ws");
    let ws = workspace
        .result
        .as_ref()
        .and_then(|result| result["id"].as_str())
        .unwrap()
        .to_owned();
    let session = |cmd| Command::Session(cmd);
    let workspace_cmd = |cmd| Command::Workspace(cmd);
    let steps: Vec<(Command, String, Args, &str)> = vec![
        (session(SessionCmd::Rename), id.clone(), named("build"), "rename"),
        (session(SessionCmd::Split), id.clone(), mode("on"), "split"),
        (
            session(SessionCmd::SplitClose),
            id.clone(),
            Args::default(),
            "split-close",
        ),
        (
            session(SessionCmd::CompanionNew),
            id.clone(),
            Args::default(),
            "companion",
        ),
        (
            workspace_cmd(WorkspaceCmd::Rename),
            ws.clone(),
            named("ops"),
            "ws-rename",
        ),
        (
            session(SessionCmd::Move),
            id.clone(),
            args(|a| a.workspace = Some(ws.clone())),
            "move",
        ),
        (
            workspace_cmd(WorkspaceCmd::Move),
            ws.clone(),
            args(|a| a.to = Some("top".into())),
            "ws-move",
        ),
        (session(SessionCmd::Close), id.clone(), Args::default(), "close"),
        (
            workspace_cmd(WorkspaceCmd::Delete),
            ws.clone(),
            Args::default(),
            "ws-delete",
        ),
    ];
    let mut replies = vec![created, workspace];
    for (cmd, target, base, op) in steps {
        replies.push(serve(&mut f, cmd, Some(&target), base, op));
    }
    let receipts: Vec<CatalogReceipt> = replies.iter().filter_map(receipt_of).collect();
    assert!(replies.iter().all(|reply| reply.ok), "{replies:?}");
    assert_eq!(receipts.len(), replies.len());
    let epoch = epoch(&f);
    assert!(
        receipts
            .iter()
            .all(|r| r.outcome == outcome::APPLIED && r.epoch == epoch),
        "{receipts:?}"
    );
    let revisions: Vec<i64> = receipts.iter().filter_map(|r| r.rev).collect();
    assert_eq!(revisions.len(), replies.len());
    assert!(revisions.is_sorted(), "{revisions:?}");
    assert_eq!(revisions.last().copied(), Some(rev(&f)));
}

/// Upstream's `aCommandTheHostRefusesAnswersRefusedWithItsError`.
#[test]
fn a_command_the_host_refuses_answers_refused_with_its_error() {
    let mut f = served(0);
    let target = wire(Uuid::new_v4());
    let reply = serve(
        &mut f,
        Command::Session(SessionCmd::Rename),
        Some(&target),
        named("x"),
        "bad",
    );
    assert!(!reply.ok);
    let receipt = receipt_of(&reply).unwrap();
    assert_eq!(receipt.outcome, outcome::REFUSED);
    assert_eq!(receipt.error, reply.error);
}

/// Upstream's `aCommandThatChangesNothingAnswersNoop`.
#[test]
fn a_command_that_changes_nothing_answers_noop() {
    let mut f = served(0);
    let target = wire(f.sessions[0]);
    let reply = serve(
        &mut f,
        Command::Session(SessionCmd::Split),
        Some(&target),
        mode("off"),
        "off",
    );
    assert!(reply.ok, "{reply:?}");
    assert_eq!(receipt_of(&reply), Some(receipt("off", &epoch(&f), outcome::NOOP)));
}

/// Upstream's `aRepeatedOperationNeverRunsTwice`.
#[test]
fn a_repeated_operation_never_runs_twice() {
    let mut f = served(0);
    let first = serve(&mut f, SESSION_NEW, None, Args::default(), "once");
    let count = session_count(&f);
    let second = serve(&mut f, SESSION_NEW, None, Args::default(), "once");
    assert_eq!(session_count(&f), count);
    assert!(second.ok);
    assert_eq!(receipt_of(&second), receipt_of(&first));
    assert_eq!(second.result, Some(json!({"receipt": receipt_of(&first)})));
}

/// Upstream's `aRefusalIsRecordedSoTheIdStaysRefused` and `anotherEpochIsRefusedBeforeTheCommandRuns`.
#[test]
fn another_epoch_is_refused_before_the_command_runs_and_stays_refused() {
    let mut f = served(0);
    let count = session_count(&f);
    let reply = serve_at(
        &mut f,
        SESSION_NEW,
        None,
        Args::default(),
        "stale",
        Some("an-old-epoch"),
    );
    let epoch = epoch(&f);
    assert_eq!(session_count(&f), count);
    assert_eq!(
        reply.error.as_deref(),
        Some(format!("the catalog is at epoch {epoch}; take its snapshot and send the command again").as_str())
    );
    assert_eq!(receipt_of(&reply).map(|r| r.outcome), Some(outcome::REFUSED.to_owned()));
    let retried = serve(&mut f, SESSION_NEW, None, Args::default(), "stale");
    assert_eq!(session_count(&f), count);
    assert_eq!(retried, reply);
}

/// Upstream's `anOperationIdWithoutAnEpochIsRefused`.
#[test]
fn an_operation_id_without_an_epoch_is_refused() {
    let mut f = served(0);
    let reply = serve_at(&mut f, SESSION_NEW, None, Args::default(), "no-epoch", None);
    assert_eq!(receipt_of(&reply).map(|r| r.outcome), Some(outcome::REFUSED.to_owned()));
}

/// Upstream's `aPageRowCreatedUnderAnOperationIdIsAppliedOnce`.
#[test]
fn a_page_row_created_under_an_operation_id_is_applied_once() {
    for cmd in [SessionCmd::Page, SessionCmd::CompanionPage] {
        let (mut f, _) = fixture_with_host(0);
        f.core.serve_catalog(CATALOG, None);
        let anchor = wire(f.sessions[0]);
        let page = || args(|a| a.html = Some(crate::html::os_path("/tmp/p.html")));
        let first = serve(&mut f, Command::Session(cmd), Some(&anchor), page(), "page-row");
        let count = session_count(&f);
        let repeated = serve(&mut f, Command::Session(cmd), Some(&anchor), page(), "page-row");
        assert_eq!(
            receipt_of(&first).map(|r| r.outcome),
            Some(outcome::APPLIED.to_owned()),
            "{cmd:?}"
        );
        assert_eq!(receipt_of(&repeated), receipt_of(&first));
        assert_eq!(session_count(&f), count);
        let row = answered_id(&first).unwrap();
        assert_eq!(
            f.store().session(row).unwrap().content(),
            crate::model::SessionContent::Page
        );
    }
}

/// Upstream's `aTerminalCommandOnAPageRowIsARefusedReceiptThatStaysRefused`.
#[test]
fn a_terminal_command_on_a_page_row_is_a_refused_receipt_that_stays_refused() {
    let (mut f, _) = fixture_with_host(0);
    f.core.serve_catalog(CATALOG, None);
    let anchor = wire(f.sessions[0]);
    let page = f.send(
        Command::Session(SessionCmd::Page),
        Some(&anchor),
        args(|a| a.html = Some(crate::html::os_path("/tmp/p.html"))),
    );
    let row = answered_id(&page).unwrap();
    let split = Command::Session(SessionCmd::Split);
    let reply = serve(&mut f, split, Some(&wire(row)), mode("on"), "page-split");
    let repeated = serve(&mut f, split, Some(&wire(row)), mode("on"), "page-split");
    assert_eq!(receipt_of(&reply).map(|r| r.outcome), Some(outcome::REFUSED.to_owned()));
    assert_eq!(
        reply.error.as_deref(),
        Some("session.split: not available on a page session")
    );
    assert_eq!(repeated, reply);
    assert!(!f.store().session(row).unwrap().split.exists);
}

/// Upstream's `aNonStructuralCommandRefusesAnOperationId`.
#[test]
fn a_non_structural_command_refuses_an_operation_id() {
    let mut f = served(0);
    let target = wire(f.sessions[0]);
    let status = args(|a| a.status = Some("active".into()));
    let reply = serve(
        &mut f,
        Command::Session(SessionCmd::Status),
        Some(&target),
        status,
        "st",
    );
    assert_eq!(reply, Response::failure("session.status takes no operation id"));
    assert!(f.core.catalog.served.as_ref().unwrap().table.receipt("st").is_none());
    let bad = serve(&mut f, SESSION_NEW, None, Args::default(), "a b");
    assert_eq!(bad, Response::failure("invalid operation id"));
}

/// Upstream's `aLookupFencesAnUnseenIdSoItsLateArrivalIsRefused`.
#[test]
fn a_lookup_fences_an_unseen_id_so_its_late_arrival_is_refused() {
    let mut f = served(0);
    let fence = lookup(&mut f, "late", None);
    let count = session_count(&f);
    let arrival = serve(&mut f, SESSION_NEW, None, Args::default(), "late");
    assert!(fence.ok);
    assert_eq!(receipt_of(&fence), Some(receipt("late", &epoch(&f), outcome::FENCED)));
    assert_eq!(session_count(&f), count);
    assert_eq!(
        arrival.error.as_deref(),
        Some("operation late was fenced by a lookup and will not run")
    );
    assert_eq!(
        receipt_of(&arrival).map(|r| r.outcome),
        Some(outcome::FENCED.to_owned())
    );
}

fn fill_table(f: &mut Fixture, position: &CatalogPosition) {
    let table = &mut f.core.catalog.served.as_mut().unwrap().table;
    for index in 0..=CatalogOperationTable::CAPACITY {
        table.record(
            receipt(&format!("filler{index}"), &position.epoch, outcome::NOOP),
            position,
        );
    }
}

fn on_disk(dir: &std::path::Path) -> CatalogOperationTable {
    serde_json::from_slice(&std::fs::read(dir.join(OPERATIONS_FILE_NAME)).unwrap()).unwrap()
}

/// Upstream's `aMissTheTablesDropCouldCoverIsUnknownAndStaysUnknown`.
#[test]
fn a_miss_the_tables_drop_could_cover_is_unknown_and_stays_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let sent = CatalogPosition::new(&epoch(&f), rev(&f));
    fill_table(&mut f, &sent);
    let count = session_count(&f);
    let first = lookup(&mut f, "lost", Some(sent.clone()));
    let again = lookup(&mut f, "lost", Some(sent));
    let arrival = serve(&mut f, SESSION_NEW, None, Args::default(), "lost");
    assert!(first.ok);
    assert_eq!(receipt_of(&first), Some(receipt("lost", &epoch(&f), outcome::UNKNOWN)));
    assert_eq!(receipt_of(&again).map(|r| r.outcome), Some(outcome::UNKNOWN.to_owned()));
    assert_eq!(
        arrival.error.as_deref(),
        Some("operation lost has an unknown outcome and will not run")
    );
    assert_eq!(session_count(&f), count);
    assert_eq!(
        on_disk(dir.path()).receipt("lost").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// Upstream's `aMissSentAfterTheLastDropIsFenced`.
#[test]
fn a_miss_sent_after_the_last_drop_is_fenced() {
    let mut f = served(0);
    let at = CatalogPosition::new(&epoch(&f), rev(&f));
    fill_table(&mut f, &at);
    let _ = f.send(SESSION_NEW, None, Args::default());
    let _ = f.core.commit();
    let now = CatalogPosition::new(&epoch(&f), rev(&f));
    let reply = lookup(&mut f, "late", Some(now));
    assert_eq!(receipt_of(&reply).map(|r| r.outcome), Some(outcome::FENCED.to_owned()));
}

fn split_session(f: &mut Fixture) -> (Uuid, Uuid, Uuid) {
    let session = f.sessions[0];
    let _ = f.send(Command::Session(SessionCmd::Split), Some(&wire(session)), mode("on"));
    let model = f.store().session(session).unwrap();
    (session, model.pane_identity, model.split_pane_identity.unwrap())
}

fn pinned(panes: &[Uuid]) -> Args {
    args(|a| a.panes = Some(panes.iter().map(|pane| wire(*pane).to_lowercase()).collect()))
}

/// Upstream's `aPanePinnedCommandRunsOnTheSameShapedSession`: a swap, which waits on its terminals, settles its
/// receipt once it answers.
#[test]
fn a_pane_pinned_command_runs_on_the_same_shaped_session() {
    use ngterm_host::fakes::FakeSurface;
    let mut f = served(0);
    let (session, primary, split) = split_session(&mut f);
    let panes = f.store_mut().and_then(|store| store.session_mut(session)).unwrap();
    let _ = panes
        .surface
        .replace(Box::new(FakeSurface::new("left").with_realized(true)));
    let _ = panes
        .split_surface
        .replace(Box::new(FakeSurface::new("right").with_realized(true)));
    let reply = serve(
        &mut f,
        Command::Session(SessionCmd::Swap),
        Some(&wire(session)),
        pinned(&[primary, split]),
        "swap",
    );
    assert!(reply.ok, "{reply:?}");
    let receipt = receipt_of(&reply).unwrap();
    assert_eq!(
        (receipt.outcome.as_str(), receipt.rev),
        (outcome::APPLIED, Some(rev(&f)))
    );
    assert_eq!(f.store().session(session).unwrap().pane_identity, split);
}

/// Upstream's `aStalePanePinIsRefusedWhateverFormTheTargetTakes` and `aPanePinnedCommandIsRefusedOnceThePanesChanged`.
#[test]
fn a_stale_pane_pin_is_refused_whatever_form_the_target_takes() {
    let mut f = served(0);
    let (session, primary, split) = split_session(&mut f);
    let prefix = wire(session)[..8].to_owned();
    for (target, op) in [(prefix.as_str(), "by-prefix"), ("active", "by-active")] {
        let stale = pinned(&[primary, Uuid::new_v4()]);
        let reply = serve(
            &mut f,
            Command::Session(SessionCmd::SplitClose),
            Some(target),
            stale,
            op,
        );
        assert!(!reply.ok);
        assert_eq!(
            reply.error.as_deref(),
            Some("the session's panes changed; take the catalog again")
        );
        assert_eq!(receipt_of(&reply).map(|r| r.outcome), Some(outcome::REFUSED.to_owned()));
        assert_eq!(f.store().session(session).unwrap().split_pane_identity, Some(split));
    }
}

#[test]
fn a_scratch_pin_is_refused_once_the_scratch_is_not_the_one_seen() {
    let mut f = served(0);
    let target = wire(f.sessions[0]);
    let none = args(|a| a.scratch = Some("NONE".into()));
    let kept = serve(
        &mut f,
        Command::Session(SessionCmd::Scratch),
        Some(&target),
        none,
        "scratch-none",
    );
    assert!(kept.ok, "{kept:?}");
    let other = args(|a| a.scratch = Some(wire(Uuid::new_v4())));
    let reply = serve(
        &mut f,
        Command::Session(SessionCmd::Scratch),
        Some(&target),
        other,
        "scratch-other",
    );
    assert_eq!(
        reply.error.as_deref(),
        Some("the session's scratch changed; take the catalog again")
    );
}

/// Upstream's `aLookupAnswersARecordedOperation`.
#[test]
fn a_lookup_answers_a_recorded_operation() {
    let mut f = served(0);
    let applied = serve(&mut f, Command::Workspace(WorkspaceCmd::New), None, named("dev"), "ws");
    let lookup = lookup(&mut f, "ws", None);
    assert!(lookup.ok);
    assert_eq!(receipt_of(&lookup), receipt_of(&applied));
    assert_eq!(lookup.result, Some(json!({"receipt": receipt_of(&applied)})));
}

/// Upstream's `aReceiptThatCannotBeSavedIsNotHandedOut`.
#[test]
fn a_receipt_that_cannot_be_saved_is_not_handed_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let blocker = dir.path().join(OPERATIONS_FILE_NAME).join("blocker");
    std::fs::create_dir_all(&blocker).unwrap();
    let unsaved = Response::failure("the receipt for operation unsaved could not be saved; ask catalog.op for it");
    let workspace_new = Command::Workspace(WorkspaceCmd::New);
    let reply = serve(&mut f, workspace_new, None, named("dev"), "unsaved");
    let fence = lookup(&mut f, "unseen", None);
    let lookup_blocked = lookup(&mut f, "unsaved", None);
    let repeat_blocked = serve(&mut f, workspace_new, None, named("dev"), "unsaved");
    let fence_again = lookup(&mut f, "unseen", None);
    let late = serve(&mut f, workspace_new, None, named("late"), "unseen");
    assert_eq!(reply, unsaved);
    assert_eq!(
        fence,
        Response::failure("could not save the fence for operation unseen")
    );
    assert_eq!(lookup_blocked, unsaved);
    assert_eq!(repeat_blocked, unsaved);
    assert!(!fence_again.ok);
    assert!(!late.ok);
    let names: Vec<&str> = f.store().workspaces().iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names.iter().filter(|name| **name == "dev").count(), 1, "{names:?}");
    assert!(!names.contains(&"late"));

    std::fs::remove_dir_all(dir.path().join(OPERATIONS_FILE_NAME)).unwrap();
    let lookup_saved = lookup(&mut f, "unsaved", None);
    let fence_saved = lookup(&mut f, "unseen", None);
    assert!(lookup_saved.ok);
    assert_eq!(
        receipt_of(&lookup_saved).map(|r| r.outcome),
        Some(outcome::APPLIED.to_owned())
    );
    assert_eq!(
        receipt_of(&fence_saved).map(|r| r.outcome),
        Some(outcome::FENCED.to_owned())
    );
    let table = on_disk(dir.path());
    assert_eq!(
        table.receipt("unsaved").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
    assert_eq!(
        table.receipt("unseen").map(|r| r.outcome.clone()),
        Some(outcome::FENCED.to_owned())
    );
}

/// A stop while a served command waits on its terminals leaves no `applied` entry behind, even when another request
/// writes the table meanwhile: the id answers as still running, and the restart fences it rather than vouch for a swap
/// that never ran.
#[test]
fn a_stop_while_an_operation_waits_leaves_no_applied_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let (session, _, _) = split_session(&mut f);
    let epoch = epoch(&f);
    let swap = Request {
        cmd: Command::Session(SessionCmd::Swap),
        target: Some(wire(session)),
        args: Some(args(|a| {
            a.op_id = Some("waiting".to_owned());
            a.epoch = Some(epoch);
        })),
    };
    let dispatched = f.core.begin(&swap);
    assert!(matches!(dispatched, Dispatched::Pending { .. }));
    let _ = f.core.commit();
    let running = Response::failure("operation waiting is still running; ask catalog.op again once it ends");
    assert_eq!(lookup(&mut f, "waiting", None), running);
    assert_eq!(f.core.dispatch(&swap), running);
    let other = lookup(&mut f, "other", None);
    assert_eq!(receipt_of(&other).map(|r| r.outcome), Some(outcome::FENCED.to_owned()));
    assert!(on_disk(dir.path()).receipt("waiting").is_none());
    drop(dispatched);
    drop(f);

    let mut restarted = Fixture::new(0);
    restarted.core.serve_catalog(CATALOG, Some(dir.path()));
    let answer = lookup(&mut restarted, "waiting", None);
    assert_eq!(receipt_of(&answer).map(|r| r.outcome), Some(outcome::FENCED.to_owned()));
}

/// A command dropped before it answers (a panicked handler, a step never resumed) leaves its id `unknown`, not
/// running: the repeat and the lookup both answer, and neither runs it again.
#[test]
fn a_dropped_operation_answers_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let (session, _, _) = split_session(&mut f);
    let epoch = epoch(&f);
    let swap = Request {
        cmd: Command::Session(SessionCmd::Swap),
        target: Some(wire(session)),
        args: Some(args(|a| {
            a.op_id = Some("dropped".to_owned());
            a.epoch = Some(epoch);
        })),
    };
    let dispatched = f.core.begin(&swap);
    assert!(matches!(dispatched, Dispatched::Pending { .. }));
    drop(dispatched);
    let _ = f.core.commit();
    assert_eq!(
        on_disk(dir.path()).receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );

    let repeat = f.core.dispatch(&swap);
    assert_eq!(
        repeat.error.as_deref(),
        Some("operation dropped has an unknown outcome and will not run")
    );
    assert_eq!(
        receipt_of(&repeat).map(|r| r.outcome),
        Some(outcome::UNKNOWN.to_owned())
    );
    let answer = lookup(&mut f, "dropped", None);
    assert_eq!(
        receipt_of(&answer).map(|r| r.outcome),
        Some(outcome::UNKNOWN.to_owned())
    );
    assert_eq!(
        on_disk(dir.path()).receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// A restart after a dropped command answers `unknown` without a lookup in between, since the next write recorded it.
#[test]
fn a_dropped_operation_stays_unknown_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let (session, _, _) = split_session(&mut f);
    let epoch = epoch(&f);
    let swap = swap_request(session, "dropped", epoch);
    drop(f.core.begin(&swap));
    let _ = f.core.commit();
    drop(f);

    let mut restarted = Fixture::new(0);
    restarted.core.serve_catalog(CATALOG, Some(dir.path()));
    let answer = lookup(&mut restarted, "dropped", None);
    assert_eq!(
        receipt_of(&answer).map(|r| r.outcome),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// The quit save records a command that still waits as `unknown`: its step never runs after the quit, but its change
/// may already be in the windows that save writes.
#[test]
fn the_quit_save_leaves_a_waiting_operation_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(0);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let (session, _, _) = split_session(&mut f);
    let epoch = epoch(&f);
    let swap = swap_request(session, "waiting", epoch);
    let dispatched = f.core.begin(&swap);
    assert!(matches!(dispatched, Dispatched::Pending { .. }));
    assert!(f.core.save_all());
    drop(dispatched);
    drop(f);

    let mut restarted = Fixture::new(0);
    restarted.core.serve_catalog(CATALOG, Some(dir.path()));
    let answer = lookup(&mut restarted, "waiting", None);
    assert_eq!(
        receipt_of(&answer).map(|r| r.outcome),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// A debounced window save that lands after a dropped command, with no commit in between, writes its `unknown` first:
/// the window it writes may hold that command's change.
#[test]
fn a_debounced_save_after_a_dropped_operation_writes_it_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, _) = persistent_served(dir.path());
    let window = core.library().active_window_id().unwrap();
    let session = core.library().store(window).unwrap().sessions().next().unwrap().id();
    let _ = core.dispatch(&Request {
        cmd: Command::Session(SessionCmd::Split),
        target: Some(wire(session)),
        args: Some(mode("on")),
    });
    let _ = core.commit();
    core.library_mut().store_mut(window).unwrap().schedule_save();
    let saves = core.commit().debounced;
    assert_eq!(saves.len(), 1);
    let epoch = core.remote.respawns.epoch().to_owned();
    let dispatched = core.begin(&swap_request(session, "dropped", epoch));
    assert!(matches!(dispatched, Dispatched::Pending { .. }));
    drop(dispatched);
    for save in saves {
        core.fire_debounced(save);
    }
    assert_eq!(
        on_disk(dir.path()).receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// Another command's receipt, written with the windows, records a command dropped while it ran as `unknown` too.
#[test]
fn a_receipt_written_after_a_dropped_operation_writes_it_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(1);
    f.core.serve_catalog(CATALOG, Some(dir.path()));
    let (first, _, _) = split_session(&mut f);
    let second = f.sessions[1];
    let _ = f.send(Command::Session(SessionCmd::Split), Some(&wire(second)), mode("on"));
    let epoch = epoch(&f);
    let kept = f.core.begin(&swap_request(first, "kept", epoch.clone()));
    let dropped = f.core.begin(&swap_request(second, "dropped", epoch));
    assert!(matches!(kept, Dispatched::Pending { .. }));
    assert!(matches!(dropped, Dispatched::Pending { .. }));
    drop(dropped);
    let _ = kept.finish_now(&mut f.core);
    let table = on_disk(dir.path());
    assert!(table.receipt("kept").is_some());
    assert_eq!(
        table.receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

fn swap_request(session: Uuid, op_id: &str, epoch: String) -> Request {
    Request {
        cmd: Command::Session(SessionCmd::Swap),
        target: Some(wire(session)),
        args: Some(args(|a| {
            a.op_id = Some(op_id.to_owned());
            a.epoch = Some(epoch);
        })),
    }
}

/// A core over a library bootstrapped from `dir` that serves its catalog there too, as `ngterm-headless` does; the
/// path of its window's file.
fn persistent_served(dir: &Path) -> (Core, PathBuf) {
    let mut persistence = LibraryPersistence::new(dir);
    let library = persistence.bootstrap("/home/u");
    let window = library.active_window_id().expect("the seeded window");
    let file = persistence.window_path(window);
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: Some(persistence),
        },
    );
    let _ = core.commit();
    core.serve_catalog(CATALOG, Some(dir));
    (core, file)
}

/// The receipt vouches for a model change on disk: a window that cannot be written keeps it unsaved, and the retry
/// writes the window before the receipt.
#[test]
fn a_receipt_waits_for_the_window_it_vouches_for() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = persistent_served(dir.path());
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(file.join("blocker")).unwrap();
    let epoch = core.remote.respawns.epoch().to_owned();
    let reply = core.dispatch(&Request {
        cmd: Command::Workspace(WorkspaceCmd::New),
        target: None,
        args: Some(args(|a| {
            a.name = Some("dev".to_owned());
            a.op_id = Some("ws".to_owned());
            a.epoch = Some(epoch);
        })),
    });
    let _ = core.commit();
    assert_eq!(
        reply,
        Response::failure("the receipt for operation ws could not be saved; ask catalog.op for it")
    );
    let table = dir.path().join(OPERATIONS_FILE_NAME);
    assert!(!table.exists() || on_disk(dir.path()).receipt("ws").is_none());

    std::fs::remove_dir_all(&file).unwrap();
    let lookup = core.dispatch(&Request {
        cmd: Command::Catalog(CatalogCmd::Op),
        target: Some("ws".to_owned()),
        args: Some(Args::default()),
    });
    assert_eq!(
        receipt_of(&lookup).map(|r| r.outcome),
        Some(outcome::APPLIED.to_owned()),
        "{lookup:?}"
    );
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
}

/// Splits `session` and commits, so a swap on it waits on its terminals.
fn split_and_commit(core: &mut Core, session: Uuid) {
    let _ = core.dispatch(&Request {
        cmd: Command::Session(SessionCmd::Split),
        target: Some(wire(session)),
        args: Some(mode("on")),
    });
    let _ = core.commit();
}

/// A window write that fails from now on: the file is replaced by a directory that is not empty.
fn block_window(file: &Path) {
    let _ = std::fs::remove_file(file);
    std::fs::create_dir_all(file.join("blocker")).unwrap();
}

fn lookup_core(core: &mut Core, op_id: &str) -> Response {
    core.dispatch(&Request {
        cmd: Command::Catalog(CatalogCmd::Op),
        target: Some(op_id.to_owned()),
        args: Some(Args::default()),
    })
}

/// The `unknown` of a command dropped while another ran reaches the disk before that other command's receipt, which
/// waits for its windows.
#[test]
fn a_dropped_operations_unknown_is_written_before_the_next_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = persistent_served(dir.path());
    let window = core.library().active_window_id().unwrap();
    let first = core.library().store(window).unwrap().sessions().next().unwrap().id();
    let _ = core.dispatch(&Request {
        cmd: SESSION_NEW,
        target: None,
        args: Some(Args::default()),
    });
    let _ = core.commit();
    let second = core
        .library()
        .store(window)
        .unwrap()
        .sessions()
        .map(crate::model::Session::id)
        .find(|id| *id != first)
        .unwrap();
    split_and_commit(&mut core, first);
    split_and_commit(&mut core, second);
    let epoch = core.remote.respawns.epoch().to_owned();
    let kept = core.begin(&swap_request(first, "kept", epoch.clone()));
    let dropped = core.begin(&swap_request(second, "dropped", epoch));
    assert!(matches!(kept, Dispatched::Pending { .. }));
    assert!(matches!(dropped, Dispatched::Pending { .. }));
    block_window(&file);
    drop(dropped);
    let reply = kept.finish_now(&mut core);
    assert_eq!(
        reply,
        Response::failure("the receipt for operation kept could not be saved; ask catalog.op for it")
    );
    let table = on_disk(dir.path());
    assert!(table.receipt("kept").is_none());
    assert_eq!(
        table.receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );

    std::fs::remove_dir_all(&file).unwrap();
    let answer = lookup_core(&mut core, "kept");
    assert!(answer.ok && receipt_of(&answer).is_some(), "{answer:?}");
    assert_eq!(receipt_of(&answer), on_disk(dir.path()).receipt("kept").cloned());
}

/// A command dropped while the table holds a receipt whose window did not land keeps its `unknown` in memory: a
/// table write then would put that receipt on the disk before its window.
#[test]
fn a_settle_never_writes_a_receipt_whose_window_did_not_land() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = persistent_served(dir.path());
    let window = core.library().active_window_id().unwrap();
    let session = core.library().store(window).unwrap().sessions().next().unwrap().id();
    split_and_commit(&mut core, session);
    block_window(&file);
    let epoch = core.remote.respawns.epoch().to_owned();
    let reply = core.dispatch(&Request {
        cmd: Command::Workspace(WorkspaceCmd::New),
        target: None,
        args: Some(args(|a| {
            a.name = Some("dev".to_owned());
            a.op_id = Some("ws".to_owned());
            a.epoch = Some(epoch.clone());
        })),
    });
    assert_eq!(
        reply,
        Response::failure("the receipt for operation ws could not be saved; ask catalog.op for it")
    );
    drop(core.begin(&swap_request(session, "dropped", epoch)));
    let _ = core.commit();
    let table = dir.path().join(OPERATIONS_FILE_NAME);
    assert!(!table.exists() || on_disk(dir.path()).receipt("ws").is_none());

    std::fs::remove_dir_all(&file).unwrap();
    let answer = lookup_core(&mut core, "ws");
    assert_eq!(
        receipt_of(&answer).map(|r| r.outcome),
        Some(outcome::APPLIED.to_owned()),
        "{answer:?}"
    );
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    let table = on_disk(dir.path());
    assert!(table.receipt("ws").is_some());
    assert_eq!(
        table.receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// A blocked window write leaves `ws` unsaved; the block clears and `dropped` is dropped.
fn unsaved_receipt_then_dropped(dir: &Path) -> (Core, PathBuf) {
    let (mut core, file) = persistent_served(dir);
    let window = core.library().active_window_id().unwrap();
    let session = core.library().store(window).unwrap().sessions().next().unwrap().id();
    split_and_commit(&mut core, session);
    block_window(&file);
    let epoch = core.remote.respawns.epoch().to_owned();
    let reply = core.dispatch(&Request {
        cmd: Command::Workspace(WorkspaceCmd::New),
        target: None,
        args: Some(args(|a| {
            a.name = Some("dev".to_owned());
            a.op_id = Some("ws".to_owned());
            a.epoch = Some(epoch.clone());
        })),
    });
    assert!(!reply.ok, "{reply:?}");
    std::fs::remove_dir_all(&file).unwrap();
    drop(core.begin(&swap_request(session, "dropped", epoch)));
    (core, file)
}

/// A blocked window write leaves `ws` unsaved; the block clears and no operation is dropped.
fn unsaved_receipt(dir: &Path) -> (Core, PathBuf) {
    let (mut core, file) = persistent_served(dir);
    block_window(&file);
    let epoch = core.remote.respawns.epoch().to_owned();
    let reply = core.dispatch(&Request {
        cmd: Command::Workspace(WorkspaceCmd::New),
        target: None,
        args: Some(args(|a| {
            a.name = Some("dev".to_owned());
            a.op_id = Some("ws".to_owned());
            a.epoch = Some(epoch);
        })),
    });
    assert!(!reply.ok, "{reply:?}");
    std::fs::remove_dir_all(&file).unwrap();
    (core, file)
}

/// A table that waited for its windows reaches the disk with the next window write, also when no operation dropped.
#[test]
fn a_commit_after_a_failed_window_write_saves_the_waiting_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = unsaved_receipt(dir.path());
    let _ = core.commit();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
}

/// Once the windows can be written again, a dropped command's `unknown` and the receipt that waited reach the disk
/// with the next window write, not only with a later lookup.
#[test]
fn a_commit_after_a_failed_window_write_saves_the_waiting_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = unsaved_receipt_then_dropped(dir.path());
    let _ = core.commit();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    let table = on_disk(dir.path());
    assert_eq!(
        table.receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
    assert_eq!(
        table.receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// The quit save writes the table that waited for its windows, so a restart never fences an id whose change is on
/// the disk.
#[test]
fn the_quit_save_writes_a_table_that_waited_for_its_windows() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = unsaved_receipt(dir.path());
    assert!(core.save_all());
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );

    let dir = tempfile::tempdir().unwrap();
    let (mut core, _) = unsaved_receipt_then_dropped(dir.path());
    assert!(core.save_all());
    assert_eq!(
        on_disk(dir.path()).receipt("dropped").map(|r| r.outcome.clone()),
        Some(outcome::UNKNOWN.to_owned())
    );
}

/// A table that cannot be written fails the quit save, though its windows land.
#[test]
fn the_quit_save_fails_when_the_waiting_table_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = unsaved_receipt(dir.path());
    block_window(&dir.path().join(OPERATIONS_FILE_NAME));
    assert!(!core.save_all());
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
}

/// A full window write that lands ends the table's wait: the table is then written alone, so a window write that
/// fails after it does not keep the table off the disk.
#[test]
fn a_landed_window_write_lets_the_waiting_table_be_written_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = unsaved_receipt(dir.path());
    assert!(core.save_windows());
    block_window(&file);
    assert!(core.save_waiting_operations());
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
}

/// The quit's own window write ends the table's wait: when the settle writes failed and it lands, the table is written
/// alone, so a window write that fails after it does not fail the quit save or keep the table off the disk.
#[test]
fn the_quit_window_write_that_lands_lets_the_waiting_table_be_written_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, _) = unsaved_receipt(dir.path());
    core.persistence
        .as_mut()
        .unwrap()
        .scripted_full_writes
        .extend([false, false, true, false]);
    assert!(core.save_all());
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
}

/// A table that failed alone, its windows landed, is retried alone on each commit and lands once it can be written:
/// no commit writes the windows again for it.
#[test]
fn a_commit_retries_a_failed_table_without_writing_the_windows_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut core, file) = persistent_served(dir.path());
    let table = dir.path().join(OPERATIONS_FILE_NAME);
    block_window(&table);
    let epoch = core.remote.respawns.epoch().to_owned();
    let reply = core.dispatch(&Request {
        cmd: Command::Workspace(WorkspaceCmd::New),
        target: None,
        args: Some(args(|a| {
            a.name = Some("dev".to_owned());
            a.op_id = Some("ws".to_owned());
            a.epoch = Some(epoch);
        })),
    });
    assert!(!reply.ok, "{reply:?}");
    let _ = core.commit();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"dev\""));
    std::fs::write(&file, "untouched").unwrap();
    let _ = core.commit();
    assert!(table.is_dir());
    std::fs::remove_dir_all(&table).unwrap();
    let _ = core.commit();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "untouched");
    assert_eq!(
        on_disk(dir.path()).receipt("ws").map(|r| r.outcome.clone()),
        Some(outcome::APPLIED.to_owned())
    );
}

/// Upstream's `theTableIsSavedAndRestoredWithTheStore` and the restart's `adopt`: an operation applied in the last run
/// is answered after a restart at revision 0 of the new epoch, and a malformed table starts empty.
#[test]
fn a_restart_answers_the_last_runs_operations_in_its_own_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = Fixture::new(0);
    first.core.serve_catalog(CATALOG, Some(dir.path()));
    let applied = serve(&mut first, SESSION_NEW, None, Args::default(), "before");
    assert!(receipt_of(&applied).unwrap().rev.unwrap() > 0);
    let mut second = Fixture::new(0);
    second.core.serve_catalog(CATALOG, Some(dir.path()));
    let answer = lookup(&mut second, "before", None);
    assert_eq!(
        receipt_of(&answer),
        Some(CatalogReceipt {
            rev: Some(0),
            ..receipt("before", &epoch(&second), outcome::APPLIED)
        })
    );
    std::fs::write(dir.path().join(OPERATIONS_FILE_NAME), r#"{"entries":"nope"}"#).unwrap();
    let mut third = Fixture::new(0);
    third.core.serve_catalog(CATALOG, Some(dir.path()));
    assert!(third.core.catalog.served.as_ref().unwrap().table.entries().is_empty());
}

/// Upstream's `aMacAppRefusesAnOperationIdOnAnyCommand`, `catalogOpRefusesAMissingOrInvalidIdBeforeTheHost` and
/// `catalogOpTakesEpochAndRevTogether`: an app with windows keeps no receipts, and a served catalog checks
/// `catalog.op`'s arguments first.
#[test]
fn an_app_with_windows_refuses_an_operation_id_and_catalog_op_checks_its_arguments() {
    let mut app = Fixture::new(0);
    let reply = serve(&mut app, SESSION_NEW, None, Args::default(), "a");
    assert_eq!(reply, Response::failure(OPERATION_RECEIPTS_ERROR));
    let mut f = served(0);
    let missing = f.send(Command::Catalog(CatalogCmd::Op), None, Args::default());
    assert_eq!(missing, Response::failure("catalog.op requires an operation id"));
    let half = f.send(
        Command::Catalog(CatalogCmd::Op),
        Some("a"),
        args(|a| a.epoch = Some("e1".into())),
    );
    assert_eq!(half, Response::failure("catalog.op takes epoch and rev together"));
    let result: Value = lookup(&mut f, "a", None).result.unwrap();
    assert_eq!(result["receipt"]["outcome"], json!("fenced"));
}
