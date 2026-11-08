//! Origin commands that cannot be launched never create or close a local row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ngterm_proto::AppIdentity;

use super::*;
use crate::model::{NewSession, RemoteBinding, WindowLibrary};
use crate::{CoreConfig, CoreState};

fn core() -> (Core, Uuid, Uuid, tempfile::TempDir) {
    let mut library = WindowLibrary::new("/tmp");
    let window = library.new_window(None);
    let store = library.store_mut(window).unwrap();
    let session = store.selected_session_id().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: crate::persist::AppSettings::default(),
            persistence: None,
        },
    );
    core.set_hud_body_dir(dir.path().join("bodies"));
    (core, window, session, dir)
}

fn failed_panel(core: &Core, session: Uuid, action: &str, reason: &str) {
    let spec = core
        .library()
        .held_session(session)
        .unwrap()
        .overlay
        .effective_hud_spec()
        .unwrap();
    assert_eq!(spec.message, format!("{action}: {reason}"));
    assert_eq!(spec.hide_after, Some(5.0));
}

#[test]
fn missing_windows_and_rows_cannot_start_origin_commands() {
    let (mut core, window, session, _dir) = core();
    let missing = Uuid::new_v4();
    assert!(!core.begin_duplicate_remote_session(missing, session));
    assert!(!core.begin_duplicate_remote_session(window, missing));
    assert!(!core.begin_new_remote_session(missing, session));
    assert!(!core.begin_new_remote_session(window, missing));
    assert!(!core.begin_end_remote_session(window, session));
    assert!(core.library().held_session(session).is_some());
    assert_eq!(core.remote_wake(), None);
}

#[test]
fn an_invalid_host_refuses_create_before_running_a_remote_command() {
    let (mut core, window, session, _dir) = core();
    let flow = AttachFlow::new(window, Some(session));
    assert!(core.start_origin_session(flow, "-invalid".to_owned(), None).is_none());
    failed_panel(&core, session, NEW_SESSION, "the remote command failed on -invalid");
    assert_eq!(core.library().store(window).unwrap().sessions().count(), 1);
    assert_eq!(core.remote_wake(), None);
}

#[test]
fn an_invalid_host_after_live_check_never_runs_create() {
    let (mut core, window, session, _dir) = core();
    let mut flow = AttachFlow::new(window, Some(session));
    flow.action = CreateRequest::New.action();
    let answer = Arc::new(Mutex::new(Some(RemoteCommandResult {
        status: 0,
        stdout: r#"{"ok":true,"result":{"restore":{"configured":"live","requestedAtLaunch":"live","active":"live","restartRequired":false}}}"#.to_owned(),
        stderr: String::new(),
    })));
    assert!(
        core.step_checking(&mut flow, "-invalid", &CreateRequest::New, answer)
            .is_none()
    );
    failed_panel(&core, session, NEW_SESSION, "the remote command failed on -invalid");
    assert_eq!(core.library().store(window).unwrap().sessions().count(), 1);
}

#[test]
fn ending_a_bound_row_with_an_invalid_host_keeps_it_open() {
    let (mut core, window, _, _dir) = core();
    let store = core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let session = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("-invalid".to_owned()),
                ..NewSession::at_cwd("/tmp")
            },
        )
        .unwrap();
    store.bind_remote(RemoteBinding::new("origin-id", HashMap::new(), Some(1), None), session);
    assert!(core.begin_end_remote_session(window, session));
    failed_panel(&core, session, END_SESSION, "the remote command failed on -invalid");
    assert!(core.library().held_session(session).is_some());
    assert_eq!(core.remote_wake(), None);
}

#[test]
fn a_created_session_that_cannot_be_listed_is_reported_without_retrying_create() {
    let (mut core, window, session, _dir) = core();
    let created = crate::pane_env::uuid_string(Uuid::new_v4());
    let wait = ListingWait::new(created.clone(), core.remote_now());
    let mut flow = AttachFlow::new(window, Some(session));
    flow.action = CreateRequest::New.action();
    assert!(
        core.step_waiting(
            &mut flow,
            "-invalid",
            None,
            &CreateRequest::New,
            Waiting { wait, read: None }
        )
        .is_none()
    );
    failed_panel(
        &core,
        session,
        NEW_SESSION,
        &format!(
            "created session {created} on -invalid but could not attach it: invalid host; `ngtermctl live attach -invalid {created}` attaches it"
        ),
    );
    assert_eq!(core.library().store(window).unwrap().sessions().count(), 1);
    assert_eq!(core.remote_wake(), None);
}
