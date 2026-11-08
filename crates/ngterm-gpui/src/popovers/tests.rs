//! The popovers in GPUI's test window over a real core: opening and closing (the core hears both), the keyboard's
//! row and Return running it, Escape and a press outside closing, a button that turned off closing its popover. The
//! popovers' posts land on a recording loop the test runs against the app state, then they sync as the window would.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::{Entity, TestAppContext, VisualTestContext, point, px, size};
use ngterm_core::model::{NewSession, WindowLibrary};
use ngterm_core::{Core, CoreConfig, CoreState};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use ngterm_proto::AppIdentity;

use super::*;

#[derive(Default)]
struct Posted(Mutex<Vec<MainTask<AppState>>>);

impl MainLoop<AppState> for Posted {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        Ok(())
    }
}

struct Harness {
    state: AppState,
    posted: Arc<Posted>,
    view: Entity<Popovers>,
    window: Uuid,
    sessions: Vec<Uuid>,
}

/// One workspace with sessions "u", "srv" and "var", each selected in turn, so the first two are recent.
fn harness(cx: &mut TestAppContext) -> (Harness, &mut VisualTestContext) {
    harness_with(cx, None, |_| {})
}

/// [`harness`] saving through `persistence`, over a core `prepare` adjusts first.
fn harness_with(
    cx: &mut TestAppContext,
    persistence: Option<ngterm_core::persist::LibraryPersistence>,
    prepare: impl FnOnce(&mut Core),
) -> (Harness, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let first = store.sessions().next().unwrap().id();
    let srv = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let var = store.add_session(workspace, NewSession::at_cwd("/var")).unwrap();
    for id in [first, srv, var] {
        store.select_session(Some(id));
    }
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            persistence,
            ..CoreState::default()
        },
    );
    prepare(&mut core);
    let posted = Arc::new(Posted::default());
    let weak: Poster = Arc::downgrade(&posted) as Poster;
    let mut state = AppState::new(core);
    state.attach(weak.clone());
    let (view, cx) = cx.add_window_view(move |_, cx| Popovers::new(window, weak, cx));
    cx.simulate_resize(size(px(800.), px(500.)));
    cx.update(|window, _| window.activate_window());
    let mut harness = Harness {
        state,
        posted,
        view,
        window,
        sessions: vec![first, srv, var],
    };
    harness.settle(cx);
    (harness, cx)
}

impl Harness {
    /// Runs what was posted and syncs the popovers, until nothing more is posted.
    fn settle(&mut self, cx: &mut VisualTestContext) {
        for _ in 0..8 {
            cx.run_until_parked();
            let tasks: Vec<MainTask<AppState>> = std::mem::take(&mut *self.posted.0.lock().unwrap());
            for task in tasks {
                task(&mut self.state);
            }
            let core = &self.state.core;
            self.view.update(cx, |popovers, cx| popovers.sync(core, cx));
            cx.run_until_parked();
            if self.posted.0.lock().unwrap().is_empty() {
                return;
            }
        }
    }

    fn open(&mut self, kind: PopoverKind, cx: &mut VisualTestContext) {
        let view = self.view.clone();
        cx.update(|window, cx| {
            view.update(cx, |popovers, cx| {
                popovers.toggle(kind, point(px(400.), px(48.)), window, cx);
            });
        });
        self.settle(cx);
    }

    fn selected(&self) -> Option<Uuid> {
        self.state
            .core
            .library()
            .store(self.window)
            .and_then(ngterm_core::model::AppStore::selected_session_id)
    }
}

fn holds_keyboard(view: &Entity<Popovers>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| view.read(cx).focus_handle().is_focused(window))
}

#[gpui_kit::test]
fn the_recent_popover_lists_and_runs_its_rows_by_keyboard(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    h.open(PopoverKind::Recent, cx);
    assert_eq!(h.view.read_with(cx, |p, _| p.open_kind()), Some(PopoverKind::Recent));
    let rows = h.view.read_with(cx, |p, _| p.row_titles());
    assert!(!rows.is_empty(), "the other sessions are recent: {rows:?}");
    assert!(
        !rows.contains(&"var".to_owned()),
        "the selected one is no recent row: {rows:?}"
    );
    assert!(holds_keyboard(&h.view, cx), "the popover takes the keyboard");

    cx.simulate_keystrokes("down enter");
    h.settle(cx);
    assert_eq!(h.view.read_with(cx, |p, _| p.open_kind()), None, "a run row closes it");
    let expected = h.sessions.iter().copied().find(|id| {
        h.state
            .core
            .library()
            .store(h.window)
            .unwrap()
            .session(*id)
            .unwrap()
            .display_name()
            == rows[0]
    });
    assert_eq!(h.selected(), expected, "the first row's session is selected");
}

#[gpui_kit::test]
fn escape_a_press_outside_and_a_second_click_close_it(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    h.open(PopoverKind::Recent, cx);
    cx.simulate_keystrokes("escape");
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "Escape closes");

    h.open(PopoverKind::Recent, cx);
    cx.simulate_click(point(px(20.), px(450.)), gpui_kit::Modifiers::default());
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "a press outside closes");
    let before = h.selected();
    assert_eq!(before, Some(h.sessions[2]), "and runs nothing");

    h.open(PopoverKind::Recent, cx);
    h.open(PopoverKind::Recent, cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "its button again closes it");
}

#[gpui_kit::test]
fn a_button_turned_off_closes_its_popover_and_opens_none(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    // the attention button is off by default: its popover never opens
    h.open(PopoverKind::Attention, cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()));

    h.open(PopoverKind::Recent, cx);
    assert!(h.view.read_with(cx, |p, _| p.is_open()));
    h.state.core.update_settings(|settings| {
        settings.hidden_interface_elements = Some(vec!["recentSessions".to_owned()]);
    });
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "its button went away");
}

/// A recent row draws its session's status glyph trailing, as the sidebar does; an idle row draws none.
#[gpui_kit::test]
fn a_recent_row_draws_its_status_glyph_trailing(cx: &mut TestAppContext) {
    use ngterm_core::model::{AgentIndicator, AgentStatus};

    let (mut h, cx) = harness(cx);
    h.open(PopoverKind::Recent, cx);
    assert!(cx.debug_bounds("agent-status").is_none(), "idle rows draw no glyph");
    h.view.update(cx, Popovers::close);
    h.settle(cx);
    let srv = h.sessions[1];
    h.state
        .core
        .library_mut()
        .store_mut(h.window)
        .unwrap()
        .set_agent_indicator(
            AgentIndicator {
                status: AgentStatus::Blocked,
                ..AgentIndicator::default()
            },
            srv,
        );
    h.settle(cx);
    h.open(PopoverKind::Recent, cx);
    let glyph = cx
        .debug_bounds("agent-status")
        .expect("the blocked row draws its glyph");
    let panel = cx.debug_bounds("popover").expect("the popover");
    assert!(
        glyph.origin.x > panel.origin.x + panel.size.width / 2.0,
        "the glyph trails the row: {glyph:?} in {panel:?}"
    );
}

/// Upstream's `testPopoversNameACompanionAfterItsParent`: the clock and the bell list a companion as `parent › child`.
#[gpui_kit::test]
fn the_popovers_name_a_companion_after_its_parent(cx: &mut TestAppContext) {
    use ngterm_core::model::{AgentIndicator, AgentStatus, CompanionOptions};

    let (mut h, cx) = harness(cx);
    h.state
        .core
        .update_settings(|settings| settings.attention_button_enabled = Some(true));
    let parent = h.sessions[2];
    let store = h.state.core.library_mut().store_mut(h.window).unwrap();
    store.rename_session(parent, "api");
    let options = CompanionOptions {
        name: Some("logs".to_owned()),
        ..CompanionOptions::default()
    };
    let companion = store.add_companion(parent, options, "/home/u").unwrap();
    store.set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        },
        companion,
    );
    store.select_session(Some(parent));
    h.settle(cx);

    h.open(PopoverKind::Recent, cx);
    let rows = h.view.read_with(cx, |p, _| p.row_titles());
    assert!(rows.contains(&"api › logs".to_owned()), "clock popover rows: {rows:?}");
    h.view.update(cx, Popovers::close);
    h.settle(cx);

    h.open(PopoverKind::Attention, cx);
    let rows = h.view.read_with(cx, |p, _| p.row_titles());
    assert_eq!(rows, ["api › logs"], "bell popover rows");
}

/// A core reading `keymap.conf` and the run counts from `dir`: six commands, "deploy" bound to a chord, "c3" run four
/// times, so it leads the list above the separator.
fn six_commands(dir: &std::path::Path) -> impl FnOnce(&mut Core) {
    let config = dir.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let keymap: String = std::iter::once("command \"deploy\" ctrl+alt+shift+j echo deploy\n".to_owned())
        .chain((2..=6).map(|index| format!("command \"c{index}\" true\n")))
        .collect();
    std::fs::write(config.join("keymap.conf"), keymap).unwrap();
    std::fs::write(
        dir.join("custom-command-usage.json"),
        r#"{"version": 1, "counts": {"c3": 4}}"#,
    )
    .unwrap();
    let dir = dir.to_owned();
    move |core: &mut Core| {
        let env = ngterm_core::paths::MapEnv::new().with("NGTERM_STATE_DIR", dir.as_os_str());
        core.set_config_locator(ngterm_core::paths::ConfigLocator::capture(&env));
        core.reload_keymap();
        core.update_settings(|settings| settings.shown_interface_elements = Some(vec!["customCommands".to_owned()]));
    }
}

/// Upstream's custom-commands popover: the most-run commands lead above a separator, the rest follow in file order,
/// the keyboard walks them both ways, wrapping, and Return runs the highlighted one and closes.
#[gpui_kit::test]
fn the_custom_commands_popover_leads_with_the_most_run_and_runs_by_keyboard(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let prepare = six_commands(dir.path());
    let persistence = ngterm_core::persist::LibraryPersistence::new(dir.path());
    let (mut h, cx) = harness_with(cx, Some(persistence), prepare);
    h.open(PopoverKind::CustomCommands, cx);
    assert_eq!(
        h.view.read_with(cx, |p, _| p.row_titles()),
        ["c3", "deploy", "c2", "c4", "c5", "c6"]
    );
    let panel = cx.debug_bounds("popover").unwrap();
    let separator = cx.update(|window, cx| {
        let color = crate::chrome::separator(&crate::chrome::current(cx));
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                quad.background.as_solid() == Some(color)
                    && (quad.bounds.size.height.0 / scale - 1.0).abs() < 0.1
                    && quad.bounds.size.width.0 / scale > f32::from(panel.size.width) / 2.0
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(separator.len(), 1, "one separator after the leading commands");
    // Up from no highlight lands on the last row, Down from there wraps to the first
    cx.simulate_keystrokes("up");
    assert_eq!(h.view.read_with(cx, |p, _| p.highlighted), Some(5));
    cx.simulate_keystrokes("down down");
    assert_eq!(h.view.read_with(cx, |p, _| p.highlighted), Some(1));
    // a letter and a key the list does not read move nothing
    cx.simulate_keystrokes("x f5");
    assert_eq!(h.view.read_with(cx, |p, _| p.highlighted), Some(1));
    cx.simulate_keystrokes("enter");
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "a run row closes it");
    assert_eq!(
        h.state.take_frontend_work().banners,
        vec![ngterm_core::NotificationEffect::App {
            identity: "command-failure:deploy".to_owned(),
            title: "Command failed".to_owned(),
            body: "deploy (custom commands are not available)".to_owned(),
        }],
        "Return ran exactly the highlighted command"
    );
}

/// A bound command draws its chord after the name; a palette-only command has no trailing label.
#[gpui_kit::test]
fn a_custom_command_line_draws_its_chord_at_the_trailing_edge(cx: &mut TestAppContext) {
    let (_, cx) = harness(cx);
    for shortcut in ["ctrl+alt+shift+j", ""] {
        let command = CustomCommand::new("deploy", "echo deploy", shortcut);
        let children = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = std::rc::Rc::clone(&children);
        let expected_width = cx.update(|window, _| {
            let run = window.text_style().to_run(shortcut.len());
            window
                .text_system()
                .shape_line(SharedString::from(shortcut.to_owned()), px(11.), &[run], None)
                .width
        });
        cx.draw(point(px(0.), px(0.)), size(px(300.), px(48.)), |_, cx| {
            command_line(
                div()
                    .flex()
                    .flex_row()
                    .size_full()
                    .on_children_prepainted(move |bounds, _, _| *captured.borrow_mut() = bounds)
                    .id("command-line"),
                &command,
                crate::chrome::current(cx),
            )
        });
        let children = children.borrow();
        assert_eq!(children.len(), if shortcut.is_empty() { 1 } else { 2 });
        if !shortcut.is_empty() {
            assert!(
                (children[1].size.width - expected_width).abs() < px(1.),
                "the shortcut's text at 11px"
            );
            assert!(children[0].right() <= children[1].left(), "the chord trails the name");
        }
    }
}

/// The bell's row selects its session in its window; opening it over the clock's popover swaps them without a second
/// opening for the core to hear.
#[gpui_kit::test]
fn the_attention_popover_replaces_the_recent_one_and_selects_its_row(cx: &mut TestAppContext) {
    use ngterm_core::model::{AgentIndicator, AgentStatus};

    let (mut h, cx) = harness_with(cx, None, |core| {
        core.update_settings(|settings| settings.attention_button_enabled = Some(true));
    });
    let srv = h.sessions[1];
    h.state
        .core
        .library_mut()
        .store_mut(h.window)
        .unwrap()
        .set_agent_indicator(
            AgentIndicator {
                status: AgentStatus::Blocked,
                ..AgentIndicator::default()
            },
            srv,
        );
    h.settle(cx);
    h.open(PopoverKind::Recent, cx);
    let view = h.view.clone();
    cx.update(|window, cx| {
        view.update(cx, |popovers, cx| {
            popovers.toggle(PopoverKind::Attention, point(px(400.), px(48.)), window, cx);
        });
    });
    assert!(
        h.posted.0.lock().unwrap().is_empty(),
        "one popover replaced another: the core heard the first opening only"
    );
    h.settle(cx);
    assert_eq!(h.view.read_with(cx, |p, _| p.open_kind()), Some(PopoverKind::Attention));
    assert_eq!(h.view.read_with(cx, |p, _| p.row_titles()), ["srv"]);
    // a row past the list runs nothing
    h.view.update(cx, |popovers, cx| popovers.activate(7, cx));
    assert!(h.view.read_with(cx, |p, _| p.is_open()));

    cx.simulate_keystrokes("down enter");
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()));
    assert_eq!(h.selected(), Some(srv), "the bell's row selected its session");
}

#[gpui_kit::test]
fn a_right_press_outside_closes_it_and_closing_none_posts_nothing(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    h.view.update(cx, Popovers::close);
    assert!(h.posted.0.lock().unwrap().is_empty(), "nothing was open");
    h.open(PopoverKind::Recent, cx);
    let at = point(px(20.), px(450.));
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, gpui_kit::Modifiers::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, gpui_kit::Modifiers::default());
    h.settle(cx);
    assert!(
        !h.view.read_with(cx, |p, _| p.is_open()),
        "a right press outside closes"
    );
    assert_eq!(h.selected(), Some(h.sessions[2]), "and runs nothing");
}

/// A remote session's recent row carries its cloud, and its title stays the session's name.
#[gpui_kit::test]
fn a_remote_recent_row_lists_with_its_cloud(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    let store = h.state.core.library_mut().store_mut(h.window).unwrap();
    let workspace = store.workspaces()[0].id();
    let remote = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("buildbox".to_owned()),
                name: Some("api".to_owned()),
                ..NewSession::at_cwd("/srv")
            },
        )
        .unwrap();
    store.select_session(Some(remote));
    store.select_session(Some(h.sessions[2]));
    h.settle(cx);
    h.open(PopoverKind::Recent, cx);
    let rows = h.view.read_with(cx, |p, _| p.recent.clone());
    let row = rows.iter().find(|row| row.session == remote).expect("the remote row");
    assert!(row.remote);
    assert_eq!(row.title, "api");
    cx.activate_a11y();
    let tree = cx.a11y_tree_update().expect("the popover's accessibility tree");
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.role() == gpui_kit::Role::MenuItem && node.label() == Some("api"))
    );
    // Inspect the same line builder the popover uses: the cloud takes its own 16px slot before the text.
    let children = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = std::rc::Rc::clone(&children);
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(48.)), |_, cx| {
        session_line(
            div()
                .flex()
                .flex_row()
                .size_full()
                .on_children_prepainted(move |bounds, _, _| *captured.borrow_mut() = bounds)
                .id("remote-line"),
            row,
            crate::chrome::current(cx),
        )
    });
    let children = children.borrow();
    assert_eq!(children.len(), 2, "the cloud and the session text");
    assert_eq!(children[0].size, size(px(row_icon::SLOT), px(row_icon::SLOT)));
    assert!(children[0].right() <= children[1].left(), "the cloud precedes the text");
}

/// A click on a row runs it: the pointer's row highlights under it, and the press goes no further than the popover.
#[gpui_kit::test]
fn a_click_on_a_row_runs_it(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    h.open(PopoverKind::Recent, cx);
    let first = h.view.read_with(cx, |p, _| p.recent[0].session);
    let panel = cx.debug_bounds("popover").expect("the popover");
    let at = point(panel.center().x, panel.origin.y + px(16.));
    cx.simulate_mouse_move(at, None, gpui_kit::Modifiers::default());
    cx.simulate_click(at, gpui_kit::Modifiers::default());
    h.settle(cx);
    assert!(!h.view.read_with(cx, |p, _| p.is_open()), "a run row closes it");
    assert_eq!(h.selected(), Some(first), "the clicked row's session is selected");
}

/// A row the core marks disabled draws dimmed and runs nothing, by keyboard or by click.
#[gpui_kit::test]
fn a_disabled_row_runs_nothing(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx);
    h.open(PopoverKind::Recent, cx);
    h.view.update(cx, |popovers, cx| {
        for row in &mut popovers.recent {
            row.enabled = false;
        }
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    let panel = cx.debug_bounds("popover").expect("the popover");
    cx.simulate_click(
        point(panel.center().x, panel.origin.y + px(16.)),
        gpui_kit::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(h.view.read_with(cx, |p, _| p.is_open()), "still open");
    assert!(h.posted.0.lock().unwrap().is_empty(), "nothing ran");
    assert_eq!(h.selected(), Some(h.sessions[2]));
}
