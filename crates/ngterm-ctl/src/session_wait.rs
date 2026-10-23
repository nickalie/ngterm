//! `ngtermctl session wait` (`agtermctlKit/Session+Wait.swift` `Session.Wait`): a client-side composite over
//! `events.read`, `window.list` and `tree` that blocks until a session's agent status is one of the `--until` values.
//! There is no `session.wait` command: a wait inside the app would hold the accept loop the awaited status writes go
//! through. The event cursor is anchored before the tree is read, so a status set between the two reads is replayed;
//! `--target` resolves here by the server's own rule (`ngterm_core::resolve`). A socket call has no receive timeout,
//! so `--timeout` is enforced by a [`Watchdog`] thread that exits 124 even while a send is blocked.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ngterm_core::error::Noun;
use ngterm_core::model::AgentStatus;
use ngterm_core::pane_env::uuid_string;
use ngterm_core::paths::Env;
use ngterm_core::persist::parse_uuid_string;
use ngterm_core::resolve::resolve_id;
use ngterm_proto::command::WindowCmd;
use ngterm_proto::tree::{ControlSessionNode, ControlTree};
use ngterm_proto::{Args, Command, ControlEvent, ControlWindowNode, EventKind, Request, Response};
use uuid::Uuid;

use crate::client::{self, ClientError};
use crate::events::{StreamState, typed};
use crate::{BasicOptions, Failure, RequestCommand, TargetOptions};

/// The exit status when `--timeout` passes first.
const TIMEOUT_EXIT_CODE: u8 = 124;
/// The exit status when the session leaves the open tree.
const CLOSED_EXIT_CODE: u8 = 2;
/// The wait before reading again after an empty page.
const POLL_DELAY: Duration = Duration::from_millis(250);
/// The kinds the composite reads, in [`EventKind`]'s declaration order.
const KINDS: [EventKind; 2] = [EventKind::Status, EventKind::SessionClosed];

/// How a wait ended (upstream's `SessionWaitOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The session's status is in the `--until` set.
    Matched { id: Uuid, status: AgentStatus },
    /// The session left the open tree: closed, soft-closed or its window closed.
    Closed,
    /// The deadline passed first.
    TimedOut,
}

/// The wait's side effects, injectable so the loop is testable (upstream's `SessionWaitDependencies`).
trait WaitDependencies {
    /// Sends one request and returns its reply.
    fn send(&mut self, request: &Request) -> Result<Response, ClientError>;
    fn sleep(&mut self, delay: Duration);
    /// Monotonic time since an arbitrary start, so a wall-clock change cannot stretch or cut the timeout.
    fn now(&mut self) -> Duration;
}

/// The socket, the thread's sleep and a monotonic clock.
struct LiveWait {
    socket: PathBuf,
    start: Instant,
}

impl WaitDependencies for LiveWait {
    fn send(&mut self, request: &Request) -> Result<Response, ClientError> {
        Ok(client::send(&self.socket, request)?.response)
    }

    fn sleep(&mut self, delay: Duration) {
        std::thread::sleep(delay);
    }

    fn now(&mut self) -> Duration {
        self.start.elapsed()
    }
}

/// Ends a wait at its deadline even while a socket call is blocked, which the poll loop's own deadline check cannot
/// reach: the client has no receive timeout and the app may never answer (upstream's `SessionWaitWatchdog`). A
/// dedicated thread, so nothing else can starve it.
struct Watchdog {
    /// Dropping it wakes the thread without firing.
    cancel: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Runs `fire` once `after` has passed, unless [`Self::cancel`] comes first.
    fn start(after: Duration, fire: impl FnOnce() + Send + 'static) -> Self {
        let (cancel, cancelled) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            if matches!(cancelled.recv_timeout(after), Err(RecvTimeoutError::Timeout)) {
                fire();
            }
        });
        Self {
            cancel: Some(cancel),
            thread: Some(thread),
        }
    }

    /// Stops the timer; once this returns, `fire` has not run and never will, or has already finished.
    fn cancel(mut self) {
        drop(self.cancel.take());
        if let Some(thread) = self.thread.take() {
            // the thread only waits and fires; a panic in `fire` is the caller's and already reported
            let _ = thread.join();
        }
    }
}

/// `ngtermctl session wait --until STATUS [--until STATUS ...] [--timeout SECONDS] [--target T] [--json]`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Block until a session's agent status is one of the given values.",
    long_about = "session wait --target ID --until completed --until blocked --timeout 600\n\nReturns at once when \
                  the session's current status is already in the set, otherwise on the first later status event for \
                  it that is. A status set before the wait started still matches, so after reusing a session wait for \
                  active first, or the previous run's completed answers immediately. A completed that auto-reset \
                  cleared before the wait started is gone.\n\nPrints the matched status, or {\"id\":\"…\",\"status\":\
                  \"…\"} with --json. Exit 0 on a match, 2 when the session leaves the open tree (closed, soft-closed \
                  or its window closed), 124 when --timeout passes first, 1 on any other failure."
)]
pub struct Wait {
    #[arg(
        long,
        action = clap::ArgAction::Append,
        help = "Status to wait for (idle, active, completed, blocked); repeat for several."
    )]
    until: Vec<String>,

    #[arg(
        long,
        allow_negative_numbers = true,
        help = "Seconds to wait before exiting 124; omit to wait indefinitely."
    )]
    timeout: Option<f64>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: BasicOptions,
}

impl Wait {
    /// The `--until` set; validation has refused any value that is not a status.
    fn statuses(&self) -> Vec<AgentStatus> {
        self.until
            .iter()
            .filter_map(|raw| AgentStatus::from_wire(raw))
            .collect()
    }

    /// `--timeout` as a duration; `None` without one, or for one too long to represent, which never passes.
    fn timeout(&self) -> Option<Duration> {
        self.timeout
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
    }

    /// Anchors the event cursor, then reads the current status, then replays events from the anchor, so a status set
    /// between the two reads is still seen. An error is the text of a local failure.
    fn wait(&self, dependencies: &mut dyn WaitDependencies) -> Result<Outcome, String> {
        let statuses = self.statuses();
        let deadline = self
            .timeout()
            .and_then(|timeout| dependencies.now().checked_add(timeout));
        let mut stream = StreamState::new(None, Some(&KINDS), None);
        let anchor = dependencies
            .send(&stream.make_request())
            .map_err(|error| error.message)?;
        stream.consume(typed(anchor).map_err(|error| error.message)?)?;
        let (id, current) = self.resolve_session(dependencies)?;
        if statuses.contains(&current) {
            return Ok(Outcome::Matched { id, status: current });
        }
        loop {
            if deadline.is_some_and(|deadline| dependencies.now() >= deadline) {
                return Ok(Outcome::TimedOut);
            }
            let response = match dependencies.send(&stream.make_request()) {
                Ok(response) => response,
                Err(error) => {
                    let delay = stream.retry_delay(&error).ok_or(error.message)?;
                    pause(dependencies, deadline, delay);
                    continue;
                }
            };
            let events = stream.consume(typed(response).map_err(|error| error.message)?)?;
            if let Some(outcome) = first_outcome(&events, id, &statuses) {
                return Ok(outcome);
            }
            if events.is_empty() {
                pause(dependencies, deadline, POLL_DELAY);
            }
        }
    }

    /// Resolves `--target` the way the server does, across every open window's tree: `active` is the frontmost
    /// window's selected session, an id or unique prefix matches in any open window. An absent status reads as idle.
    fn resolve_session(&self, dependencies: &mut dyn WaitDependencies) -> Result<(Uuid, AgentStatus), String> {
        let windows = read_windows(dependencies)?;
        let mut sessions: Vec<(Uuid, ControlSessionNode)> = Vec::new();
        let mut active = None;
        for window in windows.iter().filter(|window| window.open) {
            let nodes: Vec<(Uuid, ControlSessionNode)> = read_tree(dependencies, &window.id)?
                .workspaces
                .into_iter()
                .flat_map(|workspace| workspace.sessions)
                .filter_map(|node| parse_uuid_string(&node.id).map(|id| (id, node)))
                .collect();
            if window.active {
                active = nodes.iter().find(|(_, node)| node.active).map(|(id, _)| *id);
            }
            sessions.extend(nodes);
        }
        let target = self.target.target.as_str();
        let candidates: Vec<Uuid> = sessions.iter().map(|(id, _)| *id).collect();
        let id = resolve_id(Noun::Session, target, &candidates, active).map_err(|error| error.to_string())?;
        let status = sessions
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .and_then(|(_, node)| node.status.as_deref())
            .and_then(AgentStatus::from_wire)
            .unwrap_or_default();
        Ok((id, status))
    }

    /// The matched status as a word, or with `--json` one compact `{"id","status"}` object with sorted keys (a
    /// deliberate exception to printing the server's line: there is no single line to print).
    fn output(&self, id: Uuid, status: AgentStatus) -> Result<String, Failure> {
        if !self.options.json {
            return Ok(status.wire_name().to_owned());
        }
        let object = BTreeMap::from([("id", uuid_string(id)), ("status", status.wire_name().to_owned())]);
        serde_json::to_string(&object).map_err(|error| Failure::Local(format!("could not encode result: {error}")))
    }
}

/// Sleeps `delay`, clipped to what is left before `deadline`.
fn pause(dependencies: &mut dyn WaitDependencies, deadline: Option<Duration>, delay: Duration) {
    let delay = match deadline {
        Some(deadline) => delay.min(deadline.saturating_sub(dependencies.now())),
        None => delay,
    };
    dependencies.sleep(delay);
}

/// What the first of `events` about session `id` ends the wait with: its close, or a status in `statuses` (a repeat
/// of the current one included). Other sessions' events are skipped.
fn first_outcome(events: &[ControlEvent], id: Uuid, statuses: &[AgentStatus]) -> Option<Outcome> {
    events
        .iter()
        .filter(|event| event.session.as_deref().and_then(parse_uuid_string) == Some(id))
        .find_map(|event| match event.kind {
            EventKind::SessionClosed => Some(Outcome::Closed),
            _ => event
                .payload
                .status
                .as_deref()
                .and_then(AgentStatus::from_wire)
                .filter(|status| statuses.contains(status))
                .map(|status| Outcome::Matched { id, status }),
        })
}

/// `window.list`'s windows.
fn read_windows(dependencies: &mut dyn WaitDependencies) -> Result<Vec<ControlWindowNode>, String> {
    let reply = dependencies
        .send(&Request::new(Command::Window(WindowCmd::List)))
        .map_err(|error| error.message)?;
    let windows = reply
        .ok
        .then(|| reply.result.as_ref().and_then(|result| result.get("windows")))
        .flatten()
        .and_then(|windows| serde_json::from_value(windows.clone()).ok());
    windows.ok_or_else(|| {
        reply
            .error
            .unwrap_or_else(|| "window.list response missing windows".to_owned())
    })
}

/// `tree --window <window>`'s tree.
fn read_tree(dependencies: &mut dyn WaitDependencies, window: &str) -> Result<ControlTree, String> {
    let request = Request {
        args: Some(Args {
            window: Some(window.to_owned()),
            ..Args::default()
        }),
        ..Request::new(Command::Tree)
    };
    let reply = dependencies.send(&request).map_err(|error| error.message)?;
    let tree = reply
        .ok
        .then(|| reply.result.as_ref().and_then(|result| result.get("tree")))
        .flatten()
        .and_then(|tree| serde_json::from_value(tree.clone()).ok());
    tree.ok_or_else(|| reply.error.unwrap_or_else(|| "tree response missing tree".to_owned()))
}

impl RequestCommand for Wait {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    /// The composite's first read: the `events.read` that anchors the cursor before `window.list` and `tree` read the
    /// current status.
    fn make_request(&self) -> Request {
        StreamState::new(None, Some(&KINDS), None).make_request()
    }

    fn validate(&self) -> Result<(), String> {
        if self.until.is_empty() {
            return Err("provide at least one --until status".to_owned());
        }
        if let Some(raw) = self.until.iter().find(|raw| AgentStatus::from_wire(raw).is_none()) {
            return Err(format!(
                "invalid status: {raw} (expected idle, active, completed or blocked)"
            ));
        }
        if self
            .timeout
            .is_some_and(|seconds| !(seconds.is_finite() && seconds > 0.0))
        {
            return Err("--timeout must be a positive number of seconds".to_owned());
        }
        Ok(())
    }

    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        let watchdog = self.timeout().map(|timeout| {
            Watchdog::start(timeout, || {
                report_timeout();
                std::process::exit(i32::from(TIMEOUT_EXIT_CODE));
            })
        });
        let mut live = LiveWait {
            socket: self.options.socket_path(env),
            start: Instant::now(),
        };
        let outcome = self.wait(&mut live);
        // before anything prints, so a deadline passing now cannot add `timed out` under a match
        if let Some(watchdog) = watchdog {
            watchdog.cancel();
        }
        match outcome.map_err(Failure::Local)? {
            Outcome::Matched { id, status } => {
                println!("{}", self.output(id, status)?);
                Ok(())
            }
            Outcome::Closed => {
                eprintln!("session closed");
                Err(Failure::Exit(ExitCode::from(CLOSED_EXIT_CODE)))
            }
            Outcome::TimedOut => {
                report_timeout();
                Err(Failure::Exit(ExitCode::from(TIMEOUT_EXIT_CODE)))
            }
        }
    }
}

fn report_timeout() {
    eprintln!("timed out");
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::mpsc;

    use clap::Parser;
    use ngterm_proto::{EventBatch, EventPayload};
    use serde_json::{Value, json};

    use super::*;
    use crate::events::FIRST_RETRY_DELAY;

    const RUN: &str = "CBB5E3D0-7A9B-4C96-9EA2-18B14380DDB1";
    const WORKER: &str = "AAAA1111-0000-4000-8000-000000000001";
    const OTHER: &str = "BBBB2222-0000-4000-8000-000000000002";
    const WINDOW: &str = "CCCC3333-0000-4000-8000-000000000003";
    const BACKGROUND: &str = "DDDD4444-0000-4000-8000-000000000004";

    fn id(text: &str) -> Uuid {
        parse_uuid_string(text).unwrap()
    }

    /// The app as the composite sees it: canned windows and trees, and the event pages it returns in turn (an empty
    /// page once they run out); every request, every sleep and a clock the sleeps advance.
    struct FakeApp {
        event_pages: VecDeque<Result<Response, ClientError>>,
        trees: HashMap<String, Value>,
        windows: Vec<Value>,
        requests: Vec<Value>,
        sleeps: Vec<Duration>,
        clock: Duration,
    }

    impl FakeApp {
        /// One open, active window: the worker with `worker` as its status, and another session already completed;
        /// `selected` is the window's selected session.
        fn new(worker: Option<&str>, selected: Option<&str>) -> Self {
            let sessions = vec![
                session(WORKER, worker, selected == Some(WORKER)),
                session(OTHER, Some("completed"), selected == Some(OTHER)),
            ];
            Self {
                event_pages: VecDeque::new(),
                trees: HashMap::from([(WINDOW.to_owned(), tree(&sessions))]),
                windows: vec![window(WINDOW, true, true)],
                requests: Vec::new(),
                sleeps: Vec::new(),
                clock: Duration::ZERO,
            }
        }

        fn cmds(&self) -> Vec<&str> {
            self.requests
                .iter()
                .map(|request| request["cmd"].as_str().unwrap())
                .collect()
        }
    }

    impl WaitDependencies for FakeApp {
        fn send(&mut self, request: &Request) -> Result<Response, ClientError> {
            let wire = serde_json::to_value(request).unwrap();
            self.requests.push(wire.clone());
            match wire["cmd"].as_str().unwrap() {
                "window.list" => Ok(Response::with_result(json!({"windows": self.windows}))),
                "tree" => {
                    let window = wire["args"]["window"].as_str().unwrap_or_default();
                    Ok(self.trees.get(window).map_or_else(
                        || Response::failure("window not open"),
                        |tree| Response::with_result(json!({"tree": tree})),
                    ))
                }
                _ => self.event_pages.pop_front().unwrap_or_else(|| Ok(page(10, vec![]))),
            }
        }

        fn sleep(&mut self, delay: Duration) {
            self.sleeps.push(delay);
            self.clock += delay;
        }

        fn now(&mut self) -> Duration {
            self.clock
        }
    }

    fn session(id: &str, status: Option<&str>, active: bool) -> Value {
        let mut node = json!({"id": id, "name": "s", "cwd": "/tmp", "active": active, "split": false,
            "overlay": false, "scratch": false, "flagged": false});
        if let Some(status) = status {
            node["status"] = json!(status);
        }
        node
    }

    fn tree(sessions: &[Value]) -> Value {
        json!({"workspaces": [{"id": Uuid::new_v4().to_string(), "name": "w", "active": true, "sessions": sessions}]})
    }

    fn window(id: &str, open: bool, active: bool) -> Value {
        json!({"id": id, "name": "w", "open": open, "active": active})
    }

    fn page(next: u64, items: Vec<ControlEvent>) -> Response {
        let batch = EventBatch {
            run: RUN.to_owned(),
            next,
            items,
        };
        Response::with_result(json!({"events": batch}))
    }

    fn event(seq: u64, kind: EventKind, session: &str, status: Option<&str>, previous: Option<&str>) -> ControlEvent {
        ControlEvent {
            seq,
            ts: 1.0,
            kind,
            window: None,
            workspace: None,
            session: Some(session.to_owned()),
            payload: EventPayload {
                name: Some("s".to_owned()),
                status: status.map(str::to_owned),
                previous: previous.map(str::to_owned),
                ..EventPayload::default()
            },
        }
    }

    fn status_event(seq: u64, session: &str, status: &str) -> ControlEvent {
        event(seq, EventKind::Status, session, Some(status), None)
    }

    #[derive(Debug, Parser)]
    #[command(args_override_self = true)]
    struct Harness {
        #[command(flatten)]
        wait: Wait,
    }

    fn command(argv: &[&str]) -> Wait {
        Harness::try_parse_from(std::iter::once("wait").chain(argv.iter().copied()))
            .unwrap()
            .wait
    }

    fn matched(text: &str, status: AgentStatus) -> Outcome {
        Outcome::Matched { id: id(text), status }
    }

    #[test]
    fn the_current_status_matches_without_reading_events() {
        let mut fake = FakeApp::new(Some("completed"), None);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Completed)));
        assert_eq!(fake.cmds(), ["events.read", "window.list", "tree"]);
    }

    #[test]
    fn the_anchor_is_taken_before_the_tree_is_read() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([
            Ok(page(7, vec![])),
            Ok(page(8, vec![status_event(8, WORKER, "completed")])),
        ]);
        command(&["--target", WORKER, "--until", "completed"])
            .wait(&mut fake)
            .unwrap();
        assert_eq!(fake.cmds(), ["events.read", "window.list", "tree", "events.read"]);
        assert_eq!(
            fake.requests[0],
            json!({"cmd": "events.read", "args": {"kinds": ["status", "session.closed"]}})
        );
        assert_eq!(
            fake.requests[3],
            json!({"cmd": "events.read", "args": {"after": "7", "run": RUN, "kinds": ["status", "session.closed"]}})
        );
        assert_eq!(fake.requests[2], json!({"cmd": "tree", "args": {"window": WINDOW}}));
    }

    #[test]
    fn an_absent_status_reads_as_idle() {
        let mut fake = FakeApp::new(None, None);
        let outcome = command(&["--target", WORKER, "--until", "idle"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Idle)));
    }

    #[test]
    fn a_later_event_for_the_session_matches_and_others_are_ignored() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([
            Ok(page(10, vec![])),
            Ok(page(
                12,
                vec![status_event(11, OTHER, "completed"), status_event(12, WORKER, "active")],
            )),
            Ok(page(
                13,
                vec![event(13, EventKind::Status, WORKER, Some("blocked"), Some("active"))],
            )),
        ]);
        let outcome = command(&["--target", WORKER, "--until", "completed", "--until", "blocked"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Blocked)));
    }

    #[test]
    fn a_repeated_status_event_counts_as_reached() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([
            Ok(page(10, vec![])),
            Ok(page(
                11,
                vec![event(
                    11,
                    EventKind::Status,
                    WORKER,
                    Some("completed"),
                    Some("completed"),
                )],
            )),
        ]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Completed)));
    }

    #[test]
    fn the_sessions_close_ends_the_wait() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([
            Ok(page(10, vec![])),
            Ok(page(
                12,
                vec![
                    status_event(11, OTHER, "completed"),
                    event(12, EventKind::SessionClosed, WORKER, None, None),
                ],
            )),
        ]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Ok(Outcome::Closed));
    }

    #[test]
    fn another_sessions_close_is_ignored() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([
            Ok(page(10, vec![])),
            Ok(page(
                12,
                vec![
                    event(11, EventKind::SessionClosed, OTHER, None, None),
                    status_event(12, WORKER, "completed"),
                ],
            )),
        ]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Completed)));
    }

    #[test]
    fn the_deadline_passes_without_a_match() {
        let mut fake = FakeApp::new(Some("active"), None);
        let outcome = command(&["--target", WORKER, "--until", "completed", "--timeout", "1"]).wait(&mut fake);
        assert_eq!(outcome, Ok(Outcome::TimedOut));
        assert!(
            fake.sleeps.iter().all(|sleep| *sleep <= POLL_DELAY),
            "{:?}",
            fake.sleeps
        );
        assert_eq!(fake.clock, Duration::from_secs(1));
    }

    #[test]
    fn a_sleep_is_clipped_to_the_deadline() {
        let mut fake = FakeApp::new(Some("active"), None);
        let outcome = command(&["--target", WORKER, "--until", "completed", "--timeout", "0.3"]).wait(&mut fake);
        assert_eq!(outcome, Ok(Outcome::TimedOut));
        assert_eq!(fake.sleeps, [POLL_DELAY, Duration::from_millis(50)]);
    }

    #[test]
    fn a_refused_connect_retries_within_the_deadline() {
        let mut fake = FakeApp::new(Some("active"), None);
        let refused = ClientError {
            message: "refused".to_owned(),
            connect_errno: Some(libc::ECONNREFUSED),
        };
        fake.event_pages = VecDeque::from([
            Ok(page(10, vec![])),
            Err(refused),
            Ok(page(11, vec![status_event(11, WORKER, "completed")])),
        ]);
        let outcome = command(&["--target", WORKER, "--until", "completed", "--timeout", "5"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Completed)));
        assert_eq!(fake.sleeps, [FIRST_RETRY_DELAY]);
    }

    #[test]
    fn a_missing_socket_fails_at_once() {
        let mut fake = FakeApp::new(Some("active"), None);
        let missing = ClientError {
            message: "no socket".to_owned(),
            connect_errno: Some(libc::ENOENT),
        };
        fake.event_pages = VecDeque::from([Ok(page(10, vec![])), Err(missing)]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Err("no socket".to_owned()));
        assert!(fake.sleeps.is_empty());
    }

    #[test]
    fn a_refused_first_read_fails_at_once() {
        let mut fake = FakeApp::new(Some("active"), None);
        let refused = ClientError {
            message: "refused".to_owned(),
            connect_errno: Some(libc::ECONNREFUSED),
        };
        fake.event_pages = VecDeque::from([Err(refused)]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Err("refused".to_owned()));
        assert_eq!(fake.cmds(), ["events.read"]);
    }

    #[test]
    fn a_run_change_fails_the_wait() {
        let mut fake = FakeApp::new(Some("active"), None);
        fake.event_pages = VecDeque::from([Ok(page(10, vec![])), Ok(Response::failure("event run changed"))]);
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Err("event run changed".to_owned()));
    }

    #[test]
    fn active_resolves_to_the_frontmost_windows_selection() {
        let mut fake = FakeApp::new(Some("completed"), Some(WORKER));
        let outcome = command(&["--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(WORKER, AgentStatus::Completed)));
    }

    #[test]
    fn a_prefix_resolves_in_a_background_window() {
        let mut fake = FakeApp::new(Some("active"), Some(WORKER));
        let hidden = "EEEE5555-0000-4000-8000-000000000005";
        fake.windows.push(window(BACKGROUND, true, false));
        fake.trees
            .insert(BACKGROUND.to_owned(), tree(&[session(hidden, Some("blocked"), true)]));
        let outcome = command(&["--target", "eeee", "--until", "blocked"]).wait(&mut fake);
        assert_eq!(outcome, Ok(matched(hidden, AgentStatus::Blocked)));
    }

    #[test]
    fn a_prefix_matching_sessions_in_two_windows_is_ambiguous() {
        let mut fake = FakeApp::new(Some("completed"), None);
        let twin = "AAAA1111-0000-4000-8000-000000000009";
        fake.windows.push(window(BACKGROUND, true, false));
        fake.trees
            .insert(BACKGROUND.to_owned(), tree(&[session(twin, Some("completed"), false)]));
        let outcome = command(&["--target", "aaaa", "--until", "completed"]).wait(&mut fake);
        assert_eq!(
            outcome,
            Err("ambiguous session prefix 'aaaa' → AAAA1111, AAAA1111".to_owned())
        );
    }

    #[test]
    fn an_unresolved_target_fails() {
        for target in ["DDDD", ""] {
            let mut fake = FakeApp::new(Some("completed"), None);
            let outcome = command(&["--target", target, "--until", "completed"]).wait(&mut fake);
            assert_eq!(outcome, Err(format!("no such session: {target}")));
        }
        // no window selects a session: `active` names nothing
        let mut fake = FakeApp::new(Some("completed"), None);
        let outcome = command(&["--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Err("no such session: active".to_owned()));
    }

    #[test]
    fn closed_windows_are_not_read() {
        let mut fake = FakeApp::new(Some("completed"), None);
        fake.windows.push(window(BACKGROUND, false, false));
        command(&["--target", WORKER, "--until", "completed"])
            .wait(&mut fake)
            .unwrap();
        let trees: Vec<&Value> = fake
            .requests
            .iter()
            .filter(|request| request["cmd"] == "tree")
            .map(|request| &request["args"]["window"])
            .collect();
        assert_eq!(trees, [&json!(WINDOW)]);
    }

    #[test]
    fn a_failed_read_names_the_servers_error() {
        let mut fake = FakeApp::new(Some("completed"), None);
        fake.windows.push(window(BACKGROUND, true, false));
        let outcome = command(&["--target", WORKER, "--until", "completed"]).wait(&mut fake);
        assert_eq!(outcome, Err("window not open".to_owned()));
    }

    #[test]
    fn the_output_is_the_status_or_one_json_object() {
        let worker = id(WORKER);
        let plain = command(&["--until", "idle"])
            .output(worker, AgentStatus::Completed)
            .unwrap();
        assert_eq!(plain, "completed");
        let json = command(&["--until", "idle", "--json"])
            .output(worker, AgentStatus::Completed)
            .unwrap();
        assert_eq!(json, format!(r#"{{"id":"{WORKER}","status":"completed"}}"#));
    }

    fn refusal(argv: &[&str]) -> Result<(), String> {
        let command_line = ["ngtermctl", "session", "wait"].iter().chain(argv).copied();
        let crate::Command::Session(group) = crate::Ngtermctl::try_parse_from(command_line).unwrap().command else {
            panic!("not a session command")
        };
        group.command().validate()
    }

    #[test]
    fn invalid_arguments_are_refused_before_any_request() {
        let cases: [(&[&str], &str); 7] = [
            (&[], "provide at least one --until status"),
            (
                &["--until", "done"],
                "invalid status: done (expected idle, active, completed or blocked)",
            ),
            (
                &["--until", "completed", "--until", "Idle"],
                "invalid status: Idle (expected idle, active, completed or blocked)",
            ),
            (
                &["--until", "completed", "--timeout", "0"],
                "--timeout must be a positive number of seconds",
            ),
            (
                &["--until", "completed", "--timeout", "-1"],
                "--timeout must be a positive number of seconds",
            ),
            (
                &["--until", "completed", "--timeout", "inf"],
                "--timeout must be a positive number of seconds",
            ),
            (
                &["--until", "completed", "--timeout", "nan"],
                "--timeout must be a positive number of seconds",
            ),
        ];
        for (argv, message) in cases {
            assert_eq!(refusal(argv), Err(message.to_owned()), "{argv:?}");
        }
        assert_eq!(
            refusal(&["--until", "blocked", "--until", "idle", "--timeout", "0.5"]),
            Ok(())
        );
    }

    #[test]
    fn the_request_is_the_anchoring_read() {
        let request = serde_json::to_value(command(&["--until", "idle"]).make_request()).unwrap();
        assert_eq!(
            request,
            json!({"cmd": "events.read", "args": {"kinds": ["status", "session.closed"]}})
        );
    }

    #[test]
    fn a_timeout_too_long_to_represent_never_passes() {
        assert_eq!(command(&["--until", "idle", "--timeout", "1e300"]).timeout(), None);
        assert_eq!(
            command(&["--until", "idle", "--timeout", "1.5"]).timeout(),
            Some(Duration::from_millis(1500))
        );
    }

    #[test]
    fn the_watchdog_fires_while_the_caller_is_blocked() {
        let (fired, wait) = mpsc::channel();
        let watchdog = Watchdog::start(Duration::from_millis(50), move || fired.send(()).unwrap());
        assert_eq!(wait.recv_timeout(Duration::from_secs(5)), Ok(()));
        watchdog.cancel();
    }

    #[test]
    fn a_cancelled_watchdog_does_not_fire() {
        let (fired, wait) = mpsc::channel();
        let watchdog = Watchdog::start(Duration::from_millis(50), move || fired.send(()).unwrap());
        watchdog.cancel();
        assert_eq!(
            wait.recv_timeout(Duration::from_millis(300)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
}
