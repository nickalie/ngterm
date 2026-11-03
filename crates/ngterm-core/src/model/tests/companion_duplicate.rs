//! The store half of Duplicate as Companion: upstream's `AppStoreCompanionsTests.duplicateAsCompanion*` cases.

use uuid::Uuid;

use super::support::{add, ids};
use crate::model::{AppStore, CompanionCopy, CompanionOptions, CompanionRefusal, PaneCopy, Session, SplitAxis};
use crate::pane_env::uuid_string;
use crate::restore::spawn_plan::ORDINARY_DIALECT;

const HOME: &str = "/home/u";

/// Upstream's `CompanionGroup`: `work` holds `parent` with its companions `first` and `second`, then `after`.
struct Group {
    store: AppStore,
    work: Uuid,
    parent: Uuid,
    first: Uuid,
    second: Uuid,
    after: Uuid,
}

impl Group {
    fn new() -> Self {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let parent = add(&mut store, work, "/parent");
        let after = add(&mut store, work, "/after");
        let mut companion = |cwd: &str| {
            let options = CompanionOptions {
                cwd: Some(cwd.to_owned()),
                ..CompanionOptions::default()
            };
            store.add_companion(parent, options, HOME).unwrap()
        };
        let first = companion("/first");
        let second = companion("/second");
        Self {
            store,
            work,
            parent,
            first,
            second,
            after,
        }
    }

    fn session(&self, id: Uuid) -> &Session {
        self.store.session(id).unwrap()
    }
}

/// A copied pane in `cwd` running `argv` (its ordinary-shell line the creation command), a plain shell without one.
fn pane(cwd: &str, argv: Option<&[&str]>) -> PaneCopy {
    let argv: Option<Vec<String>> = argv.map(|argv| argv.iter().map(|word| (*word).to_owned()).collect());
    PaneCopy {
        cwd: cwd.to_owned(),
        command: argv.as_deref().map(|argv| ORDINARY_DIALECT.line(argv).unwrap()),
        argv,
    }
}

fn copy(left: PaneCopy, right: Option<PaneCopy>) -> CompanionCopy {
    CompanionCopy {
        left,
        right,
        axis: SplitAxis::LeftRight,
        ratio: None,
        select: true,
    }
}

/// Upstream's `duplicateAsCompanionCopiesTheLayoutAndRunsEachProgramAsItsCommand`.
#[test]
fn duplicate_as_companion_copies_the_layout_and_runs_each_program_as_its_command() {
    let mut g = Group::new();
    let spec = CompanionCopy {
        axis: SplitAxis::TopBottom,
        ratio: Some(0.3),
        ..copy(pane("/srv", Some(&["htop", "-d", "5"])), Some(pane("/var/log", None)))
    };
    let copied = g.store.duplicate_as_companion(g.first, spec, HOME).unwrap();
    assert_eq!(ids(&g.store, g.work), [g.parent, g.first, g.second, copied, g.after]);
    let session = g.session(copied);
    assert_eq!(session.parent_id(), Some(g.parent));
    assert_eq!(session.initial_cwd(), "/srv");
    assert_eq!(
        session.launch.command,
        ORDINARY_DIALECT.line(&["htop".to_owned(), "-d".to_owned(), "5".to_owned()])
    );
    assert_eq!(
        session.launch.argv,
        Some(vec!["htop".to_owned(), "-d".to_owned(), "5".to_owned()])
    );
    assert!(!session.launch.wait);
    assert!(session.split.shown);
    assert_eq!(session.split.axis, SplitAxis::TopBottom);
    assert_eq!(session.split.ratio, Some(0.3));
    assert_eq!(session.initial_split_cwd.as_deref(), Some("/var/log"));
    assert_eq!(session.split_launch.command, None);
    assert_eq!(session.split_launch.argv, None);
    assert_eq!(g.store.selected_session_id(), Some(copied));
}

/// Upstream's `duplicateAsCompanionFocusesTheLeftPaneOfTheCopiedSplit`: a duplicated split opened like a new split
/// and focused its right pane.
#[test]
fn duplicate_as_companion_focuses_the_left_pane_of_the_copied_split() {
    let mut g = Group::new();
    g.store.set_split_visibility(g.first, true, None);
    assert!(g.session(g.first).focuses_split());
    let spec = copy(pane("/srv", None), Some(pane("/var/log", None)));
    let copied = g.store.duplicate_as_companion(g.first, spec, HOME).unwrap();
    assert!(g.session(copied).split.exists);
    assert!(!g.session(copied).focuses_split());
}

/// Upstream's `duplicateAsCompanionOfASinglePaneMakesNoSplit`, made in the background, and an unknown source.
#[test]
fn duplicate_as_companion_of_a_single_pane_makes_no_split() {
    let mut g = Group::new();
    let selected = g.store.selected_session_id();
    let spec = CompanionCopy {
        select: false,
        ..copy(pane("/srv", None), None)
    };
    let copied = g.store.duplicate_as_companion(g.parent, spec, HOME).unwrap();
    assert_eq!(g.session(copied).launch.command, None);
    assert!(!g.session(copied).split.exists);
    assert_eq!(g.store.selected_session_id(), selected);
    let unknown = g
        .store
        .duplicate_as_companion(Uuid::new_v4(), copy(pane("/srv", None), None), HOME);
    assert_eq!(unknown, Err(CompanionRefusal::NoSuchSession));
}

/// One foreground program carrying identities in whole arguments, an embedded assignment and a daemon name.
fn identity_arguments(session: &str, pane: &str, peer: &str, daemon: &str, unrelated: &str) -> Vec<String> {
    vec![
        "tool".to_owned(),
        format!("set.NGTERM_SESSION_ID=\"{session}\""),
        format!("--pane={pane}"),
        "--peer".to_owned(),
        peer.to_owned(),
        daemon.to_owned(),
        unrelated.to_owned(),
        "left:right".to_owned(),
    ]
}

fn program(cwd: &str, argv: Vec<String>) -> PaneCopy {
    PaneCopy {
        cwd: cwd.to_owned(),
        command: ORDINARY_DIALECT.line(&argv),
        argv: Some(argv),
    }
}

fn assert_persisted_pane(command: Option<&str>, argv: Option<&[String]>, expected: &[String]) {
    assert_eq!(argv, Some(expected));
    assert_eq!(command, ORDINARY_DIALECT.line(expected).as_deref());
}

/// An all-digit UUID also has a wholly lowercase spelling; random fixtures need not contain any letters.
fn rebound_identity(spelling: &str, destination: Uuid) -> String {
    let destination = uuid_string(destination);
    if spelling == spelling.to_ascii_lowercase() {
        destination.to_ascii_lowercase()
    } else {
        destination
    }
}

/// Upstream's `duplicateAsCompanionRebindsTheSourcesIdentitiesInBothPanes`, including both persisted argv copies.
#[test]
fn duplicate_as_companion_rebinds_the_sources_identities_in_both_panes() {
    let mut g = Group::new();
    g.store.set_split_visibility(g.first, true, None);
    let source = g.session(g.first);
    let source_id = uuid_string(g.first);
    let primary = uuid_string(source.pane_identity);
    let split_id = source.split_pane_identity.unwrap();
    let split = uuid_string(split_id);
    let unrelated = uuid_string(g.after);
    let mixed = source_id[..8].to_ascii_lowercase() + &source_id[8..];
    let left = identity_arguments(
        &source_id,
        &primary,
        &split.to_ascii_lowercase(),
        &crate::live::daemon_name(source.pane_identity),
        &unrelated,
    );
    let right = identity_arguments(
        &mixed,
        &split,
        &primary.to_ascii_lowercase(),
        &crate::live::daemon_name(split_id),
        &unrelated,
    );
    let spec = copy(program("/srv", left), Some(program("/var/log", right)));
    let copied = g.store.duplicate_as_companion(g.first, spec, HOME).unwrap();
    let session = g.session(copied);
    let copied_split = session.split_pane_identity.unwrap();
    let expected_left = identity_arguments(
        &rebound_identity(&source_id, copied),
        &rebound_identity(&primary, session.pane_identity),
        &uuid_string(copied_split).to_ascii_lowercase(),
        &crate::live::daemon_name(session.pane_identity),
        &unrelated,
    );
    let expected_right = identity_arguments(
        &rebound_identity(&mixed, copied),
        &rebound_identity(&split, copied_split),
        &uuid_string(session.pane_identity).to_ascii_lowercase(),
        &crate::live::daemon_name(copied_split),
        &unrelated,
    );
    assert_persisted_pane(
        session.launch.command.as_deref(),
        session.launch.argv.as_deref(),
        &expected_left,
    );
    assert_persisted_pane(
        session.split_launch.command.as_deref(),
        session.split_launch.argv.as_deref(),
        &expected_right,
    );
    assert_eq!(session.initial_cwd(), "/srv");
    assert_eq!(session.initial_split_cwd.as_deref(), Some("/var/log"));
    let snapshot = g.store.snapshot();
    let saved = snapshot.workspaces[0]
        .sessions
        .iter()
        .find(|saved| saved.id == copied)
        .unwrap();
    assert_persisted_pane(
        saved.initial_command.as_deref(),
        saved.initial_argv.as_deref(),
        &expected_left,
    );
    assert_persisted_pane(
        saved.split_initial_command.as_deref(),
        saved.split_initial_argv.as_deref(),
        &expected_right,
    );
}

/// Upstream's `duplicateAsCompanionPersistsTheReboundCommandOfASinglePane` also keeps its matching raw argv.
#[test]
fn duplicate_as_companion_persists_the_rebound_command_of_a_single_pane() {
    let mut g = Group::new();
    let spec = copy(
        program(
            "/srv",
            vec!["tool".to_owned(), "--session".to_owned(), uuid_string(g.parent)],
        ),
        None,
    );
    let copied = g.store.duplicate_as_companion(g.parent, spec, HOME).unwrap();
    let expected = vec![
        "tool".to_owned(),
        "--session".to_owned(),
        rebound_identity(&uuid_string(g.parent), copied),
    ];
    let snapshot = g.store.snapshot();
    let saved = snapshot.workspaces[0]
        .sessions
        .iter()
        .find(|saved| saved.id == copied)
        .unwrap();
    assert_persisted_pane(
        saved.initial_command.as_deref(),
        saved.initial_argv.as_deref(),
        &expected,
    );
    assert_eq!(saved.split_initial_command, None);
    assert_eq!(saved.split_initial_argv, None);
    assert_eq!(saved.cwd, "/srv");
}
