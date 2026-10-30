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
