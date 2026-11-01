//! Companions in the flat lists and New Companion, ported from upstream's `AppActionsPaletteTests` (the session
//! palette's grouping, the flagged child dragged ahead of its parent, the attention row), the core half of
//! `RecentSessionsButtonUITests.testPopoversNameACompanionAfterItsParent` and `AppActionsCompanionsTests`' New
//! Companion: every flat list names a companion `parent › child`.

use uuid::Uuid;

use super::{Fixture, wire};
use crate::app_actions::{ActionEffect, ActionOrigin};
use crate::keymap::BuiltinAction;
use crate::model::{AgentIndicator, AgentStatus, CompanionOptions, FlaggedOrderPlacement, SidebarMode};
use crate::palette::{PaletteEntry, PaletteMode, filter_entries};
use crate::palette_catalog::PaletteCommand;

fn titles(entries: &[PaletteEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.title.as_str()).collect()
}

impl Fixture {
    /// A session named `name` appended to the first workspace.
    fn named(&mut self, name: &str) -> Uuid {
        let id = self.add_session(self.workspace);
        self.store_mut().unwrap().rename_session(id, name);
        id
    }

    /// A companion of `parent` named `name`, created in the background.
    fn companion(&mut self, parent: Uuid, name: &str) -> Uuid {
        let options = CompanionOptions {
            name: Some(name.to_owned()),
            select: false,
            ..CompanionOptions::default()
        };
        self.store_mut()
            .unwrap()
            .add_companion(parent, options, "/home/u")
            .expect("the companion is created")
    }

    /// Renames every session of the first workspace "zeta", so the seeded ones sort last.
    fn rename_all_zeta(&mut self) {
        for id in self.order(self.workspace) {
            self.store_mut().unwrap().rename_session(id, "zeta");
        }
    }

    fn blocked(&mut self, id: Uuid) {
        let indicator = AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        };
        self.store_mut().unwrap().set_agent_indicator(indicator, id);
    }
}

#[test]
fn session_rows_group_companions_under_identically_named_parents() {
    let mut fixture = Fixture::new(0);
    fixture.rename_all_zeta();
    let first = fixture.named("api");
    let second = fixture.named("api");
    fixture.companion(first, "zz");
    fixture.companion(second, "aa");

    let sessions = fixture.core.palette_entries(PaletteMode::Sessions);
    assert_eq!(titles(&sessions)[..4], ["api", "api › zz", "api", "api › aa"]);
    let editor = fixture.core.palette_entries(PaletteMode::FlaggedEditor);
    let titles = titles(&editor);
    assert_eq!(titles[titles.len() - 4..], ["api", "api › zz", "api", "api › aa"]);
}

#[test]
fn a_flagged_child_ahead_of_its_parent_still_lists_under_it_in_the_session_palette() {
    let mut fixture = Fixture::new(0);
    fixture.rename_all_zeta();
    let first = fixture.named("api");
    let second = fixture.named("api");
    let first_child = fixture.companion(first, "a");
    let second_child = fixture.companion(second, "b");
    let store = fixture.store_mut().unwrap();
    store.set_flags(true, &[first, first_child, second, second_child]);
    assert!(store.move_flagged(&[first_child], FlaggedOrderPlacement::Before, first));
    store.set_sidebar_mode(SidebarMode::Flagged);

    let entries = fixture.core.palette_entries(PaletteMode::Sessions);
    let rows: Vec<&str> = filter_entries(&entries, "", PaletteMode::Sessions)
        .into_iter()
        .map(|index| entries[index].id.as_str())
        .collect();
    let want: Vec<String> = [first, first_child, second, second_child].map(wire).into();
    assert_eq!(rows, want);

    let reversed: Vec<PaletteEntry> = entries.iter().rev().cloned().collect();
    let a_to_z: Vec<&str> = filter_entries(&reversed, "", PaletteMode::Actions)
        .into_iter()
        .map(|index| reversed[index].title.as_str())
        .collect();
    assert_eq!(a_to_z, ["api", "api", "api › a", "api › b"]);
}

#[test]
fn an_attention_companion_row_names_its_parent_in_the_palette_and_the_bell() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.named("api");
    let child = fixture.companion(parent, "logs");
    fixture.blocked(child);

    let entries = fixture.core.palette_entries(PaletteMode::Attention);
    let row = entries.iter().find(|entry| entry.id == wire(child)).unwrap();
    assert_eq!(row.title, "api › logs");
    let bell = fixture.core.attention_rows();
    let row = bell.iter().find(|row| row.session == child).unwrap();
    assert_eq!(row.title, "api › logs");
}

#[test]
fn the_recent_sessions_popover_names_a_companion_after_its_parent() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.named("api");
    let child = fixture.companion(parent, "logs");
    let store = fixture.store_mut().unwrap();
    store.select_session(Some(child));
    store.select_session(Some(parent));

    let rows = fixture.core.recent_session_rows(fixture.window);
    let row = rows.iter().find(|row| row.session == child).unwrap();
    assert_eq!(row.title, "api › logs");
    assert!(
        rows.iter().all(|row| row.session != parent),
        "the current session is left out"
    );
}

#[test]
fn new_companion_adds_a_selected_companion_of_the_active_session() {
    let mut fixture = Fixture::new(1);
    let [first, second] = [fixture.sessions[0], fixture.sessions[1]];
    let origin = ActionOrigin {
        window: fixture.window,
        quick: false,
    };
    assert_eq!(
        fixture.core.perform_builtin(origin, BuiltinAction::NewCompanion),
        ActionEffect::Done
    );
    let child = fixture.selected().unwrap();
    assert_eq!(fixture.store().session(child).unwrap().parent_id(), Some(second));
    assert_eq!(fixture.order(fixture.workspace), [first, second, child]);

    // from a companion, the menu command adds a sibling at the group's end
    fixture
        .core
        .perform_menu_command(fixture.window, PaletteCommand::NewCompanion);
    let sibling = fixture.selected().unwrap();
    assert_eq!(fixture.store().session(sibling).unwrap().parent_id(), Some(second));
    assert_eq!(fixture.order(fixture.workspace), [first, second, child, sibling]);
}
