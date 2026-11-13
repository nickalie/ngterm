//! A viewer's new split must attach to an origin daemon before the terminal receives its launch seed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ngterm_host::fakes::FakeSurface;
use ngterm_proto::{ControlLiveEndpoint, command::SessionCmd};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::commands::notify::NotificationEffect;
use crate::live::daemon_name;
use crate::model::{NewSession, RemoteBinding, RemoteOrigin};
use crate::remote::frames::PresentationLayout;
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};
use crate::restore::spawn_plan::LaunchSeed;

const ORIGIN: &str = "origin-session";

#[derive(Default)]
struct Runner {
    results: Mutex<VecDeque<RemoteCommandResult>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl RemoteCommandRunner for Runner {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        self.calls.lock().unwrap().push(argv.to_vec());
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| result(&json!({"ok": false})))
    }
}

fn result(body: &serde_json::Value) -> RemoteCommandResult {
    RemoteCommandResult {
        status: 0,
        stdout: body.to_string(),
        stderr: String::new(),
    }
}

struct Remote {
    fixture: Fixture,
    runner: Arc<Runner>,
    row: Uuid,
    pane: Uuid,
    origin_pane: Uuid,
}

impl Remote {
    fn new() -> Self {
        let mut fixture = Fixture::new(0);
        let workspace = fixture.workspace;
        let store = fixture.store_mut().unwrap();
        let row = store
            .add_session(
                workspace,
                NewSession {
                    remote_host: Some("buildbox".to_owned()),
                    command: Some("existing remote attach".to_owned()),
                    wait: true,
                    ..NewSession::at_cwd("/home/u")
                },
            )
            .unwrap();
        let primary = store.session(row).unwrap().pane_identity;
        store
            .session_mut(row)
            .unwrap()
            .surface
            .replace(Box::new(FakeSurface::new(wire(primary))));
        let endpoint = ControlLiveEndpoint {
            peer: None,
            attach_bridge: None,
            executable: "/ngterm-session".to_owned(),
            socket_directory: "/tmp/live".to_owned(),
        };
        let origin_pane = Uuid::from_u128(1);
        store.bind_remote(
            RemoteBinding::new(
                ORIGIN,
                HashMap::from([(primary, daemon_name(origin_pane))]),
                None,
                Some(RemoteOrigin {
                    host: "buildbox".to_owned(),
                    endpoint,
                    session_name: "remote".to_owned(),
                }),
            ),
            row,
        );
        let reply = fixture.session(
            SessionCmd::Split,
            Some(&wire(row)),
            args(|args| {
                args.mode = Some("on".to_owned());
            }),
        );
        assert!(reply.ok);
        let session = fixture.store_mut().unwrap().session_mut(row).unwrap();
        let pane = session.split_pane_identity.unwrap();
        session.split_surface.replace(Box::new(FakeSurface::new(wire(pane))));
        let runner = Arc::new(Runner::default());
        fixture.core.set_remote_runner(runner.clone());
        Self {
            fixture,
            runner,
            row,
            pane,
            origin_pane,
        }
    }

    fn script(&self, results: impl IntoIterator<Item = RemoteCommandResult>) {
        self.runner.results.lock().unwrap().extend(results);
    }

    fn listing(panes: &[Uuid]) -> RemoteCommandResult {
        let panes: Vec<_> = panes
            .iter()
            .enumerate()
            .map(|(index, pane)| {
                json!({
                    "pane": if index == 0 { "left" } else { "right" }, "daemon": daemon_name(*pane)
                })
            })
            .collect();
        result(&json!({"ok": true, "result": {"remote": {
            "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/live"},
            "sessions": [{"id": ORIGIN, "name": "remote", "windowID": "w", "workspaceID": "ws",
                "windowName": "main", "workspaceName": "work", "cwd": "/remote", "panes": panes}]
        }}}))
    }

    fn spawn(&mut self) -> LaunchSeed {
        for _ in 0..400 {
            self.fixture.core.remote_tick(Instant::now());
            let grants = self.fixture.core.take_spawn_grants();
            if let Some(grant) = grants.into_iter().find(|grant| grant.token == wire(self.pane)) {
                return grant.seed;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the remote pane never received a spawn grant");
    }

    fn request(&mut self) {
        assert_eq!(self.fixture.core.request_pane_spawn(self.row, &wire(self.pane)), None);
    }

    fn has_failure(&mut self, reason: &str) -> bool {
        self.fixture
            .core
            .take_notifications()
            .iter()
            .any(|effect| matches!(effect, NotificationEffect::App { body, .. } if body.contains(reason)))
    }

    fn wait_for_listing(&mut self) {
        for _ in 0..400 {
            if self.runner.calls.lock().unwrap().len() >= 2 {
                return;
            }
            self.fixture.core.remote_tick(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the origin listing never started");
    }
}

#[test]
fn a_remote_split_is_created_once_and_attached_with_lead_and_reconnect() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    remote.request();
    let seed = remote.spawn();
    let command = crate::remote::viewer_run::readable(&seed.command.unwrap());
    assert!(command.contains("ssh") && command.contains("buildbox") && command.contains(&daemon_name(target)));
    assert!(command.contains("NGTERM_LIVE_MANAGED_CLAIM=1") && command.contains("reconnecting"));
    assert!(seed.wait_after_command);
    assert!(seed.remote, "the attach names its own end: never a failure to launch");
    assert!(!seed.live, "the viewer must not wrap ssh in a local live daemon");
    let calls = remote.runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let create = calls[0].last().unwrap();
    assert!(create.contains("split") && create.contains("on") && create.contains(ORIGIN));
    let session = remote.fixture.store().session(remote.row).unwrap();
    assert_eq!(
        session.remote_presentation().unwrap().binding.remote_pane(remote.pane),
        Some(target)
    );
    assert_eq!(session.initial_cwd(), "/home/u");
}

#[test]
fn a_refused_remote_split_gets_an_error_receipt_instead_of_a_local_shell() {
    let mut remote = Remote::new();
    remote.script([result(&json!({"ok": false, "error": "origin refused"}))]);
    remote.request();
    let seed = remote.spawn();
    assert!(seed.wait_after_command && seed.remote);
    assert!(seed.command.is_some());
    assert!(remote.has_failure("origin refused"));
    assert_eq!(remote.runner.calls.lock().unwrap().len(), 1);
}

#[test]
fn a_remote_split_waits_for_the_origin_daemon_without_reissuing_the_mutation() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane]),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    assert_eq!(remote.runner.calls.lock().unwrap().len(), 3);
}

#[test]
fn a_closed_split_does_not_bind_or_spawn_its_replacement() {
    let mut remote = Remote::new();
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, Uuid::from_u128(2)]),
    ]);
    remote.request();
    remote.wait_for_listing();
    let store = remote.fixture.store_mut().unwrap();
    store.close_split(remote.row);
    store.toggle_split(remote.row, None);
    let session = store.session_mut(remote.row).unwrap();
    let replacement = session.split_pane_identity.unwrap();
    session
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(replacement))));
    for _ in 0..20 {
        remote.fixture.core.remote_tick(Instant::now());
        assert!(remote.fixture.core.take_spawn_grants().is_empty());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(remote.fixture.core.take_spawn_grants().is_empty());
    let session = remote.fixture.store().session(remote.row).unwrap();
    assert_ne!(session.split_pane_identity, Some(remote.pane));
    assert!(session.split_launch.command.is_none());
    assert_eq!(
        session.remote_presentation().unwrap().binding.remote_pane(replacement),
        None
    );
}

#[test]
fn a_local_split_still_receives_its_plain_shell_seed_immediately() {
    let mut fixture = Fixture::new(0);
    let row = fixture.sessions[0];
    fixture.store_mut().unwrap().toggle_split(row, None);
    let pane = fixture.store().session(row).unwrap().split_pane_identity.unwrap();
    assert_eq!(
        fixture.core.request_pane_spawn(row, &wire(pane)),
        Some(LaunchSeed::default())
    );
}

#[test]
fn a_remote_split_reuses_the_surviving_origins_other_pane_after_local_close() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    remote.spawn();
    let old = remote.pane;
    let store = remote.fixture.store_mut().unwrap();
    store.close_split(remote.row);
    store.toggle_split(remote.row, None);
    let session = store.session_mut(remote.row).unwrap();
    remote.pane = session.split_pane_identity.unwrap();
    session
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(remote.pane))));
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    let binding = &remote
        .fixture
        .store()
        .session(remote.row)
        .unwrap()
        .remote_presentation()
        .unwrap()
        .binding;
    assert_ne!(remote.pane, old);
    assert_eq!(binding.local_pane(target), Some(remote.pane));
}

#[test]
fn a_remote_split_completed_during_soft_close_spawns_only_after_undo() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    assert!(remote.fixture.store_mut().unwrap().soft_close_session(remote.row));
    for _ in 0..400 {
        remote.fixture.core.remote_tick(Instant::now());
        let attached = remote
            .fixture
            .core
            .library()
            .held_session(remote.row)
            .unwrap()
            .split_launch
            .command
            .is_some();
        if attached {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(remote.fixture.core.take_spawn_grants().is_empty());
    assert!(remote.fixture.store_mut().unwrap().undo_pending_close(None));
    remote.fixture.core.commit();
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
}

#[test]
fn a_changed_origin_endpoint_does_not_attach_to_another_server() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    let mut listing = Remote::listing(&[remote.origin_pane, target]);
    listing.stdout = listing.stdout.replace("/tmp/live", "/tmp/another-server");
    remote.script([result(&json!({"ok": true})), listing]);
    remote.request();
    let seed = remote.spawn();
    assert!(seed.command.is_some());
    assert!(remote.has_failure("endpoint changed"));
    let session = remote.fixture.store().session(remote.row).unwrap();
    assert_eq!(
        session.remote_presentation().unwrap().binding.remote_pane(remote.pane),
        None
    );
}

#[test]
fn a_transient_listing_failure_is_retried_without_recreating_the_origin_split() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        RemoteCommandResult {
            status: 255,
            stdout: String::new(),
            stderr: "temporary ssh failure".to_owned(),
        },
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    assert_eq!(remote.runner.calls.lock().unwrap().len(), 3);
    assert!(!remote.has_failure("temporary ssh failure"));
}

#[test]
fn closing_the_sibling_during_creation_does_not_change_the_requested_origin_daemon() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane]),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.request();
    remote.fixture.store_mut().unwrap().close_primary_pane(remote.row);
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    assert_eq!(remote.runner.calls.lock().unwrap().len(), 3);
    assert_eq!(
        remote.fixture.store().session(remote.row).unwrap().pane_identity,
        remote.pane
    );
}

#[test]
fn completing_a_split_reconsiders_a_primary_removal_deferred_by_its_unbound_pane() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([result(&json!({"ok": true})), Remote::listing(&[target])]);
    let session = remote.fixture.store_mut().unwrap().session_mut(remote.row).unwrap();
    let primary = session.pane_identity;
    session
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(remote.pane)).with_realized(false)));
    remote.request();
    remote.fixture.core.apply_remote_layout(
        &PresentationLayout {
            panes: vec![target],
            primary: target,
            axis: None,
            shown: false,
        },
        remote.row,
    );
    remote.fixture.core.pane_held(remote.row, &wire(primary));
    assert_eq!(
        remote.fixture.store().session(remote.row).unwrap().pane_identity,
        primary
    );
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    let session = remote.fixture.store().session(remote.row).unwrap();
    assert_eq!(session.pane_identity, remote.pane);
    assert!(!session.split.exists);
}

#[test]
fn an_old_layout_does_not_close_the_split_when_its_binding_completes() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[remote.origin_pane, target]),
    ]);
    remote.fixture.core.apply_remote_layout(
        &PresentationLayout {
            panes: vec![remote.origin_pane],
            primary: remote.origin_pane,
            axis: None,
            shown: false,
        },
        remote.row,
    );
    remote.request();
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(target)));
    assert!(remote.fixture.store().session(remote.row).unwrap().split.exists);
}

#[test]
fn repeated_listing_failures_stop_at_the_ready_deadline_and_report_the_last_error() {
    use std::sync::atomic::{AtomicU64, Ordering};

    let mut remote = Remote::new();
    let start = Instant::now();
    let ticks = AtomicU64::new(0);
    remote.fixture.core.remote.clock =
        Box::new(move || start + Duration::from_secs(ticks.fetch_add(1, Ordering::Relaxed)));
    remote.script(
        std::iter::once(result(&json!({"ok": true}))).chain((0..100).map(|_| RemoteCommandResult {
            status: 255,
            stdout: String::new(),
            stderr: "persistent ssh failure".to_owned(),
        })),
    );
    remote.request();
    let seed = remote.spawn();
    assert!(seed.command.is_some() && seed.wait_after_command);
    assert!(remote.has_failure("persistent ssh failure"));
    assert!(remote.runner.calls.lock().unwrap().len() > 2);
}

#[test]
fn consecutive_pending_splits_get_distinct_origin_daemons() {
    let mut remote = Remote::new();
    let first = Uuid::from_u128(2);
    let second = Uuid::from_u128(3);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[first]),
        result(&json!({"ok": true})),
        Remote::listing(&[first, second]),
    ]);
    remote.request();
    let first_local = remote.pane;
    let store = remote.fixture.store_mut().unwrap();
    store.close_primary_pane(remote.row);
    store.toggle_split(remote.row, None);
    let session = store.session_mut(remote.row).unwrap();
    let second_local = session.split_pane_identity.unwrap();
    session
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(second_local))));
    assert_eq!(
        remote.fixture.core.request_pane_spawn(remote.row, &wire(second_local)),
        None
    );
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(first)));
    remote.pane = second_local;
    assert!(crate::remote::viewer_run::readable(&remote.spawn().command.unwrap()).contains(&daemon_name(second)));
    let binding = &remote
        .fixture
        .store()
        .session(remote.row)
        .unwrap()
        .remote_presentation()
        .unwrap()
        .binding;
    assert_eq!(binding.remote_pane(first_local), Some(first));
    assert_eq!(binding.remote_pane(second_local), Some(second));
    assert_eq!(binding.local_pane(first), Some(first_local));
    assert_eq!(remote.runner.calls.lock().unwrap().len(), 4);
}

#[test]
fn a_cached_origin_swap_is_applied_after_the_new_terminal_is_realized() {
    let mut remote = Remote::new();
    let target = Uuid::from_u128(2);
    remote.script([
        result(&json!({"ok": true})),
        Remote::listing(&[target, remote.origin_pane]),
    ]);
    let session = remote.fixture.store_mut().unwrap().session_mut(remote.row).unwrap();
    let primary = session.pane_identity;
    session
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(remote.pane)).with_realized(false)));
    remote.fixture.core.apply_remote_layout(
        &PresentationLayout {
            panes: vec![target, remote.origin_pane],
            primary: target,
            axis: Some("horizontal".to_owned()),
            shown: true,
        },
        remote.row,
    );
    remote.request();
    remote.spawn();
    assert_eq!(
        remote.fixture.store().session(remote.row).unwrap().pane_identity,
        primary
    );
    remote
        .fixture
        .store_mut()
        .unwrap()
        .session_mut(remote.row)
        .unwrap()
        .split_surface
        .replace(Box::new(FakeSurface::new(wire(remote.pane)).with_realized(true)));
    remote.fixture.core.remote_tick(Instant::now());
    let session = remote.fixture.store().session(remote.row).unwrap();
    assert_eq!(session.pane_identity, remote.pane);
    assert_eq!(session.split_pane_identity, Some(primary));
    assert_eq!(session.split.axis, crate::model::SplitAxis::TopBottom);
}

#[test]
fn a_remote_split_carries_the_exact_capability_and_nonce_of_its_attach() {
    for (cached, capability) in [
        (None, None),
        (None, Some(crate::attach_bridge::VERSION)),
        (Some(crate::attach_bridge::VERSION), None),
        (Some(crate::attach_bridge::VERSION), Some(crate::attach_bridge::VERSION)),
    ] {
        let mut remote = Remote::new();
        remote
            .fixture
            .store_mut()
            .unwrap()
            .session_mut(remote.row)
            .unwrap()
            .remote_presentation
            .as_mut()
            .unwrap()
            .binding
            .origin
            .as_mut()
            .unwrap()
            .endpoint
            .attach_bridge = cached;
        let target = Uuid::from_u128(2);
        let mut listing = Remote::listing(&[remote.origin_pane, target]);
        let mut body: serde_json::Value = serde_json::from_str(&listing.stdout).unwrap();
        if let Some(capability) = capability {
            body["result"]["remote"]["endpoint"]["attachBridge"] = json!(capability);
        }
        listing.stdout = body.to_string();
        remote.script([result(&json!({"ok":true})), listing]);
        remote.request();
        let seed = remote.spawn();
        let expected = remote.fixture.core.live.leads.nonce(remote.pane).unwrap();
        let nonce = seed
            .env
            .iter()
            .find(|(key, _)| key == crate::attach_bridge::VARIABLE)
            .and_then(|(_, value)| crate::attach_bridge::parse_variable(value));
        assert_eq!(nonce, capability.map(|_| expected));
        assert_eq!(
            crate::remote::viewer_run::readable(seed.command.as_ref().unwrap()).contains("attach-bridge"),
            capability.is_some()
        );
    }
}
