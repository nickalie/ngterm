//! The store half of Duplicate as Companion: upstream's three `AppStoreCompanionsTests.duplicateAsCompanion*` cases.

use uuid::Uuid;

use super::support::{add, ids};
use crate::model::{AppStore, CompanionCopy, CompanionOptions, CompanionRefusal, PaneCopy, Session, SplitAxis};

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

/// A copied pane in `cwd` running `argv` (its POSIX line the creation command), a plain shell without one.
fn pane(cwd: &str, argv: Option<&[&str]>) -> PaneCopy {
    let argv: Option<Vec<String>> = argv.map(|argv| argv.iter().map(|word| (*word).to_owned()).collect());
    PaneCopy {
        cwd: cwd.to_owned(),
        command: argv.as_deref().map(crate::command_restore::shell_quoted_line),
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
    assert_eq!(session.launch.command.as_deref(), Some("'htop' '-d' '5'"));
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
