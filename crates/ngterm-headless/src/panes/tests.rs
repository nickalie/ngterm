use ngterm_term::ExitStatus;

use super::*;

fn exit(held: bool) -> PaneEvent {
    PaneEvent::Exited(ExitStatus {
        code: 0,
        signal: None,
        held,
    })
}

const SHELL: Ran = Ran {
    command: false,
    wait: false,
};
const HELD_COMMAND: Ran = Ran {
    command: true,
    wait: true,
};

#[test]
fn titles_directories_and_notifications_reach_the_model() {
    assert_eq!(
        report(&PaneEvent::Title("vim".to_owned()), SHELL, "box"),
        Some(Report::Pane(PaneReport::Title("vim".to_owned())))
    );
    assert_eq!(
        report(&PaneEvent::Pwd("file://box/srv/app".to_owned()), SHELL, "box"),
        Some(Report::Pane(PaneReport::Pwd("/srv/app".to_owned())))
    );
    assert_eq!(
        report(&PaneEvent::Pwd("file://elsewhere/srv".to_owned()), SHELL, "box"),
        None,
        "another host's directory is not this pane's"
    );
    assert_eq!(
        report(
            &PaneEvent::Notification {
                title: "done".to_owned(),
                body: "build".to_owned()
            },
            SHELL,
            "box"
        ),
        Some(Report::Notification {
            title: "done".to_owned(),
            body: "build".to_owned()
        })
    );
    assert_eq!(report(&PaneEvent::Bell, SHELL, "box"), None);
}

#[test]
fn an_exit_closes_the_pane_unless_a_held_command_ended() {
    assert_eq!(report(&exit(false), SHELL, "box"), Some(Report::Exited));
    // no key will ever close a held exit here
    assert_eq!(report(&exit(true), SHELL, "box"), Some(Report::Exited));
    assert_eq!(report(&exit(true), HELD_COMMAND, "box"), Some(Report::Held));
    assert_eq!(report(&exit(false), HELD_COMMAND, "box"), Some(Report::Exited));
}

#[test]
fn a_model_without_windows_lays_out_no_pane() {
    let core = Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState::default(),
    );
    assert!(layout(&core).is_empty());
}

/// A core over one window whose first session has a split, each pane holding a fake terminal; the session's id and
/// its primary's and split's tokens.
fn split_core() -> (Core, Uuid, String, String) {
    use ngterm_core::model::WindowLibrary;
    use ngterm_host::fakes::FakeSurface;

    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    store.toggle_split(session, None);
    let found = store.session_mut(session).unwrap();
    let primary = pane_env::uuid_string(found.pane_identity);
    let split = pane_env::uuid_string(found.split_pane_identity.unwrap());
    drop(
        found
            .pane_slot_mut(StatusPane::Left)
            .replace(Box::new(FakeSurface::new(primary.clone()))),
    );
    drop(
        found
            .pane_slot_mut(StatusPane::Right)
            .replace(Box::new(FakeSurface::new(split.clone()))),
    );
    let core = Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState {
            library,
            ..ngterm_core::CoreState::default()
        },
    );
    (core, session, primary, split)
}

/// What the panes start under, with the stock config and no login shell, resources or binaries.
fn environment() -> PaneEnvironment {
    let config = GhosttyConfig::default();
    PaneEnvironment {
        socket: "/tmp/a.sock".to_owned(),
        program_version: "1.2.3".to_owned(),
        host: Host::new(&config, None, None, None),
        config,
        grid: DEFAULT_GRID,
        hostname: "box".to_owned(),
    }
}

fn role(core: &Core, session: Uuid, token: &str) -> Option<StatusPane> {
    PaneSpec::of_token(core, session, token).map(|spec| spec.pane)
}

#[test]
fn a_token_keeps_its_role_until_the_model_moves_it() {
    let (core, session, primary, split) = split_core();
    assert_eq!(role(&core, session, &primary), Some(StatusPane::Left));
    assert_eq!(role(&core, session, &split), Some(StatusPane::Right));
    assert_eq!(role(&core, session, "no-such-token"), None);
    assert_eq!(role(&core, Uuid::new_v4(), &primary), None);
}

#[test]
fn a_swap_moves_each_token_to_the_other_role() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .swap_panes(session)
        .unwrap();
    assert_eq!(role(&core, session, &split), Some(StatusPane::Left));
    assert_eq!(role(&core, session, &primary), Some(StatusPane::Right));
}

#[test]
fn a_promoted_split_survivor_is_the_primary() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    core.library_mut()
        .store_mut(window)
        .unwrap()
        .close_primary_pane(session);
    let spec = PaneSpec::of_token(&core, session, &split).unwrap();
    assert_eq!(spec.pane, StatusPane::Left);
    assert!(
        spec.env(&environment())
            .contains(&("NGTERM_PANE".to_owned(), "left".to_owned())),
        "the survivor's shell is told its new role"
    );
    assert_eq!(role(&core, session, &primary), None);
}

#[test]
fn a_respawned_primary_keeps_its_token_but_needs_a_fresh_terminal() {
    let (mut core, session, primary, split) = split_core();
    let window = core.library().active_window_id().unwrap();
    let store = core.library_mut().store_mut(window).unwrap();
    store
        .add_companion(session, ngterm_core::model::CompanionOptions::default(), "/home/u")
        .unwrap();
    store.close_split_pane(session);
    let spec = PaneSpec::of_token(&core, session, &primary).unwrap();
    assert!(carries(&core, &spec), "the terminal still runs");
    let store = core.library_mut().store_mut(window).unwrap();
    assert_eq!(
        store.close_primary_pane(session),
        ngterm_core::model::PrimaryPaneExit::Respawned
    );
    let spec = PaneSpec::of_token(&core, session, &primary).expect("the pane is still laid out");
    assert_eq!(spec.pane, StatusPane::Left);
    assert!(!carries(&core, &spec), "the respawn tore its terminal down");
    assert_eq!(role(&core, session, &split), None);
}
