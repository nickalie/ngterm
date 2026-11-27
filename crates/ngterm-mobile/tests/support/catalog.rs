//! A headless machine's list on the phone follows its catalog ([`follows_the_catalog`]): the machine restores two page
//! sessions and a terminal session from its saved state ([`seed_pages`]), since a headless machine cannot open a page
//! itself. The phone lists them with each page's kind and attaches no page. What the machine changes reaches the list
//! with no listing asked for. New Session, Rename Session and End Session go under operation ids, which the machine's
//! receipts record. A session whose live session is lost stays listed as ended, is not attached, and leaves with End
//! Session. A reply lost while the machine is stopped says the outcome is unknown, and the list then shows what became
//! of it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use ngterm_mobile::RemoteSession;
use ngterm_mobile::catalog::CatalogState;

use crate::machine::{World, text};
use crate::phone::{Client, GRID, MACHINE_ID, MACHINE_LABEL, WAIT, block_on, poll};

/// The saved window, its workspaces and its sessions ([`seed_pages`]).
const WINDOW: &str = "11111111-1111-4111-8111-111111111111";
const PAGES_WORKSPACE: &str = "pages";
const WORK_WORKSPACE: &str = "work";
const FILE_PAGE: &str = "33333333-3333-4333-8333-333333333333";
const URL_PAGE: &str = "44444444-4444-4444-8444-444444444444";
const TERMINAL: &str = "66666666-6666-4666-8666-666666666666";
/// The machine's file of operation receipts, in its state dir.
const RECEIPTS: &str = "catalog-operations.json";

/// A row as the test compares it: its name, workspace, whether it is a page, and whether it ended.
type Row = (String, String, bool, bool);

/// Saves, in the private state dir `state`, a window whose first workspace holds a file page session and a URL page
/// session and whose second holds a terminal session, all top level, as a desktop ngterm saves them.
pub(crate) fn seed_pages(state: &Path) {
    let home = state.parent().expect("the state dir's parent").join("home");
    let page = state.join("page.html");
    fs::write(&page, "<p>e2e page</p>\n").expect("write the page");
    let window = serde_json::json!({
        "version": 1,
        "workspaces": [
            {
                "id": "22222222-2222-4222-8222-222222222222",
                "name": PAGES_WORKSPACE,
                "sessions": [
                    page_session(FILE_PAGE, &home, &serde_json::json!({"kind": "file", "path": page})),
                    page_session(URL_PAGE, &home, &serde_json::json!({"kind": "url", "url": "https://example.com/"})),
                ],
            },
            {
                "id": "55555555-5555-4555-8555-555555555555",
                "name": WORK_WORKSPACE,
                "sessions": [{
                    "id": TERMINAL, "cwd": home, "flagged": false, "isSplit": false,
                    "paneIdentity": "77777777-7777-4777-8777-777777777777",
                }],
            },
        ],
    });
    let windows = serde_json::json!({
        "frontmost": WINDOW,
        "version": 1,
        "windows": [{"id": WINDOW, "isOpen": true, "name": "window 1"}],
    });
    fs::create_dir_all(state.join("windows")).expect("create the saved windows' directory");
    fs::write(state.join("windows.json"), windows.to_string()).expect("save the windows");
    fs::write(state.join(format!("windows/{WINDOW}.json")), window.to_string()).expect("save the window");
}

fn page_session(id: &str, home: &Path, source: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": id, "content": "page", "cwd": home, "flagged": false,
        "pages": [{"slot": "session", "source": source}],
    })
}

/// The whole walk through a headless machine's catalog, on a phone connected through the world's proxy.
pub(crate) fn follows_the_catalog(world: &World, phone: &mut Client) {
    the_catalog_lists_pages_and_the_origins_order(world, phone);
    a_page_row_is_never_attached(phone);
    the_list_follows_the_machine_without_a_listing(world, phone);
    // first, so the New Session after it starts a session server in place of the one it kills
    a_session_whose_live_session_is_lost_stays_listed_as_ended(world, phone);
    session_actions_go_under_operation_ids(world, phone);
    a_lost_reply_leaves_the_outcome_unknown(world, phone);
}

/// The first listing over ssh is the private instance's own (its plain listing has no page row), and starts the
/// catalog; once it is connected the rows hold the page sessions with their kinds, in the machine's own order.
fn the_catalog_lists_pages_and_the_origins_order(world: &World, phone: &mut Client) {
    let listed: Vec<String> = phone.list().into_iter().map(|(id, _)| id).collect();
    assert_eq!(listed, [TERMINAL], "the listing over ssh is not the private instance's");
    let private: Vec<String> = world.headless.tree().into_iter().map(|(id, _)| id).collect();
    assert!(private.iter().any(|id| id == TERMINAL), "{private:?}");
    phone.wait("the catalog connected", |seen| {
        seen.catalog.last() == Some(&CatalogState::Connected)
    });
    let rows = follow_the_origin(world, phone);
    assert_eq!(kind(&rows, FILE_PAGE), Some("file"), "{rows:?}");
    assert_eq!(kind(&rows, URL_PAGE), Some("url"), "{rows:?}");
    assert_eq!(kind(&rows, TERMINAL), None, "{rows:?}");
    assert!(rows.iter().all(|row| !row.ended), "{rows:?}");
}

fn a_page_row_is_never_attached(phone: &mut Client) {
    let rows = last_rows(phone);
    let page = rows
        .iter()
        .find(|row| row.id == FILE_PAGE)
        .expect("the file page's row");
    let refused = block_on(phone.core.attach_session(MACHINE_ID.to_owned(), FILE_PAGE.to_owned()));
    let expected = format!(
        "{} is a page on {MACHINE_LABEL}; this app shows terminal sessions only",
        page.name
    );
    assert_eq!(refused.map_err(|error| error.to_string()), Err(expected));
}

/// What the machine itself changes reaches the phone's list with no listing asked for: a rename, a new session, a
/// session moved to a new workspace, the workspaces' order and a closed session.
fn the_list_follows_the_machine_without_a_listing(world: &World, phone: &mut Client) {
    let ctl = |arguments: &[&str]| world.headless.ctl(arguments).expect("ngtermctl on the machine");
    ctl(&["session", "rename", "e2e-renamed", "--target", TERMINAL]);
    phone.wait_rows("the rename", |rows| named(rows, "e2e-renamed").is_some());
    let made = ctl(&["session", "new", "--no-select", "--name=e2e-live"])
        .trim()
        .to_owned();
    phone.wait_rows("the new session", |rows| named(rows, "e2e-live").is_some());
    let workspace = ctl(&["workspace", "new", "e2e-moved"]).trim().to_owned();
    ctl(&["session", "move", &workspace, "--target", &made]);
    phone.wait_rows("the moved session", |rows| {
        named(rows, "e2e-live").is_some_and(|row| row.workspace_name == "e2e-moved")
    });
    ctl(&["workspace", "move", "--to", "top", "--target", &workspace]);
    phone.wait_rows("the workspace first", |rows| {
        rows.first().is_some_and(|row| row.workspace_name == "e2e-moved")
    });
    follow_the_origin(world, phone);
    ctl(&["session", "close", "--target", &made]);
    phone.wait_rows("the closed session gone", |rows| named(rows, "e2e-live").is_none());
    follow_the_origin(world, phone);
}

/// New Session, Rename Session (to a name that starts with `-`) and End Session from the phone each reach the machine
/// and the list, and each leaves the machine an applied receipt under an operation id of its own.
fn session_actions_go_under_operation_ids(world: &World, phone: &mut Client) {
    let before = receipts(world).len();
    let made = block_on(
        phone
            .core
            .create_session(MACHINE_ID.to_owned(), Some("e2e-op".to_owned())),
    )
    .expect("New Session");
    assert_eq!(made.name, "e2e-op");
    phone.wait_rows("the phone's new session", |rows| named(rows, "e2e-op").is_some());
    the_new_session_shows_its_pane(phone, &made.id);

    block_on(
        phone
            .core
            .rename_session(MACHINE_ID.to_owned(), made.id.clone(), "-e2e-dashed".to_owned()),
    )
    .expect("Rename Session");
    phone.wait_rows("the phone's rename", |rows| named(rows, "-e2e-dashed").is_some());
    assert!(
        world
            .headless
            .tree()
            .iter()
            .any(|(id, name)| *id == made.id && name == "-e2e-dashed"),
        "{:?}",
        world.headless.tree()
    );

    block_on(phone.core.end_session(MACHINE_ID.to_owned(), made.id.clone())).expect("End Session");
    phone.wait_rows("the phone's ended session gone", |rows| {
        rows.iter().all(|row| row.id != made.id)
    });
    assert!(world.headless.tree().iter().all(|(id, _)| *id != made.id));
    follow_the_origin(world, phone);

    let receipts = receipts(world);
    let fresh = &receipts[before..];
    assert_eq!(fresh.len(), 3, "one receipt for each action: {receipts:?}");
    assert!(fresh.iter().all(|(_, outcome)| outcome == "applied"), "{receipts:?}");
    let mut ids: Vec<&String> = fresh.iter().map(|(id, _)| id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "an operation id was used twice: {receipts:?}");
}

/// The session New Session answered with attaches and shows its pane, as the app opens it right after.
fn the_new_session_shows_its_pane(phone: &mut Client, session: &str) {
    phone.seen.layout = None;
    block_on(phone.core.attach_session(MACHINE_ID.to_owned(), session.to_owned())).expect("attach the new session");
    let layout = phone.wait_layout();
    let shown = block_on(
        phone
            .core
            .show_pane(MACHINE_ID.to_owned(), session.to_owned(), layout.primary, GRID),
    );
    assert!(shown.is_ok(), "the new session's pane: {:?}", shown.err());
    phone
        .core
        .detach_session(MACHINE_ID.to_owned(), session.to_owned())
        .expect("detach the new session");
}

/// New Session while the machine is stopped, with the connection lost before its reply: the phone says the outcome
/// is unknown. Once the machine runs again and the phone reconnects, the list shows what became of it.
fn a_lost_reply_leaves_the_outcome_unknown(world: &World, phone: &mut Client) {
    phone.drain();
    phone.seen.states.clear();
    phone.seen.catalog.clear();
    let core = phone.core.clone();
    let stopped = Stopped::new(world.headless.child.id());
    let creating =
        thread::spawn(move || block_on(core.create_session(MACHINE_ID.to_owned(), Some("e2e-lost".to_owned()))));
    let before = receipts(world).len();
    let state = world.headless.private.state.clone();
    let command = poll("New Session in flight on the machine", || in_flight(&state));
    poll("New Session's request queued at the stopped machine's socket", || {
        request_queued(command).then_some(())
    });
    world.proxy.sever();
    let answer = creating.join().expect("the New Session thread");
    let expected = format!(
        "{MACHINE_LABEL} did not answer New Session; its outcome is unknown until {MACHINE_LABEL} can be asked"
    );
    assert_eq!(
        answer.map(|session| session.id).map_err(|error| error.to_string()),
        Err(expected)
    );
    drop(stopped);

    phone.wait("the machine connected again", |seen| {
        seen.states.last() == Some(&ngterm_mobile::MachineState::Connected)
    });
    phone.wait("the catalog connected again", |seen| {
        seen.catalog.last() == Some(&CatalogState::Connected)
    });
    let applied = poll("the machine applied the lost New Session", || {
        let receipts = receipts(world);
        (receipts.len() > before).then_some(receipts)
    });
    assert_eq!(applied.len(), before + 1, "{applied:?}");
    assert_eq!(applied.last().map(|(_, outcome)| outcome.as_str()), Some("applied"));
    assert!(
        origin_rows(world).iter().any(|(name, ..)| name == "e2e-lost"),
        "the machine has no e2e-lost session"
    );
    follow_the_origin(world, phone);
}

/// The machine's session server dies with its live sessions: each terminal session stays listed, marked ended, while
/// the machine's plain listing drops it. The phone does not attach it, and End Session takes it away.
fn a_session_whose_live_session_is_lost_stays_listed_as_ended(world: &World, phone: &mut Client) {
    let server = session_server(world).expect("the machine's session server");
    signal("-KILL", server);
    let rows = phone.wait_rows("the ended terminal session", |rows| {
        rows.iter().any(|row| row.id == TERMINAL && row.ended)
    });
    assert!(
        rows.iter().filter(|row| row.page.is_none()).all(|row| row.ended),
        "a terminal session outlived its session server: {rows:?}"
    );
    assert!(
        world.headless.tree().iter().all(|(id, _)| id != TERMINAL),
        "the plain listing still lists the ended session"
    );
    follow_the_origin(world, phone);

    let refused = block_on(phone.core.attach_session(MACHINE_ID.to_owned(), TERMINAL.to_owned()));
    assert_eq!(
        refused.map_err(|error| error.to_string()),
        Err(format!("the session has ended on {MACHINE_LABEL}"))
    );

    let before = receipts(world).len();
    block_on(phone.core.end_session(MACHINE_ID.to_owned(), TERMINAL.to_owned())).expect("End Session");
    phone.wait_rows("the ended session gone", |rows| {
        rows.iter().all(|row| row.id != TERMINAL)
    });
    let receipts = receipts(world);
    assert_eq!(receipts.len(), before + 1, "{receipts:?}");
    assert_eq!(receipts.last().map(|(_, outcome)| outcome.as_str()), Some("applied"));
    follow_the_origin(world, phone);
}

impl Client {
    /// Takes in events until the last listing's rows satisfy `done`; answers them.
    fn wait_rows(&mut self, what: &str, done: impl Fn(&[RemoteSession]) -> bool) -> Vec<RemoteSession> {
        self.wait(what, |seen| seen.trees.last().is_some_and(|rows| done(rows)));
        last_rows(self)
    }
}

/// Waits until the phone's rows are the machine's own sessions, in its order, with its page and ended marks; answers
/// the rows.
fn follow_the_origin(world: &World, phone: &mut Client) -> Vec<RemoteSession> {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        phone.settle(std::time::Duration::from_millis(200));
        let origin = origin_rows(world);
        let rows = phone.seen.trees.last().cloned().unwrap_or_default();
        if compared(&rows) == origin {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the phone's rows never followed the machine within {WAIT:?}:\nphone  {:?}\nmachine {origin:?}",
            compared(&rows)
        );
    }
}

fn last_rows(phone: &Client) -> Vec<RemoteSession> {
    phone.seen.trees.last().cloned().unwrap_or_default()
}

fn compared(rows: &[RemoteSession]) -> Vec<Row> {
    rows.iter()
        .map(|row| {
            (
                row.name.clone(),
                row.workspace_name.clone(),
                row.page.is_some(),
                row.ended,
            )
        })
        .collect()
}

/// The machine's own top-level sessions (`ngtermctl tree --json`), in its order.
fn origin_rows(world: &World) -> Vec<Row> {
    let output = world.headless.ctl(&["tree", "--json"]).expect("the machine's tree");
    let reply: serde_json::Value = serde_json::from_str(&output).expect("the tree's json");
    let workspaces = reply["result"]["tree"]["workspaces"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    workspaces
        .iter()
        .flat_map(|workspace| {
            let name = text(&workspace["name"]);
            workspace["sessions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|session| session["parent"].is_null())
                .map(move |session| {
                    (
                        text(&session["name"]),
                        name.clone(),
                        session["content"] == "page",
                        session["ended"] == true,
                    )
                })
        })
        .collect()
}

fn kind<'a>(rows: &'a [RemoteSession], id: &str) -> Option<&'a str> {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("no row {id}: {rows:?}"))
        .page
        .as_deref()
}

fn named<'a>(rows: &'a [RemoteSession], name: &str) -> Option<&'a RemoteSession> {
    rows.iter().find(|row| row.name == name)
}

/// The machine's operation receipts, oldest first, as (operation id, outcome).
fn receipts(world: &World) -> Vec<(String, String)> {
    let Ok(saved) = fs::read_to_string(world.headless.private.state.join(RECEIPTS)) else {
        return Vec::new();
    };
    let table: serde_json::Value = serde_json::from_str(&saved).expect("the receipts' json");
    table["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|entry| (text(&entry["opID"]), text(&entry["outcome"])))
        .collect()
}

/// The machine's process, stopped until dropped: it takes commands into its socket and answers none.
struct Stopped(u32);

impl Stopped {
    fn new(pid: u32) -> Self {
        signal("-STOP", pid);
        Self(pid)
    }
}

impl Drop for Stopped {
    fn drop(&mut self) {
        signal("-CONT", self.0);
    }
}

fn signal(name: &str, pid: u32) {
    let sent = Command::new("kill").args([name, &pid.to_string()]).status();
    assert!(
        sent.as_ref().is_ok_and(std::process::ExitStatus::success),
        "kill {name} {pid}: {sent:?}"
    );
}

/// Every process's pid and argv.
fn processes() -> Vec<(u32, Vec<String>)> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let argv = fs::read(entry.path().join("cmdline")).ok()?;
            let words = argv
                .split(|byte| *byte == 0)
                .filter(|word| !word.is_empty())
                .map(|word| String::from_utf8_lossy(word).into_owned())
                .collect();
            Some((pid, words))
        })
        .collect()
}

/// The pid of an `ngtermctl session new` under an operation id that runs against the instance of `state`.
fn in_flight(state: &Path) -> Option<u32> {
    let ours = format!("NGTERM_STATE_DIR={}", state.display());
    processes().into_iter().find_map(|(pid, words)| {
        let program = words.first().map(PathBuf::from);
        (program.is_some_and(|program| program.file_name().is_some_and(|name| name == "ngtermctl"))
            && words.iter().any(|word| word == "--op-id")
            && words.iter().any(|word| word == "new")
            && fs::read(format!("/proc/{pid}/environ"))
                .is_ok_and(|environ| environ.split(|byte| *byte == 0).any(|entry| entry == ours.as_bytes())))
        .then_some(pid)
    })
}

/// Whether a connected Unix socket of `pid` has bytes its peer has not read (`ss`'s Send-Q): the request waits in
/// the stopped machine's socket.
fn request_queued(pid: u32) -> bool {
    let output = Command::new("ss").args(["-xpH"]).output().expect("run ss (iproute2)");
    assert!(
        output.status.success(),
        "ss -xpH failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let owner = format!("pid={pid},");
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let columns: Vec<&str> = line.split_whitespace().collect();
        line.contains(&owner)
            && columns.get(1) == Some(&"ESTAB")
            && columns
                .get(3)
                .and_then(|queued| queued.parse::<u64>().ok())
                .is_some_and(|queued| queued > 0)
    })
}

/// The session server of the instance's private namespace (`ngterm-session server NAMESPACE`).
fn session_server(world: &World) -> Option<u32> {
    let program = world.headless.private.stage.join("bin/ngterm-session");
    let namespace = ngterm_core::live::socket_directory(&world.headless.private.state);
    processes().into_iter().find_map(|(pid, words)| {
        (words.len() >= 3 && Path::new(&words[0]) == program && words[1] == "server" && words[2] == namespace)
            .then_some(pid)
    })
}
