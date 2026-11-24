//! Operation receipts on a served catalog (upstream's `ServerActionsReceiptsTests` and the dispatcher cases of
//! `CatalogOperationsTests`): a structural command sent with an `opID` runs once and answers its receipt, a repeat
//! answers the same, a lookup fences an unseen id, and nothing only memory holds is handed out.

use ngterm_proto::command::{CatalogCmd, SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, CatalogReceipt, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::catalog_served::served;
use super::overlay_page::fixture_with_host;
use super::{Fixture, answered_id, args, wire};
use crate::catalog::OPERATIONS_FILE_NAME;
use crate::catalog::fixture::CATALOG;
use crate::catalog::operations::{CatalogOperationTable, CatalogPosition, outcome, receipt};
use crate::dispatch::OPERATION_RECEIPTS_ERROR;

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
