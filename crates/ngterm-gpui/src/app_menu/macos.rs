//! `AppKit`'s menu bar shares the window menu's model and dispatch path. Edit uses the responder chain so `WebKit` owns editing.
use super::{AppAction, AppMenu, Entry, MenuAction, MenuItem};
use crate::{
    keymap_dispatch::{KeyOrigin, window_of},
    state::{AppState, Poster, post},
};
use gpui_kit::App;
use ngterm_core::Core;
use serde_json::{Value, json};
use std::{cell::RefCell, ffi::CString, sync::OnceLock};
use uuid::Uuid;

#[derive(Default)]
struct Registry {
    model: Option<(Uuid, AppMenu)>,
    actions: Vec<(Uuid, MenuAction)>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }
static EVENTS: OnceLock<async_channel::Sender<(Uuid, MenuAction)>> = OnceLock::new();
unsafe extern "C" {
    fn ngterm_menus_set(json: *const std::ffi::c_char, clicked: extern "C" fn(usize));
}
extern "C" fn clicked(tag: usize) {
    let action = REGISTRY.with(|registry| registry.borrow().actions.get(tag).copied());
    if let (Some(events), Some(action)) = (EVENTS.get(), action) {
        let _ = events.try_send(action);
    }
}
/// Queue native menu actions outside `AppKit`'s synchronous key dispatch.
pub fn init(poster: &Poster, cx: &mut App) {
    let (sender, receiver) = async_channel::unbounded();
    if EVENTS.set(sender).is_err() {
        return;
    }
    let poster = poster.clone();
    cx.spawn(async move |cx| {
        while let Ok((id, action)) = receiver.recv().await {
            cx.update(|cx| match action {
                MenuAction::Command(item) => post(&poster, move |state: &mut AppState| state.menu_activated(id, item)),
                MenuAction::List(list) => {
                    post(&poster, move |state: &mut AppState| state.menu_list_activated(id, list));
                }
                MenuAction::App(AppAction::Quit) => post(&poster, AppState::quit_from_menu),
                MenuAction::App(AppAction::ResetLiveSessions) => post(&poster, AppState::reset_live_sessions),
                MenuAction::App(_) => {
                    if let Some(handle) = window_of(KeyOrigin::Window(id)) {
                        let run = super::perform(&poster, id);
                        let _ = handle.update(cx, |_, window, cx| run(action, window, cx));
                    }
                }
            });
        }
    })
    .detach();
}
fn item(item: &MenuItem, id: Uuid, actions: &mut Vec<(Uuid, MenuAction)>) -> Value {
    let tag = actions.len();
    actions.push((id, item.action));
    let (key, mods) = item
        .chord
        .as_ref()
        .map_or(("", 0), |chord| (chord.key.as_str(), chord.mods.bits()));
    json!({"title":item.title,"enabled":item.enabled,"checked":item.checked,"tag":tag,"key":key,"mods":mods})
}
fn entries(items: &[Entry], id: Uuid, actions: &mut Vec<(Uuid, MenuAction)>) -> Vec<Value> {
    items
        .iter()
        .map(|entry| match entry {
            Entry::Item(value) => item(value, id, actions),
            Entry::Separator => json!({"separator":true}),
            Entry::Label(title) => json!({"title":title,"enabled":false}),
            Entry::Submenu {
                title,
                entries: children,
            } => json!({"title":title,"children":entries(children,id,actions)}),
        })
        .collect()
}
/// Refresh only when model titles, checks or availability change.
pub fn sync(core: &Core) {
    let id = core.library().active_window_id().unwrap_or_default();
    let model = AppMenu::of(core, id);
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if registry.model.as_ref() == Some(&(id, model.clone())) {
            return;
        }
        registry.actions.clear();
        let app: Vec<_> = model
            .app
            .iter()
            .map(|value| item(value, id, &mut registry.actions))
            .collect();
        let menus: Vec<_> = model
            .menus
            .iter()
            .map(|(title, items)| json!({"title":title,"children":entries(items,id,&mut registry.actions)}))
            .collect();
        let Ok(value) = CString::new(json!({"app":app,"menus":menus}).to_string()) else {
            return;
        };
        // SAFETY: main thread, native code copies JSON and retains only a static callback.
        unsafe {
            ngterm_menus_set(value.as_ptr(), clicked);
        }
        registry.model = Some((id, model));
    });
}
