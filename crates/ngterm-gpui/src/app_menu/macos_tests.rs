//! The native menu's JSON, and what a click on one of its items runs, with a recording standing in for the `AppKit`
//! menu bar, which needs the main thread (`tests/macos_appkit_sweep.rs` drives the real one).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::TestAppContext;
use ngterm_core::menu_lists::MenuListAction;
use ngterm_core::model::WindowLibrary;
use ngterm_core::palette_catalog::PaletteCommand;
use ngterm_core::{CoreConfig, CoreState};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Request};

use super::super::Item;
use super::*;
use crate::keymap_dispatch::register_window;
use crate::live_runtime::ResetAlert;

fn core_with(library: WindowLibrary) -> Core {
    Core::new(
        CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            ..CoreState::default()
        },
    )
}

fn seeded(home: &str) -> Core {
    core_with(WindowLibrary::seeded(home))
}

/// A sync through a recording: the JSON the menu bar would be given, if any.
fn synced(core: &Core) -> Option<Value> {
    let mut json = None;
    sync_with(core, |value| {
        json = Some(serde_json::from_slice(value.to_bytes()).unwrap());
    });
    json
}

/// The first entry titled `title` in `entries` and their submenus.
fn entry<'a>(entries: &'a [Value], title: &str) -> &'a Value {
    entries
        .iter()
        .find_map(|value| find_title(value, title))
        .unwrap_or_else(|| panic!("no entry {title}"))
}

fn menu<'a>(json: &'a Value, title: &str) -> &'a [Value] {
    json["menus"]
        .as_array()
        .unwrap()
        .iter()
        .find(|menu| menu["title"] == title)
        .unwrap_or_else(|| panic!("no menu {title}"))["children"]
        .as_array()
        .unwrap()
}

fn action_of(tag: &Value) -> (Uuid, MenuAction) {
    let tag = usize::try_from(tag.as_u64().unwrap()).unwrap();
    REGISTRY.with_borrow(|registry| registry.actions[tag])
}

/// Adds a session, then closes it so Open Recent has an entry and the model changes.
fn close_a_new_session(core: &mut Core, name: &str) {
    let created = core.dispatch(&Request {
        cmd: Command::Session(SessionCmd::New),
        target: None,
        args: Some(Args {
            name: Some(name.to_owned()),
            ..Args::default()
        }),
    });
    assert!(created.ok, "{created:?}");
    let id = created.result.unwrap()["id"].as_str().unwrap().to_owned();
    let closed = core.dispatch(&Request {
        cmd: Command::Session(SessionCmd::Close),
        target: Some(id),
        args: None,
    });
    assert!(closed.ok, "{closed:?}");
    let _ = core.commit();
}

#[test]
fn the_json_gives_every_clickable_item_a_tag_that_finds_its_action() {
    let core = seeded("/home/u");
    let window = core.library().active_window_id().unwrap();
    let json = synced(&core).expect("the first sync sets the menu");
    let model = AppMenu::of(&core, window);

    let tops: Vec<_> = json["menus"]
        .as_array()
        .unwrap()
        .iter()
        .map(|menu| menu["title"].as_str().unwrap())
        .collect();
    assert_eq!(tops, ["File", "View", "Navigate", "Help"]);
    let app = json["app"].as_array().unwrap();
    let titles: Vec<_> = app.iter().map(|item| item["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Settings…", "Quit"]);
    assert_eq!(action_of(&app[1]["tag"]), (window, MenuAction::App(AppAction::Quit)));

    let mut tags = Vec::new();
    for item in app.iter().chain(json["menus"].as_array().unwrap()) {
        collect_tags(item, &mut tags);
    }
    let count = model.items().len();
    assert_eq!(tags.len(), count, "one tag per clickable item");
    tags.sort_unstable();
    let indexes: Vec<u64> = (0..u64::try_from(count).unwrap()).collect();
    assert_eq!(tags, indexes, "tags are the registry's indexes");
}

fn collect_tags(entry: &Value, tags: &mut Vec<u64>) {
    if let Some(tag) = entry["tag"].as_u64() {
        tags.push(tag);
    }
    for child in entry["children"].as_array().into_iter().flatten() {
        collect_tags(child, tags);
    }
}

#[test]
fn an_items_chord_and_state_reach_the_json() {
    let core = seeded("/home/u");
    let window = core.library().active_window_id().unwrap();
    let json = synced(&core).unwrap();
    let model = AppMenu::of(&core, window);
    let (_, with_chord) = model
        .items()
        .into_iter()
        .find(|(_, item)| item.chord.is_some())
        .expect("a seeded menu shows a chord");
    let chord = with_chord.chord.as_ref().unwrap();
    let file = menu(&json, "File");
    let view = menu(&json, "View");
    let navigate = menu(&json, "Navigate");
    let found = [file, view, navigate]
        .into_iter()
        .flatten()
        .find_map(|top| find_title(top, &with_chord.title))
        .unwrap_or_else(|| panic!("no {} in the JSON", with_chord.title));
    assert_eq!(found["key"], chord.key.as_str());
    assert_eq!(found["mods"], chord.mods.bits());
    assert_eq!(found["enabled"], with_chord.enabled);
    assert_eq!(found["checked"], with_chord.checked);

    let quit = entry(json["app"].as_array().unwrap(), "Quit");
    assert_eq!(quit["key"], "", "an item with no chord names no key");
    assert_eq!(quit["mods"], 0);
}

fn find_title<'a>(value: &'a Value, title: &str) -> Option<&'a Value> {
    if value["title"] == title {
        return Some(value);
    }
    value["children"]
        .as_array()?
        .iter()
        .find_map(|child| find_title(child, title))
}

#[test]
fn separators_labels_and_submenus_keep_their_shape() {
    let mut core = seeded("/home/u");
    close_a_new_session(&mut core, "doomed");
    let json = synced(&core).unwrap();
    let file = menu(&json, "File");
    assert!(
        file.iter()
            .any(|entry| *entry == serde_json::json!({"separator": true})),
        "{file:?}"
    );
    let recent = entry(file, "Open Recent");
    assert!(recent["children"].is_array(), "{recent}");
    assert!(recent.get("tag").is_none(), "a submenu is not clickable: {recent}");
    let heading = entry(file, "Sessions");
    assert_eq!(*heading, serde_json::json!({"title": "Sessions", "enabled": false}));
}

#[test]
fn an_unchanged_model_is_not_sent_again_and_a_changed_one_is_retagged() {
    let mut core = seeded("/home/u");
    let first = synced(&core).expect("the first sync sets the menu");
    assert_eq!(synced(&core), None, "nothing changed");
    assert!(!first.to_string().contains("doomed"));

    close_a_new_session(&mut core, "doomed");
    let second = synced(&core).expect("the model changed");
    assert!(second.to_string().contains("doomed"), "{second}");
    let window = core.library().active_window_id().unwrap();
    let count = AppMenu::of(&core, window).items().len();
    assert_eq!(
        REGISTRY.with_borrow(|registry| registry.actions.len()),
        count,
        "the earlier sync's tags are gone"
    );
}

#[test]
fn with_no_window_open_the_items_belong_to_no_window_and_stand_down() {
    let core = core_with(WindowLibrary::default());
    let json = synced(&core).expect("the first sync sets the menu");
    let first = entry(menu(&json, "File"), "New Session");
    assert_eq!(first["enabled"], false);
    assert_eq!(action_of(&first["tag"]).0, Uuid::nil());
}

#[derive(Default)]
struct Posted(Mutex<Vec<MainTask<AppState>>>);

impl MainLoop<AppState> for Posted {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        panic!("a menu click posts at once");
    }
}

impl Posted {
    /// Runs the tasks posted so far on `state`, returning how many there were.
    fn run_on(&self, state: &mut AppState) -> usize {
        let tasks = std::mem::take(&mut *self.0.lock().unwrap());
        let count = tasks.len();
        for task in tasks {
            task(state);
        }
        count
    }
}

fn poster_of(queue: &Arc<Posted>) -> (Arc<dyn MainLoop<AppState>>, Poster) {
    let main_loop: Arc<dyn MainLoop<AppState>> = queue.clone();
    let weak = Arc::downgrade(&main_loop);
    (main_loop, weak)
}

/// Clicks item `tag`, lets the loop run, and runs what it posted on `state`: how many tasks that was.
fn click(tag: usize, cx: &mut TestAppContext, queue: &Posted, state: &mut AppState) -> usize {
    clicked(tag);
    cx.run_until_parked();
    queue.run_on(state)
}

/// `EVENTS` is the process's, so use one context even when GPUI's `ITERATIONS` override is set. A click before
/// `init` is ignored; after two calls to `init`, each kind of action still reaches the first poster.
#[test]
fn each_native_click_runs_its_action_in_the_menus_window() {
    let cx = &mut TestAppContext::single();
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(seeded(dir.path().to_str().unwrap()));
    let window = state.core.library().active_window_id().unwrap();
    let unregistered = Uuid::new_v4();
    let actions = [
        (window, MenuAction::Command(Item::Row(PaletteCommand::NewSession))),
        (window, MenuAction::List(MenuListAction::RevealSession)),
        (window, MenuAction::App(AppAction::Quit)),
        (window, MenuAction::App(AppAction::ResetLiveSessions)),
        (window, MenuAction::App(AppAction::Settings)),
        (unregistered, MenuAction::App(AppAction::Settings)),
    ];
    REGISTRY.with_borrow_mut(|registry| registry.actions = actions.to_vec());
    let view = cx.add_empty_window();
    view.update(|shown, _| register_window(shown.window_handle(), KeyOrigin::Window(window)));

    let queue = Arc::new(Posted::default());
    let (_main_loop, poster) = poster_of(&queue);

    clicked(0);
    assert_eq!(queue.run_on(&mut state), 0, "before init a click has nowhere to go");

    cx.update(|cx| init(&poster, cx));
    let second = Arc::new(Posted::default());
    let (_second_loop, second_poster) = poster_of(&second);
    cx.update(|cx| init(&second_poster, cx));

    assert_eq!(
        click(usize::MAX, cx, &queue, &mut state),
        0,
        "an unknown tag is dropped"
    );

    core_items_run_through_the_core(cx, &queue, &mut state, window, dir.path());
    app_items_run_in_the_menus_window(cx, &queue, &mut state, window);

    assert_eq!(second.run_on(&mut state), 0, "no clicks reached the second poster");
    cx.quit();
}

/// Tags 0 and 1 of the click test: a palette command and a live entry.
fn core_items_run_through_the_core(
    cx: &mut TestAppContext,
    queue: &Posted,
    state: &mut AppState,
    window: Uuid,
    home: &std::path::Path,
) {
    let sessions = |state: &AppState| state.core.library().store(window).unwrap().sessions().count();
    let before = sessions(state);
    assert_eq!(click(0, cx, queue, state), 1);
    assert_eq!(sessions(state), before + 1, "a command runs through the core");
    let _ = state.take_frontend_work();

    assert_eq!(click(1, cx, queue, state), 1);
    assert_eq!(state.take_frontend_work().reveals, [home.to_path_buf()], "a live entry");
}

/// Tags 2 to 5 of the click test: the app's own items, and one whose window is gone.
fn app_items_run_in_the_menus_window(cx: &mut TestAppContext, queue: &Posted, state: &mut AppState, window: Uuid) {
    assert_eq!(click(2, cx, queue, state), 1);
    assert!(
        state.take_frontend_work().quit_prompt.is_some(),
        "Quit asks first with a window open"
    );

    assert_eq!(click(3, cx, queue, state), 1);
    assert!(
        matches!(state.take_frontend_work().live_reset, Some(ResetAlert::Refused(_))),
        "Reset Live Sessions reports why it cannot run"
    );

    assert_eq!(
        click(4, cx, queue, state),
        1,
        "an app item in a registered window runs over it"
    );
    assert_eq!(state.take_frontend_work().settings, Some(window));

    assert_eq!(click(5, cx, queue, state), 0, "the window that opened the menu is gone");
}
