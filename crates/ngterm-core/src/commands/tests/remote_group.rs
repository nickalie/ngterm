//! Remote group attach receipts and lifecycle (upstream's `ControlServerRemoteGroupTests.swift`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ngterm_proto::command::LiveCmd;
use ngterm_proto::{Args, Command, ControlResult, Request, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::dispatch::Dispatched;
use crate::html::{HtmlHost, HtmlNavigation};
use crate::live::daemon_name;
use crate::model::{NewSession, RemoteBinding, Session};
use crate::remote::page_channel::{ChannelLink, Permits};
use crate::remote::replica_page::{HelperKey, PageHelperLauncher};
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};

const PAGE_ID: &str = "5A1E7C2B-3D4F-4E5A-8B6C-7D8E9F0A1B2C";

#[derive(Default)]
struct Gate {
    calls: Vec<Vec<String>>,
    answers: Vec<Option<RemoteCommandResult>>,
}

#[derive(Default)]
struct Runner {
    gate: Mutex<Gate>,
    changed: Condvar,
}

impl RemoteCommandRunner for Runner {
    fn run(&self, argv: &[String], _: Duration) -> RemoteCommandResult {
        let mut gate = self.gate.lock().unwrap();
        let index = gate.calls.len();
        gate.calls.push(argv.to_vec());
        self.changed.notify_all();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(answer) = gate.answers.get_mut(index).and_then(Option::take) {
                return answer;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "unanswered fake remote call {index}");
            gate = self.changed.wait_timeout(gate, remaining).unwrap().0;
        }
    }
}

impl Runner {
    fn calls(&self) -> Vec<Vec<String>> {
        self.gate.lock().unwrap().calls.clone()
    }

    fn answer(&self, index: usize, answer: RemoteCommandResult) {
        let mut gate = self.gate.lock().unwrap();
        gate.answers.resize_with(index + 1, || None);
        gate.answers[index] = Some(answer);
        self.changed.notify_all();
    }
}

struct Host;
impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

struct Helper;
impl ChannelLink for Helper {
    fn send(&self, _: Vec<u8>) -> bool {
        true
    }
    fn stop(&self) {}
}

/// Records each helper's key, so a test can end the page there.
#[derive(Default)]
struct Launcher(RefCell<Vec<HelperKey>>);
impl PageHelperLauncher for Launcher {
    fn launch(&self, _: &[String], key: HelperKey, _: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        self.0.borrow_mut().push(key);
        Ok(Box::new(Helper))
    }
}

fn result(status: i32, value: &Value) -> RemoteCommandResult {
    RemoteCommandResult {
        status,
        stdout: value.to_string(),
        stderr: String::new(),
    }
}

fn endpoint() -> Value {
    json!({"executable":"/opt/ngterm/bin/ngterm-session","socketDirectory":"/tmp/ngterm-live-group"})
}

fn terminal(id: &str, parent: Option<&str>) -> Value {
    let mut row = json!({"id":id,"name":format!("{id}-name"),"windowID":"w1","windowName":"main",
        "workspaceID":"ws1","workspaceName":"work","cwd":"/repo",
        "panes":[{"pane":"left","daemon":daemon_name(Uuid::from_u128(0xa3))}]});
    if let Some(parent) = parent {
        row["parent"] = json!(parent);
    }
    row
}

fn parent(members: Option<&[&str]>) -> Value {
    let mut row = terminal("P", None);
    row["name"] = json!("build");
    if let Some(members) = members {
        row["companions"] = json!(
            members
                .iter()
                .map(|id| json!({"id":id,"name":member_name(id)}))
                .collect::<Vec<_>>()
        );
    }
    row
}

fn member_name(id: &str) -> String {
    if id == "pg" {
        "report".to_owned()
    } else {
        format!("{id}-name")
    }
}

fn page() -> Value {
    let mut row = terminal("pg", Some("P"));
    row["name"] = json!("report");
    row["content"] = json!("page");
    row["panes"] = json!([]);
    row
}

fn tree(rows: &[Value]) -> RemoteCommandResult {
    result(
        0,
        &json!({"ok":true,"result":{"remote":{"endpoint":endpoint(),"sessions":rows}}}),
    )
}

fn page_attach() -> RemoteCommandResult {
    result(
        0,
        &json!({"ok":true,"result":{"id":"pg","parent":"P","companionPage":{
        "page":PAGE_ID,"token":"tok-1","source":{"file":{"path":"/srv/report.html","grantRoot":"/srv"}},
        "identity":"report.html","chromeless":false,"javascript":true,"navigation":false,"browse":false,
        "persistent":false,"follow":false}}}),
    )
}

struct Group {
    fixture: Fixture,
    runner: Arc<Runner>,
    launcher: Rc<Launcher>,
}

impl Group {
    fn new(answers: Vec<RemoteCommandResult>) -> Self {
        let mut fixture = Fixture::new(0);
        let runner = Arc::new(Runner::default());
        for (index, answer) in answers.into_iter().enumerate() {
            runner.answer(index, answer);
        }
        fixture.core.set_remote_runner(runner.clone());
        fixture.core.set_html_host(Box::new(Host));
        let launcher = Rc::new(Launcher::default());
        fixture.core.set_page_helper_launcher(launcher.clone());
        Self {
            fixture,
            runner,
            launcher,
        }
    }

    fn request(&mut self, cmd: LiveCmd, target: &str, args: Args) -> Response {
        self.fixture.send(Command::Live(cmd), Some(target), args)
    }

    fn attach(&mut self) -> Response {
        self.request(
            LiveCmd::AttachGroup,
            "P",
            args(|a| a.host = Some("buildbox".to_owned())),
        )
    }

    fn parent(&self) -> Uuid {
        self.fixture
            .store()
            .sessions()
            .find(|row| row.remote_host() == Some("buildbox") && row.parent_id().is_none())
            .unwrap()
            .id()
    }

    fn origins(&self, parent: Uuid) -> Vec<String> {
        self.fixture
            .store()
            .companions(parent)
            .into_iter()
            .map(|row| row.remote_presentation().unwrap().binding.remote_session_id.clone())
            .collect()
    }

    fn begin(&mut self) -> Dispatched {
        self.begin_group("P", None)
    }

    fn begin_group(&mut self, target: &str, window: Option<Uuid>) -> Dispatched {
        self.fixture.core.begin(&Request {
            cmd: Command::Live(LiveCmd::AttachGroup),
            target: Some(target.to_owned()),
            args: Some(args(|a| {
                a.host = Some("buildbox".to_owned());
                a.window = window.map(wire);
            })),
        })
    }

    fn attach_root(&mut self) -> Response {
        self.request(
            LiveCmd::AttachGroup,
            "PR",
            args(|a| a.host = Some("buildbox".to_owned())),
        )
    }

    fn no_remote_rows(&self) -> bool {
        let library = self.fixture.core.library();
        library
            .open_ids()
            .into_iter()
            .filter_map(|window| library.store(window))
            .all(|store| store.sessions().all(|row| row.remote_host().is_none()))
    }

    fn until_calls(&mut self, mut dispatch: Dispatched, count: usize) -> Dispatched {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.runner.calls().len() < count {
            assert!(
                Instant::now() < deadline,
                "expected {count} calls, got {:?}",
                self.runner.calls()
            );
            dispatch = match dispatch {
                Dispatched::Pending { resume, .. } => resume(&mut self.fixture.core),
                Dispatched::Done(response) => panic!("completed early: {response:?}"),
            };
            std::thread::yield_now();
        }
        dispatch
    }
}

fn receipt(response: &Response) -> ControlResult {
    serde_json::from_value(response.result.clone().unwrap()).unwrap()
}

#[test]
fn a_group_arrives_in_origin_order_with_only_its_parent_selected() {
    let mut group = Group::new(vec![
        tree(&[
            parent(Some(&["c1", "pg", "c2"])),
            terminal("c1", Some("P")),
            terminal("c2", Some("P")),
            page(),
        ]),
        page_attach(),
    ]);
    let reply = group.attach();
    assert!(reply.ok, "{reply:?}");
    let parent = group.parent();
    assert_eq!(group.origins(parent), ["c1", "pg", "c2"]);
    assert_eq!(group.fixture.selected(), Some(parent));
    let receipt = receipt(&reply).remote_group.unwrap();
    assert!(receipt.membership_known);
    assert_eq!(
        receipt.companions.iter().map(|r| r.origin.as_str()).collect::<Vec<_>>(),
        ["c1", "pg", "c2"]
    );
    assert!(receipt.companions.iter().all(|r| r.id.is_some() && r.error.is_none()));
    let calls = group.runner.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls.iter().filter(|a| a.last().unwrap().contains("live tree")).count(),
        1
    );
    assert!(calls[1].last().unwrap().contains("page-attach"));
}

#[test]
fn a_group_attached_with_no_select_leaves_the_selection_and_the_focus_alone() {
    let mut group = Group::new(vec![tree(&[parent(Some(&["c1"])), terminal("c1", Some("P"))])]);
    let selected = group.fixture.selected();
    assert!(selected.is_some());
    group.fixture.core.clear_pane_focus_requests();
    let reply = group.request(
        LiveCmd::AttachGroup,
        "P",
        args(|a| {
            a.host = Some("buildbox".to_owned());
            a.no_select = Some(true);
        }),
    );
    assert!(reply.ok, "{reply:?}");
    let parent = group.parent();
    assert_eq!(group.origins(parent), ["c1"]);
    assert_eq!(group.fixture.selected(), selected);
    let rows = std::iter::once(parent).chain(group.fixture.store().companions(parent).into_iter().map(Session::id));
    assert!(
        rows.into_iter()
            .all(|row| !group.fixture.core.pane_focus_requested(row))
    );
}

/// A group a viewer shows already, its parent and its terminal companions, attaches again onto those same rows.
#[test]
fn attaching_a_shown_group_again_answers_with_its_rows() {
    let listing = || {
        tree(&[
            parent(Some(&["c1", "c2"])),
            terminal("c1", Some("P")),
            terminal("c2", Some("P")),
        ])
    };
    let mut group = Group::new(vec![listing(), listing()]);
    let first = group.attach();
    assert!(first.ok, "{first:?}");
    let rows = group.fixture.store().sessions().count();

    let again = group.attach();

    assert!(again.ok, "{again:?}");
    assert_eq!(group.fixture.store().sessions().count(), rows, "no second rows");
    let [first, again] = [&first, &again].map(receipt);
    assert_eq!(again.id, first.id);
    let ids = |receipt: &ControlResult| {
        receipt
            .remote_group
            .as_ref()
            .unwrap()
            .companions
            .iter()
            .map(|member| member.id.clone().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&again), ids(&first));
    assert_eq!(group.origins(group.parent()), ["c1", "c2"]);
}

#[test]
fn a_known_empty_group_is_one_row_and_ok() {
    let mut group = Group::new(vec![tree(&[parent(Some(&[]))])]);
    let reply = group.attach();
    assert!(reply.ok, "{reply:?}");
    let receipt = receipt(&reply).remote_group.unwrap();
    assert!(receipt.membership_known && receipt.companions.is_empty());
    assert_eq!(
        group
            .fixture
            .store()
            .sessions()
            .filter(|r| r.remote_host().is_some())
            .count(),
        1
    );
}

#[test]
fn a_companion_target_is_refused_before_any_row_exists() {
    for target in [terminal("c1", Some("P")), page()] {
        let id = target["id"].as_str().unwrap().to_owned();
        let mut group = Group::new(vec![tree(&[parent(Some(&["c1"])), target])]);
        let before = group.fixture.store().sessions().count();
        let reply = group.request(
            LiveCmd::AttachGroup,
            &id,
            args(|a| a.host = Some("buildbox".to_owned())),
        );
        assert_eq!(reply.error, Some(format!("{id} is a companion, not a group's parent")));
        assert_eq!(group.fixture.store().sessions().count(), before);
    }
}

#[test]
fn an_unavailable_member_is_named_and_successful_rows_stay_in_the_failed_receipt() {
    let mut group = Group::new(vec![tree(&[parent(Some(&["c1", "c2"])), terminal("c1", Some("P"))])]);
    let reply = group.attach();
    let row = receipt(&reply).id.unwrap();
    assert!(!reply.ok);
    assert_eq!(
        reply.error,
        Some(format!(
            "kept build ({row}) and 1 companion(s); not attached: c2-name (unavailable on buildbox)"
        ))
    );
    assert_eq!(group.origins(group.parent()), ["c1"]);
    let receipt = receipt(&reply).remote_group.unwrap();
    assert_eq!(receipt.companions[1].error.as_deref(), Some("unavailable on buildbox"));
    assert!(receipt.companions[0].id.is_some());
}

#[test]
fn a_failing_terminal_member_is_named_with_its_reason() {
    let mut child = terminal("c1", Some("P"));
    child["panes"][0]["pane"] = json!("middle");
    let mut group = Group::new(vec![tree(&[parent(Some(&["c1"])), child])]);
    let reply = group.attach();
    let row = receipt(&reply).id.unwrap();
    assert_eq!(
        reply.error,
        Some(format!(
            "kept build ({row}) and 0 companion(s); not attached: c1-name (buildbox reported panes ngterm cannot address)"
        ))
    );
    assert!(group.origins(group.parent()).is_empty());
}

#[test]
fn a_failing_page_is_named_and_later_terminals_stay() {
    let mut group = Group::new(vec![
        tree(&[parent(Some(&["pg", "c1"])), page(), terminal("c1", Some("P"))]),
        result(1, &json!({"ok":false,"error":"no such page"})),
    ]);
    let reply = group.attach();
    let row = receipt(&reply).id.unwrap();
    assert_eq!(
        reply.error,
        Some(format!(
            "kept build ({row}) and 1 companion(s); not attached: report (no such page)"
        ))
    );
    assert_eq!(group.origins(group.parent()), ["c1"]);
}

fn close_during_page(members: &[&str], missing: &str, page_reply: RemoteCommandResult, page_error: &str) {
    let mut group = Group::new(vec![tree(&[parent(Some(members)), page(), terminal("c1", Some("P"))])]);
    let dispatch = group.begin();
    let dispatch = group.until_calls(dispatch, 2);
    let parent = group.parent();
    group.fixture.store_mut().unwrap().close_session(parent);
    group.runner.answer(1, page_reply);
    let reply = dispatch.finish_now(&mut group.fixture.core);
    assert!(!reply.ok);
    assert_eq!(
        reply.error,
        Some(format!(
            "parent row {} was closed; not attached: {missing}",
            wire(parent)
        ))
    );
    assert!(
        group.fixture.store().sessions().all(|r| r.remote_host().is_none()),
        "no orphan remote rows"
    );
    let receipt = receipt(&reply).remote_group.unwrap();
    assert_eq!(
        receipt
            .companions
            .iter()
            .filter_map(|r| r.error.as_deref())
            .collect::<Vec<_>>(),
        if members[0] == "pg" {
            vec![page_error, "parent row was closed"]
        } else {
            vec![page_error]
        }
    );
    if members[0] == "c1" {
        assert!(
            receipt.companions[0].id.is_some(),
            "earlier successful receipt survives close"
        );
    }
}

#[test]
fn a_parent_closed_during_page_wait_stops_later_members() {
    close_during_page(
        &["pg", "c1"],
        "report, c1-name",
        page_attach(),
        "its parent row here was closed",
    );
}

#[test]
fn a_parent_closed_during_the_last_page_wait_is_reported_closed() {
    close_during_page(&["c1", "pg"], "report", page_attach(), "its parent row here was closed");
}

#[test]
fn an_origin_page_refusal_survives_a_parent_closed_during_the_wait() {
    close_during_page(
        &["pg", "c1"],
        "report, c1-name",
        result(1, &json!({"ok":false,"error":"no such page"})),
        "no such page",
    );
}

#[test]
fn a_parent_moved_during_page_wait_takes_later_members_with_it() {
    let mut group = Group::new(vec![tree(&[
        parent(Some(&["pg", "c1"])),
        page(),
        terminal("c1", Some("P")),
    ])]);
    let dispatch = group.begin();
    let dispatch = group.until_calls(dispatch, 2);
    let parent = group.parent();
    let (workspace, _) = group.fixture.add_workspace("other", 0);
    assert_ne!(
        group
            .fixture
            .store()
            .workspace_for_session(parent)
            .map(crate::model::Workspace::id),
        Some(workspace)
    );
    assert!(group.fixture.store_mut().unwrap().move_session(parent, workspace, None));
    group.runner.answer(1, page_attach());
    let reply = dispatch.finish_now(&mut group.fixture.core);
    assert!(reply.ok, "{reply:?}");
    assert_eq!(group.origins(parent), ["pg", "c1"]);
    for row in std::iter::once(parent).chain(
        group
            .fixture
            .store()
            .companions(parent)
            .into_iter()
            .map(crate::model::Session::id),
    ) {
        assert_eq!(
            group
                .fixture
                .store()
                .workspace_for_session(row)
                .map(crate::model::Workspace::id),
            Some(workspace)
        );
    }
}

#[test]
fn an_origin_without_membership_attaches_listed_children_and_marks_unknown() {
    let mut group = Group::new(vec![tree(&[parent(None), terminal("c1", Some("P"))])]);
    let reply = group.attach();
    assert!(reply.ok, "{reply:?}");
    assert!(!receipt(&reply).remote_group.unwrap().membership_known);
    assert_eq!(group.origins(group.parent()), ["c1"]);
}

#[test]
fn the_dispatcher_reaches_group_attach() {
    let mut group = Group::new(vec![tree(&[parent(Some(&["c1"])), terminal("c1", Some("P"))])]);
    let reply = group.attach();
    assert!(reply.ok, "{reply:?}");
    assert_eq!(receipt(&reply).remote_group.unwrap().companions[0].origin, "c1");
}

#[test]
fn group_validation_precedes_remote_discovery() {
    let mut group = Group::new(vec![]);
    for (target, host, error) in [
        (None, None, "live.attach.group requires a host"),
        (None, Some("buildbox"), "live.attach.group requires a remote session"),
        (Some("not plain"), Some("buildbox"), "invalid remote session"),
    ] {
        let reply = group.fixture.send(
            Command::Live(LiveCmd::AttachGroup),
            target,
            args(|a| a.host = host.map(str::to_owned)),
        );
        assert_eq!(reply.error.as_deref(), Some(error));
    }
    assert!(group.runner.calls().is_empty());
}

#[test]
fn group_discovery_and_page_wait_do_not_block_unrelated_requests() {
    let mut group = Group::new(vec![]);
    let dispatch = group.begin();
    let dispatch = group.until_calls(dispatch, 1);
    assert!(matches!(dispatch, Dispatched::Pending { .. }));
    assert!(group.fixture.send(Command::Tree, None, Args::default()).ok);
    group.runner.answer(0, tree(&[parent(Some(&["pg"])), page()]));
    let dispatch = group.until_calls(dispatch, 2);
    assert!(group.fixture.send(Command::Tree, None, Args::default()).ok);
    group.runner.answer(1, page_attach());
    assert!(dispatch.finish_now(&mut group.fixture.core).ok);
}

#[test]
fn individual_terminal_companion_joins_only_one_matching_parent_in_its_window() {
    for (parents, host, other_endpoint, other_window) in [
        (0, "buildbox", false, false),
        (1, "buildbox", false, false),
        (2, "buildbox", false, false),
        (1, "otherbox", false, false),
        (1, "buildbox", true, false),
        (1, "buildbox", false, true),
    ] {
        let mut group = Group::new(vec![]);
        if parents > 0 {
            group.runner.answer(0, tree(&[parent(Some(&[]))]));
            let reply = group.request(LiveCmd::Attach, "P", args(|a| a.host = Some(host.to_owned())));
            assert!(reply.ok, "{reply:?}");
            if parents == 2 {
                let first = Uuid::parse_str(receipt(&reply).id.as_deref().unwrap()).unwrap();
                show_twice(&mut group, first, host);
            }
        }
        let listing_call = usize::from(parents > 0);
        let mut listing = tree(&[terminal("c1", Some("P"))]);
        if other_endpoint {
            let mut value: Value = serde_json::from_str(&listing.stdout).unwrap();
            value["result"]["remote"]["endpoint"]["socketDirectory"] = json!("/different");
            listing.stdout = value.to_string();
        }
        group.runner.answer(listing_call, listing);
        let window = other_window.then(|| group.fixture.core.library_mut().new_window(None));
        let reply = group.request(
            LiveCmd::Attach,
            "c1",
            args(|a| {
                a.host = Some("buildbox".to_owned());
                a.window = window.map(wire);
            }),
        );
        assert!(reply.ok, "{reply:?}");
        let row = Uuid::parse_str(receipt(&reply).id.as_deref().unwrap()).unwrap();
        let child = group
            .fixture
            .core
            .library()
            .store_for_session(row)
            .unwrap()
            .session(row)
            .unwrap();
        assert_eq!(
            child.parent_id().is_some(),
            parents == 1 && host == "buildbox" && !other_endpoint && !other_window
        );
    }
}

/// A second row bound to `row`'s origin session: `live attach` never adds one, so it stands for a row an older build
/// left behind.
fn show_twice(group: &mut Group, row: Uuid, host: &str) {
    let workspace = group.fixture.workspace;
    let store = group.fixture.store_mut().unwrap();
    let binding = &store.session(row).unwrap().remote_presentation().unwrap().binding;
    let twin_binding = RemoteBinding::new(
        binding.remote_session_id.clone(),
        HashMap::new(),
        None,
        binding.origin.clone(),
    );
    let spec = NewSession {
        remote_host: Some(host.to_owned()),
        ..NewSession::at_cwd("/home/u")
    };
    let twin = store.add_session(workspace, spec).unwrap();
    store.bind_remote(twin_binding, twin);
}

#[test]
fn every_attached_group_row_reports_its_origin_id_in_the_tree_and_events() {
    use ngterm_proto::Command;
    use ngterm_proto::command::{EventsCmd, SessionCmd};

    let mut group = Group::new(vec![
        tree(&[parent(Some(&["c1", "pg"])), terminal("c1", Some("P")), page()]),
        page_attach(),
    ]);
    let anchor = group
        .fixture
        .send(Command::Events(EventsCmd::Read), None, Args::default());
    let anchor = anchor.result.unwrap()["events"].clone();
    assert!(group.attach().ok);
    let parent = group.parent();
    let ids: Vec<_> = std::iter::once(parent)
        .chain(group.fixture.store().companions(parent).iter().map(|s| s.id()))
        .collect();
    let tree = group.fixture.send(Command::Tree, None, Args::default());
    let tree = tree.result.unwrap()["tree"].clone();
    let nodes: Vec<_> = tree["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| w["sessions"].as_array().unwrap())
        .collect();
    for (id, origin) in ids.iter().zip(["P", "c1", "pg"]) {
        assert_eq!(
            nodes.iter().find(|s| s["id"] == wire(*id)).unwrap()["remoteSessionID"],
            origin
        );
    }
    assert!(
        nodes
            .iter()
            .filter(|s| s.get("remoteHost").is_none())
            .all(|s| s.get("remoteSessionID").is_none())
    );
    assert!(
        group
            .fixture
            .session(SessionCmd::Close, Some(&wire(ids[1])), Args::default())
            .ok
    );
    let _ = group.fixture.core.commit();
    let events = group.fixture.send(
        Command::Events(EventsCmd::Read),
        None,
        args(|a| {
            a.run = Some(anchor["run"].as_str().unwrap().to_owned());
            a.after = Some(anchor["next"].to_string());
            a.kinds = Some(vec!["remote.opened,remote.closed".to_owned()]);
        }),
    );
    let events = events.result.unwrap()["events"]["items"].as_array().unwrap().clone();
    assert_eq!(
        events
            .iter()
            .map(|e| e["payload"]["remoteSessionID"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["P", "c1", "pg", "c1"]
    );
    assert_eq!(
        events.iter().map(|e| e["kind"].as_str().unwrap()).collect::<Vec<_>>(),
        ["remote.opened", "remote.opened", "remote.opened", "remote.closed"]
    );
}

// MARK: - a group under a top-level page

const ROOT_PAGE_ID: &str = "6B2F8D3C-4E5A-4F6B-9C7D-8E9F0A1B2C3D";

fn page_root(members: &[&str]) -> Value {
    let members: Vec<Value> = members
        .iter()
        .map(|id| json!({"id":id,"name":member_name(id)}))
        .collect();
    json!({"id":"PR","name":"dash","windowID":"w1","windowName":"main","workspaceID":"ws1","workspaceName":"work",
        "cwd":"/srv","content":"page","panes":[],"companions":members})
}

fn page_under(id: &str, parent: &str) -> Value {
    let mut row = page();
    row["id"] = json!(id);
    row["parent"] = json!(parent);
    row
}

fn page_attach_of(id: &str, parent: Option<&str>, page: &str) -> RemoteCommandResult {
    let mut value: Value = serde_json::from_str(&page_attach().stdout).unwrap();
    value["result"]["id"] = json!(id);
    value["result"]["parent"] = parent.map_or(Value::Null, |parent| json!(parent));
    value["result"]["companionPage"]["page"] = json!(page);
    result(0, &value)
}

fn root_attach() -> RemoteCommandResult {
    page_attach_of("PR", None, ROOT_PAGE_ID)
}

/// Upstream's `testAPageRootGroupArrivesAsATopLevelReplicaRowWithItsCompanionsUnderIt`.
#[test]
fn a_page_root_group_arrives_as_a_top_level_replica_row_with_its_companions_under_it() {
    let mut group = Group::new(vec![
        tree(&[
            page_root(&["c1", "pg"]),
            terminal("c1", Some("PR")),
            page_under("pg", "PR"),
        ]),
        root_attach(),
        page_attach_of("pg", Some("PR"), PAGE_ID),
    ]);
    let current = group.fixture.store().current_workspace_id();

    let reply = group.attach_root();

    assert!(reply.ok, "{reply:?}");
    let row = Uuid::parse_str(receipt(&reply).id.as_deref().unwrap()).unwrap();
    let store = group.fixture.store();
    let found = store.session(row).unwrap();
    assert_eq!(found.parent_id(), None);
    assert_eq!(found.content(), crate::model::SessionContent::Page);
    assert_eq!(found.remote_host(), Some("buildbox"));
    assert_eq!(found.display_name(), "dash");
    assert_eq!(
        found.overlay.page().unwrap().replica_of,
        Uuid::parse_str(ROOT_PAGE_ID).ok()
    );
    assert_eq!(
        store.workspace_for_session(row).map(crate::model::Workspace::id),
        current
    );
    assert_eq!(group.fixture.selected(), Some(row));
    assert_bound_to_the_origin(&group, row);
    let receipt = receipt(&reply).remote_group.unwrap();
    assert_eq!(
        receipt.companions.iter().filter(|member| member.id.is_some()).count(),
        2
    );
    let calls = group.runner.calls();
    assert!(calls[1].last().unwrap().contains("page-attach"));
    assert!(calls[1].last().unwrap().contains("PR"));
}

/// The page root's row and its page companion carry the origin's endpoint, which later companions find it by.
fn assert_bound_to_the_origin(group: &Group, row: Uuid) {
    let store = group.fixture.store();
    let found = store.session(row).unwrap();
    let binding = &found.remote_presentation().unwrap().binding;
    assert_eq!(binding.remote_session_id, "PR");
    let endpoint: ngterm_proto::ControlLiveEndpoint = serde_json::from_value(endpoint()).unwrap();
    assert_eq!(
        binding.origin,
        Some(crate::model::RemoteOrigin {
            host: "buildbox".to_owned(),
            endpoint: endpoint.clone(),
            session_name: "dash".to_owned(),
        })
    );
    assert_eq!(binding.daemon(found.pane_identity), None, "a page owns no daemon");
    assert_eq!(group.origins(row), ["c1", "pg"]);
    let page_companion = store.companions(row).last().unwrap().id();
    let page_binding = &store
        .session(page_companion)
        .unwrap()
        .remote_presentation()
        .unwrap()
        .binding;
    assert_eq!(
        page_binding.origin.as_ref().map(|origin| &origin.endpoint),
        Some(&endpoint)
    );
}

/// Upstream's `testAPageRootGroupKeepsTheRootAndNamesAnUnavailableMember`.
#[test]
fn a_page_root_group_keeps_the_root_and_names_an_unavailable_member() {
    let mut group = Group::new(vec![
        tree(&[page_root(&["c1", "c2"]), terminal("c1", Some("PR"))]),
        root_attach(),
    ]);

    let reply = group.attach_root();

    let row = receipt(&reply).id.unwrap();
    assert!(!reply.ok);
    assert_eq!(
        reply.error,
        Some(format!(
            "kept dash ({row}) and 1 companion(s); not attached: c2-name (unavailable on buildbox)"
        ))
    );
    assert_eq!(group.origins(Uuid::parse_str(&row).unwrap()), ["c1"]);
}

/// Upstream's `testAPageRootTheOriginRefusesLeavesNoRow`.
#[test]
fn a_page_root_the_origin_refuses_leaves_no_row() {
    let refused = json!({"ok":false,"error":"a page on this Mac's loopback cannot be shown on another Mac"});
    let mut group = Group::new(vec![
        tree(&[page_root(&["c1"]), terminal("c1", Some("PR"))]),
        result(1, &refused),
    ]);

    let reply = group.attach_root();

    assert_eq!(
        reply.error.as_deref(),
        Some("a page on this Mac's loopback cannot be shown on another Mac")
    );
    assert!(group.no_remote_rows());
}

/// Upstream's `testAPageRootWhoseWorkspaceClosesDuringTheHandoffLeavesNoRow`.
#[test]
fn a_page_root_whose_workspace_closes_during_the_handover_leaves_no_row() {
    let mut group = Group::new(vec![tree(&[page_root(&["c1"]), terminal("c1", Some("PR"))])]);
    group.fixture.add_workspace("other", 1);
    let destination = group.fixture.store().current_workspace_id().unwrap();
    let dispatch = group.begin_group("PR", None);
    let dispatch = group.until_calls(dispatch, 2);
    assert!(
        group
            .fixture
            .store_mut()
            .unwrap()
            .remove_workspace(destination)
            .is_some()
    );
    group.runner.answer(1, root_attach());

    let reply = dispatch.finish_now(&mut group.fixture.core);

    assert_eq!(
        reply.error.as_deref(),
        Some("the window or workspace to attach into was closed")
    );
    assert!(reply.result.is_none());
    assert!(group.no_remote_rows());
    assert_eq!(group.runner.calls().len(), 2, "no member is attempted");
}

/// Upstream's `testAPageRootWhoseWindowClosesDuringTheHandoffLeavesNoRow`.
#[test]
fn a_page_root_whose_window_closes_during_the_handover_leaves_no_row() {
    let mut group = Group::new(vec![tree(&[page_root(&[])])]);
    let destination = group.fixture.core.library_mut().new_window(Some("closing"));
    let dispatch = group.begin_group("PR", Some(destination));
    let dispatch = group.until_calls(dispatch, 2);
    group.fixture.core.library_mut().close_window(destination);
    group.runner.answer(1, root_attach());

    let reply = dispatch.finish_now(&mut group.fixture.core);

    assert_eq!(
        reply.error.as_deref(),
        Some("the window or workspace to attach into was closed")
    );
    assert!(group.no_remote_rows());
    let closed = group.fixture.core.library().store(destination);
    assert!(closed.is_none_or(|store| store.sessions().all(|row| row.remote_host().is_none())));
}

/// Upstream's `testAPageRootEndingThereWhileItsGroupIsParkedFinalizesTheGroup`: Undo would bring back a dead page.
#[test]
fn a_page_root_ending_there_while_its_group_is_parked_finalizes_the_group() {
    let mut group = Group::new(vec![
        tree(&[
            page_root(&["c1", "pg"]),
            terminal("c1", Some("PR")),
            page_under("pg", "PR"),
        ]),
        root_attach(),
        page_attach_of("pg", Some("PR"), PAGE_ID),
        result(0, &json!({"ok":true,"result":{}})),
    ]);
    let reply = group.attach_root();
    assert!(reply.ok, "{reply:?}");
    let root = Uuid::parse_str(receipt(&reply).id.as_deref().unwrap()).unwrap();
    let members: Vec<Uuid> = group
        .fixture
        .store()
        .companions(root)
        .into_iter()
        .map(crate::model::Session::id)
        .collect();
    assert_eq!(members.len(), 2);
    assert!(group.fixture.store_mut().unwrap().soft_close_session(root));
    group.fixture.core.commit();
    assert!(group.fixture.store().pending_close_session(root).is_some());

    let key = group.launcher.0.borrow()[0];
    group.fixture.core.page_helper_closed(key, "exit 0");
    group.fixture.core.commit();
    let deadline = Instant::now() + Duration::from_secs(5);
    while group.fixture.store().pending_close_session(root).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        group.fixture.core.remote_tick(Instant::now() + Duration::from_secs(60));
        group.fixture.core.commit();
    }

    let store = group.fixture.store_mut().unwrap();
    assert!(store.pending_close_session(root).is_none());
    for member in &members {
        assert!(store.pending_close_session(*member).is_none());
    }
    assert!(!store.undo_pending_close(None));
    assert!(store.session(root).is_none());
    assert!(members.iter().all(|member| store.session(*member).is_none()));
}

/// Upstream's `ControlServerRemoteCompanionsTests.testAListedPageRootAttachesAtTopLevelAndItsCompanionsAttachUnderItLater`.
#[test]
fn a_listed_page_root_attaches_at_top_level_and_its_companions_attach_under_it_later() {
    let listing = || {
        tree(&[
            page_root(&["c1", "pg"]),
            terminal("c1", Some("PR")),
            page_under("pg", "PR"),
        ])
    };
    let mut group = Group::new(vec![
        listing(),
        root_attach(),
        listing(),
        listing(),
        page_attach_of("pg", Some("PR"), PAGE_ID),
    ]);
    let current = group.fixture.store().current_workspace_id();
    let attach = |group: &mut Group, session: &str| {
        group.request(LiveCmd::Attach, session, args(|a| a.host = Some("buildbox".to_owned())))
    };

    let attached = attach(&mut group, "PR");

    assert!(attached.ok, "{attached:?}");
    assert_eq!(receipt(&attached).parent, None);
    let row = Uuid::parse_str(receipt(&attached).id.as_deref().unwrap()).unwrap();
    let found = group.fixture.store().session(row).unwrap();
    assert_eq!(found.parent_id(), None);
    assert_eq!(found.content(), crate::model::SessionContent::Page);
    assert_eq!(found.remote_host(), Some("buildbox"));
    assert_eq!(
        group
            .fixture
            .store()
            .workspace_for_session(row)
            .map(crate::model::Workspace::id),
        current
    );

    let terminal = attach(&mut group, "c1");
    let page = attach(&mut group, "pg");

    assert!(terminal.ok, "{terminal:?}");
    assert!(page.ok, "{page:?}");
    assert_eq!(group.origins(row), ["c1", "pg"]);
    let contents: Vec<_> = group
        .fixture
        .store()
        .companions(row)
        .into_iter()
        .map(crate::model::Session::content)
        .collect();
    assert_eq!(
        contents,
        [
            crate::model::SessionContent::Terminal,
            crate::model::SessionContent::Page
        ]
    );
}

/// Upstream's remote page contract: New Companion on a page root's row has the origin make one under that root.
#[test]
fn new_companion_on_a_page_roots_row_makes_one_under_that_root_on_the_origin() {
    const C9: &str = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";
    let created = json!({"ok":true,"result":{"id":C9,"parent":"PR"}});
    let mut group = Group::new(vec![
        tree(&[page_root(&[])]),
        root_attach(),
        tree(&[page_root(&[])]),
        result(0, &created),
        tree(&[page_root(&[C9]), terminal(C9, Some("PR"))]),
    ]);
    let reply = group.attach_root();
    assert!(reply.ok, "{reply:?}");
    let root = receipt(&reply).id.unwrap();

    let response = group.fixture.session(
        ngterm_proto::command::SessionCmd::CompanionNew,
        Some(&root),
        args(|a| a.no_select = Some(true)),
    );

    assert!(response.ok, "{response:?}");
    assert_eq!(group.origins(Uuid::parse_str(&root).unwrap()), [C9]);
    let create = group.runner.calls()[3].last().unwrap().clone();
    for word in ["session", "new", "--parent", "PR"] {
        assert!(create.contains(word), "{word}: {create}");
    }
}
