//! How a flat list names and orders companions (`AppStore::list_label`, `AppStore::list_ordered`,
//! `WindowLibrary::attention_label`), ported from upstream's `AppStoreNameSanitizeTests` (the four list cases) and
//! `WindowLibraryAttentionTests.anAttentionCompanionIsLabelledAfterItsParent`.

use uuid::Uuid;

use super::support::add;
use crate::model::{AgentIndicator, AgentStatus, AppStore, CompanionOptions, Session, WindowLibrary};

const HOME: &str = "/home/u";

fn named(name: &str) -> CompanionOptions {
    CompanionOptions {
        name: Some(name.to_owned()),
        ..CompanionOptions::default()
    }
}

fn companion(store: &mut AppStore, parent: Uuid, name: &str) -> Uuid {
    store
        .add_companion(parent, named(name), HOME)
        .expect("the companion is created")
}

fn session_named(store: &mut AppStore, workspace: Uuid, cwd: &str, name: &str) -> Uuid {
    let id = add(store, workspace, cwd);
    store.rename_session(id, name);
    id
}

fn sessions<'a>(store: &'a AppStore, ids: &[Uuid]) -> Vec<&'a Session> {
    ids.iter()
        .map(|id| store.session(*id).expect("the session exists"))
        .collect()
}

fn ordered(store: &AppStore, ids: &[Uuid]) -> Vec<Uuid> {
    store
        .list_ordered(&sessions(store, ids))
        .into_iter()
        .map(Session::id)
        .collect()
}

fn label(store: &AppStore, id: Uuid) -> String {
    store.list_label(store.session(id).expect("the session exists"))
}

#[test]
fn list_label_names_a_companion_after_its_parent() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = session_named(&mut store, work, "/parent", "api");
    let after = add(&mut store, work, "/after");
    let first = companion(&mut store, parent, "logs");

    assert_eq!(label(&store, parent), "api");
    assert_eq!(label(&store, first), "api › logs");
    assert_eq!(label(&store, after), store.session(after).unwrap().display_name());

    store.rename_session(parent, "web");
    assert_eq!(label(&store, first), "web › logs");
}

#[test]
fn list_order_keeps_each_group_together_under_identically_named_parents() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let zeta = session_named(&mut store, work, "/z", "zeta");
    let first = session_named(&mut store, work, "/a", "api");
    let second = session_named(&mut store, work, "/b", "api");
    let first_child = companion(&mut store, first, "zz");
    let second_child = companion(&mut store, second, "aa");

    let all: Vec<Uuid> = store
        .workspace(work)
        .unwrap()
        .sessions
        .iter()
        .map(Session::id)
        .collect();

    assert_eq!(ordered(&store, &all), [first, first_child, second, second_child, zeta]);
}

#[test]
fn a_companion_listed_ahead_of_its_parent_still_follows_it() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let first = session_named(&mut store, work, "/a", "api");
    let second = session_named(&mut store, work, "/b", "api");
    let first_child = companion(&mut store, first, "a");
    let second_child = companion(&mut store, second, "b");

    assert_eq!(
        ordered(&store, &[first_child, first, second, second_child]),
        [first, first_child, second, second_child]
    );
}

#[test]
fn a_companion_listed_without_its_parent_sorts_on_its_own_label() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let parent = session_named(&mut store, work, "/parent", "m");
    let after = session_named(&mut store, work, "/after", "a");
    let first = companion(&mut store, parent, "c");

    assert_eq!(ordered(&store, &[first, after]), [after, first]);
}

#[test]
fn list_order_ignores_case() {
    let mut store = AppStore::new();
    let work = store.add_workspace("work");
    let upper = session_named(&mut store, work, "/b", "Beta");
    let lower = session_named(&mut store, work, "/a", "alpha");

    assert_eq!(ordered(&store, &[upper, lower]), [lower, upper]);
}

#[test]
fn an_attention_companion_is_labelled_after_its_parent() {
    let mut library = WindowLibrary::new(HOME);
    let window = library.new_window(None);
    let store = library.store_mut(window).expect("the window is open");
    let workspace = store.workspaces()[0].id();
    let parent = session_named(store, workspace, "/tmp", "api");
    let child = companion(store, parent, "logs");
    store.set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        },
        child,
    );

    let entries = library.attention_across_windows();
    let entry = entries
        .iter()
        .find(|entry| entry.session.id() == child)
        .expect("the companion needs attention");

    assert_eq!(library.attention_label(entry), "api › logs");
}
