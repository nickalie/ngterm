//! A remote window's shared ssh connections (upstream's `SshConnectionPool` as `ControlServer+RemoteCatalog.swift`,
//! `RemoteCatalogClient`, `RemoteCatalogActions`, `ControlServer+RemotePresentation.swift` and
//! `ControlServer+RemoteReconnect.swift` use it): every ssh channel the window opens rides a master of its pool.

use super::*;
use crate::remote::catalog::pool::SshConnectionPool;

const W: Uuid = Uuid::from_u128(0x2222_0000_0000_4000_8000_0000_0000_0001);

impl Remote {
    fn pool(&self) -> SshConnectionPool {
        self.fixture
            .core
            .remote
            .viewer
            .catalogs
            .get(&self.window)
            .and_then(|run| run.pool.clone())
            .expect("the window's pool")
    }

    /// Ticks until `done` holds.
    fn tick_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        for _ in 0..400 {
            self.fixture.core.remote_tick(Instant::now());
            self.fixture.core.commit();
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("{what} never happened");
    }
}

/// Whether `line` rides a shared connection.
fn pooled(line: &str) -> bool {
    line.contains("ControlMaster=auto") && line.contains("ControlPath=")
}

fn present_link(remote: &Remote, id: Uuid) -> Link {
    remote
        .links()
        .into_iter()
        .rev()
        .find(|link| link.0.borrow().key.is_some_and(|key| key.session == id))
        .expect("the row's stream")
}

#[test]
fn the_probe_and_the_catalog_stream_ride_the_windows_master() {
    let remote = Remote::opened(&server_version());

    let probe = remote.runner.remote_runs().remove(0);
    let stream = remote.link().0.borrow().argv.clone();

    assert!(pooled(&probe.join(" ")), "{probe:?}");
    assert_eq!(stream[..2], ["/bin/sh", "-c"]);
    assert!(stream[2].contains("ngterm_connection_wait"), "{stream:?}");
    assert!(pooled(&stream[2]), "{stream:?}");
    assert!(
        remote
            .runner
            .runs
            .lock()
            .unwrap()
            .iter()
            .any(|argv| argv[..3] == ["ssh", "-O", "check"]),
        "the master is checked before the probe"
    );
}

#[test]
fn every_pane_and_scratch_attach_rides_a_channel_and_the_rows_stream_too() {
    let mut a = session_with(Uuid::new_v4(), "a", vec![pane(), pane()]);
    let scratch = pane();
    a.fields.scratch = Some(CatalogScratch {
        pane: scratch.clone(),
        shown: true,
    });
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone()])]));

    let row = remote.row(a.id());
    for command in [&row.launch.command, &row.split_launch.command, &row.scratch_command] {
        let command = readable(command.as_deref().unwrap());
        assert!(pooled(&command), "{command}");
        assert!(command.contains("if ngterm_connection_wait; then"), "{command}");
    }
    let stream = present_link(&remote, a.id()).0.borrow().argv.clone();
    assert!(
        stream[2].contains("ngterm_connection_wait") && pooled(&stream[2]),
        "{stream:?}"
    );
    // the catalog's stream, the three panes and the row's stream
    assert_eq!(remote.pool().load(), vec![5]);
}

#[test]
fn a_pane_the_window_no_longer_shows_gives_its_channel_back() {
    let (a, b) = (session("a"), session("b"));
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone(), b.clone()])]));
    let before = remote.pool().load();

    let target = wire(a.id());
    remote
        .fixture
        .send(Command::Session(SessionCmd::Detach), Some(&target), Args::default());
    remote.fixture.core.commit();
    let after_detach = remote.pool().load();
    remote.delta(1, CatalogChange::SessionRemoved(b.id()));

    assert_eq!(before, vec![5]);
    assert_eq!(after_detach, vec![3], "a's pane and stream are given back");
    assert_eq!(remote.pool().load(), vec![1], "the catalog's stream alone is left");
}

#[test]
fn an_action_rides_a_channel_and_one_with_no_master_is_never_sent() {
    let a = session("a");
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone()])]));
    let target = wire(a.id());

    let renamed = remote.fixture.send(
        Command::Session(SessionCmd::Rename),
        Some(&target),
        args(|args| args.name = Some("x".to_owned())),
    );
    assert!(renamed.ok, "{renamed:?}");
    let sent = remote
        .runner
        .runs
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|argv| argv.last().is_some_and(|line| line.contains("--op-id")))
        .cloned()
        .unwrap();
    assert!(pooled(&sent.join(" ")), "{sent:?}");

    remote.runner.masters_down.store(true, Ordering::SeqCst);
    let operations = remote.runner.operations().len();
    let refused = remote.fixture.send(
        Command::Workspace(WorkspaceCmd::Rename),
        Some(&wire(W)),
        args(|args| args.name = Some("y".to_owned())),
    );

    assert_eq!(refused.error.as_deref(), Some("no connection to box"));
    assert_eq!(
        remote.runner.operations().len(),
        operations,
        "nothing reached the server"
    );
    let marks = remote.tree()["remote"]["operations"].clone();
    assert_eq!(
        marks.as_array().map(Vec::len),
        Some(1),
        "only the rename sent before is marked: {marks}"
    );
}

#[test]
fn closing_the_window_ends_its_masters() {
    let mut remote = Remote::opened(&server_version());
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![session("a")])]));
    let pool = remote.pool();

    remote.fixture.core.close_window(remote.window);
    remote.fixture.core.commit();

    assert!(pool.load().is_empty());
    let runner = remote.runner.clone();
    remote.tick_until("the master's exit", move |_| {
        runner
            .runs
            .lock()
            .unwrap()
            .iter()
            .any(|argv| argv[..3] == ["ssh", "-O", "exit"])
    });
}

#[test]
fn a_stopped_scratch_reconnects_through_the_pool() {
    let mut remote = Remote::opened(&server_version());
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone()])]));
    let mut shown = a.fields.clone();
    shown.scratch = Some(CatalogScratch {
        pane: pane(),
        shown: true,
    });
    remote.delta(1, CatalogChange::SessionChanged(shown));
    let token = wire(Uuid::new_v4());
    let row = remote.store_mut_row_for_pool(a.id());
    row.scratch_command = None;
    row.scratch_surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(token.clone())));

    assert!(crate::Core::pane_exited(&mut remote.fixture.core, a.id(), &token));
    remote.fixture.core.commit();
    remote.tick_until("the scratch's fresh attach", |remote| {
        remote.row(a.id()).scratch_command.is_some()
    });

    let probe = remote
        .runner
        .remote_runs()
        .into_iter()
        .find(|argv| argv.last().is_some_and(|line| line == "true"))
        .expect("the reconnect probe");
    assert!(pooled(&probe.join(" ")), "{probe:?}");
    let command = readable(remote.row(a.id()).scratch_command.as_deref().unwrap());
    assert!(pooled(&command), "{command}");
}

#[test]
fn without_a_pool_root_nothing_rides_a_shared_connection() {
    let mut remote = Remote::new(&server_version());
    remote.fixture.core.remote.viewer.ssh_pool_root = None;
    remote.window = answered_id(&remote.open("box")).unwrap();
    remote.settle();
    let a = session("a");
    remote.snapshot(snapshot(vec![workspace(W, "w", vec![a.clone()])]));

    let command = readable(remote.row(a.id()).launch.command.as_deref().unwrap());
    let stream = remote.link().0.borrow().argv.clone();

    assert!(!command.contains("ControlPath"), "{command}");
    assert_eq!(stream[0], "ssh");
    assert!(
        remote
            .runner
            .runs
            .lock()
            .unwrap()
            .iter()
            .all(|argv| !master_command(argv))
    );
}

impl Remote {
    fn store_mut_row_for_pool(&mut self, id: Uuid) -> &mut Session {
        self.fixture
            .core
            .library_mut()
            .store_mut(self.window)
            .and_then(|store| store.session_mut(id))
            .expect("the row")
    }
}
