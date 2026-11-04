//! Remote group attach receipts and lifecycle (upstream's `ControlServerRemoteGroupTests.swift`).

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

struct Launcher;
impl PageHelperLauncher for Launcher {
    fn launch(&self, _: &[String], _: HelperKey, _: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
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
        fixture.core.set_page_helper_launcher(Rc::new(Launcher));
        Self { fixture, runner }
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
        self.fixture.core.begin(&Request {
            cmd: Command::Live(LiveCmd::AttachGroup),
            target: Some("P".to_owned()),
            args: Some(args(|a| a.host = Some("buildbox".to_owned()))),
        })
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
        for index in 0..parents {
            group.runner.answer(index, tree(&[parent(Some(&[]))]));
            let reply = group.request(LiveCmd::Attach, "P", args(|a| a.host = Some(host.to_owned())));
            assert!(reply.ok, "{reply:?}");
        }
        let mut listing = tree(&[terminal("c1", Some("P"))]);
        if other_endpoint {
            let mut value: Value = serde_json::from_str(&listing.stdout).unwrap();
            value["result"]["remote"]["endpoint"]["socketDirectory"] = json!("/different");
            listing.stdout = value.to_string();
        }
        group.runner.answer(parents, listing);
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
