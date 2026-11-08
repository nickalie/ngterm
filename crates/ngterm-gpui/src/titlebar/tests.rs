//! The title bar: hover texts and chord labels, the row per toolbar mode and decoration, the window controls per
//! layout, the traced glyphs (the GTK build's glyph tests), the glyph assets, and the bar over a core in GPUI's test
//! window following the toolbar mode, the toggles and the covers.

use gpui_kit::{AppContext as _, Entity, TestAppContext, Tiling, VisualTestContext, px, size};
use ngterm_core::model::WindowLibrary;
use ngterm_core::{CoreConfig, CoreState};
use ngterm_proto::AppIdentity;

use super::*;

#[test]
fn hover_texts_name_the_chord_when_bound() {
    assert_eq!(hint("Dashboard", Some("Shift+Ctrl+M")), "Dashboard (Shift+Ctrl+M)");
    assert_eq!(hint("Dashboard", None), "Dashboard");
}

#[test]
fn only_a_held_press_moves_the_window() {
    let mut pressed = true;
    assert!(starts_move(&mut pressed, Some(MouseButton::Left)));
    assert!(!pressed, "the move takes the press");
    assert!(
        !starts_move(&mut pressed, Some(MouseButton::Left)),
        "further movement during the held press starts no second drag"
    );
    let mut pressed = true;
    assert!(
        !starts_move(&mut pressed, Some(MouseButton::Right)),
        "only the primary button can drag the header"
    );
    assert!(!pressed);
    // a press whose release landed past the bar: the next hover ends it rather than moving the window
    let mut pressed = true;
    assert!(!starts_move(&mut pressed, None));
    assert!(!pressed);
    assert!(!starts_move(&mut pressed, Some(MouseButton::Left)), "no press, no move");
}

#[test]
fn chords_read_as_gtk_labels_them() {
    let chord = |mods, key: &str| Chord::new(mods, key);
    assert_eq!(
        chord_label(&chord(Modifiers::CONTROL | Modifiers::SHIFT, "m")),
        "Shift+Ctrl+M"
    );
    assert_eq!(chord_label(&chord(Modifiers::CONTROL, "tab")), "Ctrl+Tab");
    assert_eq!(
        chord_label(&chord(Modifiers::COMMAND | Modifiers::OPTION, "f5")),
        "Alt+Super+F5"
    );
    assert_eq!(chord_label(&chord(Modifiers::CONTROL, "=")), "Ctrl+=");
    assert_eq!(chord_label(&chord(Modifiers::empty(), "delete")), "BackSpace");
}

#[test]
fn the_row_follows_the_mode() {
    assert_eq!(height(ToolbarMode::Normal), NORMAL_HEIGHT);
    assert_eq!(height(ToolbarMode::Compact), COMPACT_HEIGHT);
    assert_eq!(
        height(ToolbarMode::Hidden),
        COMPACT_HEIGHT,
        "the minimal bar carries the app menu and the controls"
    );
}

#[test]
fn the_window_controls_follow_the_layout_and_what_the_window_manager_supports() {
    let all = WindowControls::default();
    let standard = default_button_layout();
    assert_eq!(
        controls(true, all, standard, false),
        [WindowButton::Minimize, WindowButton::Maximize, WindowButton::Close]
    );
    assert!(controls(true, all, standard, true).is_empty());
    assert!(
        controls(false, all, standard, false).is_empty(),
        "the window manager draws them"
    );
    let tiling_wm = WindowControls {
        minimize: false,
        maximize: false,
        ..WindowControls::default()
    };
    assert_eq!(controls(true, tiling_wm, standard, false), [WindowButton::Close]);
    let left = button_layout(&[WindowButton::Close, WindowButton::Minimize], &[]);
    assert_eq!(
        controls(true, all, left, true),
        [WindowButton::Close, WindowButton::Minimize]
    );
    assert!(controls(true, all, left, false).is_empty());
}

#[test]
fn the_bar_draws_the_controls_where_no_system_title_bar_does() {
    let client = Decorations::Client {
        tiling: Tiling::default(),
    };
    assert!(
        draws_controls(client),
        "a client-decorated window has no other title bar"
    );
    assert_eq!(
        draws_controls(Decorations::Server),
        cfg!(windows),
        "Windows hides its title bar for the app's; a window manager's title bar carries its own controls"
    );
}

/// The filled area of `glyph` left and right of the slot's middle, inside the frame.
fn fill_by_half(glyph: SplitGlyph) -> (f32, f32) {
    let (mut left, mut right) = (0.0, 0.0);
    for stroke in glyph_strokes(PaneGlyph::Split(glyph)) {
        // the frame and the divider line carry no pane
        if !stroke.fill || stroke.width <= 1.0 || stroke.height <= 1.0 {
            continue;
        }
        let area = stroke.width * stroke.height;
        if stroke.x + stroke.width <= 8.0 {
            left += area;
        } else if stroke.x >= 8.0 {
            right += area;
        }
    }
    (left, right)
}

#[test]
fn a_hidden_split_fills_only_the_pane_on_screen() {
    let (left, right) = fill_by_half(SplitGlyph::Left);
    assert!(left > 0.0 && right == 0.0, "{left} vs {right}");
    let (left, right) = fill_by_half(SplitGlyph::Right);
    assert!(right > 0.0 && left == 0.0, "{left} vs {right}");
    let (left, right) = fill_by_half(SplitGlyph::Both);
    assert!(left > 0.0 && right > 0.0);
    assert_eq!(fill_by_half(SplitGlyph::None), (0.0, 0.0), "an empty outline");
    // stacked: the halves are on top of each other
    let top: Vec<_> = glyph_strokes(PaneGlyph::Split(SplitGlyph::Top))
        .into_iter()
        .filter(|stroke| stroke.fill && stroke.height > 1.0)
        .collect();
    assert_eq!(top.len(), 1);
    assert!(top[0].y + top[0].height <= 8.0, "{top:?}");
}

#[test]
fn the_scratch_glyph_fills_while_it_shows() {
    let filled = |active| {
        glyph_strokes(PaneGlyph::Scratch(active))
            .iter()
            .filter(|stroke| stroke.fill)
            .count()
    };
    assert_eq!(filled(true), 1);
    assert_eq!(filled(false), 0);
}

#[test]
fn a_long_host_keeps_both_ends() {
    assert_eq!(middle_ellipsis("user@host", 30), "user@host");
    let cut = middle_ellipsis("deploy@build-runner-0042.eu-central.example.com", 30);
    assert_eq!(cut.chars().count(), 30);
    assert!(
        cut.starts_with("deploy@build-ru") && cut.ends_with("example.com"),
        "{cut}"
    );
    assert!(cut.contains('…'));
}

#[test]
fn every_glyph_the_bar_draws_is_served() {
    for path in glyph_paths() {
        assert!(serves(&path), "{path}");
    }
    // gpui-kit's own component icons stay reachable behind the bar's
    assert!(serves(&IconName::ChevronDown.path()));
}

fn core(settings: ngterm_core::persist::AppSettings) -> (Core, Uuid) {
    let library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings,
            persistence: None,
        },
    );
    (core, window)
}

fn colors() -> ChromeColors {
    chrome::colors(
        &crate::terminal::TerminalConfig::default(),
        &ngterm_core::persist::AppSettings::default(),
    )
}

/// The bar of `window` in a fresh test window, synced once over `core` under `cover`.
fn synced(cx: &mut TestAppContext, core: &Core, window: Uuid, cover: Cover) -> Bar {
    // the app menu's button is gpui-kit's, which reads its theme
    cx.update(gpui_kit::init);
    let (bar, cx) = cx.add_window_view(move |_, _| Titlebar::new(window, crate::state::detached_poster()));
    cx.simulate_resize(size(px(900.), px(600.)));
    bar.update(cx, |bar, cx| bar.sync(core, cover, false, colors(), cx));
    cx.run_until_parked();
    bar.read_with(cx, |bar, _| bar.bar().cloned().expect("synced"))
}

fn normal_mode() -> ngterm_core::persist::AppSettings {
    ngterm_core::persist::AppSettings {
        toolbar_mode: Some("normal".into()),
        attention_button_enabled: Some(true),
        ..Default::default()
    }
}

#[gpui_kit::test]
fn the_normal_bar_shows_both_lines_the_toggle_at_the_sidebars_edge_and_the_bell(cx: &mut TestAppContext) {
    let (core, window) = core(normal_mode());
    let shown = synced(cx, &core, window, Cover::None);
    assert_eq!(shown.model.mode, ToolbarMode::Normal);
    assert_eq!(shown.model.label.title, "u");
    assert_eq!(shown.model.label.subtitle, "/home/u", "normal mode's second line");
    assert!(shown.model.sidebar_toggle);
    let width = core.library().store(window).unwrap().sidebar().width;
    #[expect(clippy::cast_possible_truncation, reason = "a sidebar width fits an f32")]
    let width = width as f32;
    assert_eq!(shown.sidebar, Some(width), "the toggle sits at the sidebar's edge");
    assert!(
        shown.model.attention.is_some_and(|bell| !bell.enabled),
        "nothing needs attention"
    );
    assert_eq!(shown.hints.attention, "No sessions need attention (Shift+Ctrl+I)");
    assert!(shown.model.custom_commands.is_none(), "hidden by default");
    assert!(shown.model.dividers.after_jump && shown.model.dividers.after_panes);
}

#[gpui_kit::test]
fn a_cover_strips_the_bar_and_the_sidebar_steps_aside(cx: &mut TestAppContext) {
    let (core, window) = core(normal_mode());
    let zoomed = synced(cx, &core, window, Cover::Zoom);
    assert_eq!(zoomed.cover, Cover::Zoom);
    assert_eq!(zoomed.sidebar, None);
    let dashboard = synced(cx, &core, window, Cover::Dashboard("Dashboard".into()));
    assert_eq!(dashboard.cover, Cover::Dashboard("Dashboard".into()));
    assert_eq!(dashboard.sidebar, None);
}

#[gpui_kit::test]
fn the_hidden_and_compact_modes_and_the_interface_toggles(cx: &mut TestAppContext) {
    let (mut core, window) = core(normal_mode());
    core.update_settings(|settings| {
        settings.toolbar_mode = Some("hidden".into());
        settings.hidden_interface_elements = Some(vec!["recentSessions".into()]);
    });
    let hidden = synced(cx, &core, window, Cover::None);
    assert_eq!(hidden.model.mode, ToolbarMode::Hidden);
    assert_eq!(hidden.model.label, TitlebarComposition::default(), "no title");
    core.update_settings(|settings| settings.toolbar_mode = Some("compact".into()));
    let compact = synced(cx, &core, window, Cover::None);
    assert!(compact.model.recent.is_none());
    assert!(!compact.model.dividers.after_jump, "one jump button left");
    assert!(compact.model.label.subtitle.is_empty(), "compact: one line");
}

/// A bar of `window` over `core` in a fresh 900x600 test window (server-decorated, as GPUI's test platform reports).
fn shown_bar<'a>(
    cx: &'a mut TestAppContext,
    core: &Core,
    window: Uuid,
) -> (Entity<Titlebar>, &'a mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (bar, cx) = cx.add_window_view(move |_, _| Titlebar::new(window, crate::state::detached_poster()));
    cx.simulate_resize(size(px(900.), px(600.)));
    bar.update(cx, |bar, cx| bar.sync(core, Cover::None, false, colors(), cx));
    cx.run_until_parked();
    (bar, cx)
}

#[gpui_kit::test]
fn the_hidden_mode_keeps_the_app_menu_under_the_window_managers_decorations(cx: &mut TestAppContext) {
    let (mut core, window) = core(normal_mode());
    core.update_settings(|settings| settings.toolbar_mode = Some("hidden".into()));
    let (_bar, cx) = shown_bar(cx, &core, window);
    assert_eq!(cx.update(|window, _| window.window_decorations()), Decorations::Server);
    let button = cx
        .debug_bounds("app-menu-button")
        .expect("the app menu's only path stays");
    let row = cx.debug_bounds("titlebar").expect("a minimal bar");
    assert_eq!(row.size.height, COMPACT_HEIGHT);
    assert!(row.contains(&button.center()));
}

#[gpui_kit::test]
fn a_menu_whose_button_is_no_longer_drawn_is_closed(cx: &mut TestAppContext) {
    let (core, window) = core(normal_mode());
    let (bar, cx) = shown_bar(cx, &core, window);
    let at = cx.debug_bounds("app-menu-button").expect("drawn").center();
    cx.simulate_click(at, gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(bar.read_with(cx, |bar, _| bar.menu_open()), "the menu opened");
    // the button goes undismissed: no open-change callback runs for it
    bar.update(cx, |bar, cx| {
        bar.shown = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("app-menu-button").is_none());
    assert!(
        !bar.read_with(cx, |bar, _| bar.menu_open()),
        "the dropped menu holds the keyboard no longer"
    );
    // drawn again, the button opens a fresh menu
    bar.update(cx, |bar, cx| bar.sync(&core, Cover::None, false, colors(), cx));
    cx.run_until_parked();
    let at = cx.debug_bounds("app-menu-button").expect("drawn again").center();
    cx.simulate_click(at, gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(bar.read_with(cx, |bar, _| bar.menu_open()));
}

#[gpui_kit::test]
fn an_unchanged_sync_leaves_the_bar_alone(cx: &mut TestAppContext) {
    let (core, window) = core(ngterm_core::persist::AppSettings::default());
    let bar = cx.new(|_| Titlebar::new(window, crate::state::detached_poster()));
    let notified = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = std::rc::Rc::clone(&notified);
    let _watch = cx.update(|cx| cx.observe(&bar, move |_, _| counter.set(counter.get() + 1)));
    bar.update(cx, |bar, cx| bar.sync(&core, Cover::None, false, colors(), cx));
    bar.update(cx, |bar, cx| bar.sync(&core, Cover::None, false, colors(), cx));
    cx.run_until_parked();
    assert_eq!(notified.get(), 1);
    let mut lighter = colors();
    lighter.background = Rgb::WHITE;
    bar.update(cx, |bar, cx| bar.sync(&core, Cover::None, false, lighter, cx));
    cx.run_until_parked();
    assert_eq!(notified.get(), 2, "a chrome change repaints the bar");
}

#[test]
fn the_assets_list_the_bars_glyphs_once_each_in_order() {
    let listed = Icons.list("icons").unwrap();
    for path in glyph_paths() {
        assert!(listed.contains(&path), "{path} is listed");
    }
    let mut sorted = listed.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(listed, sorted, "sorted, with no path twice");
}

// MARK: - the bar's buttons over a recording loop

/// The main loop the bar posts to: tasks wait for the test to run them.
#[derive(Default)]
struct Posted(std::sync::Mutex<Vec<ngterm_host::MainTask<AppState>>>);

impl ngterm_host::MainLoop<AppState> for Posted {
    fn post(&self, task: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(
        &self,
        _: std::time::Duration,
        _: ngterm_host::MainTask<AppState>,
    ) -> Result<(), ngterm_host::MainLoopClosed> {
        Ok(())
    }
}

/// A window's bar over a real app state: what the bar posts runs against the state, and the bar syncs under the cover
/// the core shows, as the window's landing does.
struct Harness {
    state: AppState,
    posted: std::sync::Arc<Posted>,
    view: Entity<Titlebar>,
    window: Uuid,
    events: Rc<std::cell::RefCell<Vec<TitlebarEvent>>>,
    _events: gpui_kit::Subscription,
}

impl Harness {
    fn settle(&mut self, cx: &mut VisualTestContext) {
        for _ in 0..8 {
            cx.run_until_parked();
            let tasks: Vec<_> = std::mem::take(&mut *self.posted.0.lock().unwrap());
            for task in tasks {
                task(&mut self.state);
            }
            let core = &self.state.core;
            let window = self.window;
            let cover = if core.zoom_target(window).is_some() {
                Cover::Zoom
            } else if core.dashboard(window).is_some() {
                Cover::Dashboard("Dashboard".to_owned())
            } else {
                Cover::None
            };
            self.view
                .update(cx, |bar, cx| bar.sync(core, cover, false, colors(), cx));
            cx.run_until_parked();
            if self.posted.0.lock().unwrap().is_empty() {
                return;
            }
        }
    }

    fn store_mut(&mut self) -> &mut ngterm_core::model::AppStore {
        self.state.core.library_mut().store_mut(self.window).unwrap()
    }

    fn shown(&self, cx: &mut VisualTestContext) -> Bar {
        self.view.read_with(cx, |bar, _| bar.bar().cloned().unwrap())
    }
}

/// Sessions "u" and "srv" (selected, so "u" is recent) in a normal-mode bar with the bell on, `prepare` adjusting the
/// core first.
fn harness(cx: &mut TestAppContext, prepare: impl FnOnce(&mut Core)) -> (Harness, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (mut core, window) = core(normal_mode());
    let store = core.library_mut().store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let srv = store
        .add_session(workspace, ngterm_core::model::NewSession::at_cwd("/srv"))
        .unwrap();
    store.select_session(Some(srv));
    prepare(&mut core);
    let recorded = std::sync::Arc::new(Posted::default());
    let poster: Poster = std::sync::Arc::downgrade(&recorded) as Poster;
    let mut state = AppState::new(core);
    state.attach(poster.clone());
    let (view, cx) = cx.add_window_view(move |_, _| Titlebar::new(window, poster));
    cx.simulate_resize(size(px(900.), px(600.)));
    let events: Rc<std::cell::RefCell<Vec<TitlebarEvent>>> = Rc::default();
    let sink = Rc::clone(&events);
    let subscription =
        cx.update(|_, cx| cx.subscribe(&view, move |_, event: &TitlebarEvent, _| sink.borrow_mut().push(*event)));
    let mut harness = Harness {
        state,
        posted: recorded,
        view,
        window,
        events,
        _events: subscription,
    };
    harness.settle(cx);
    (harness, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    cx.simulate_click(at, gpui_kit::Modifiers::default());
}

/// A keymap with one custom command, read from `dir`, and the custom-commands button shown.
fn one_command(dir: &std::path::Path) -> impl FnOnce(&mut Core) {
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/keymap.conf"), "command \"deploy\" true\n").unwrap();
    let dir = dir.to_owned();
    move |core: &mut Core| {
        let env = ngterm_core::paths::MapEnv::new().with("NGTERM_STATE_DIR", dir.as_os_str());
        core.set_config_locator(ngterm_core::paths::ConfigLocator::capture(&env));
        core.reload_keymap();
        core.update_settings(|settings| {
            settings.shown_interface_elements = Some(vec!["customCommands".to_owned()]);
        });
    }
}

/// The clock, the bell and the custom-commands button each ask for their popover, hung from the pointer at the bar's
/// bottom edge; the bell turns the blocked color and says what it shows while a session waits on the user.
#[gpui_kit::test]
fn the_popover_buttons_ask_for_their_popover_under_the_pointer(cx: &mut TestAppContext) {
    use ngterm_core::model::{AgentIndicator, AgentStatus};
    let dir = tempfile::tempdir().unwrap();
    let commands = one_command(dir.path());
    let (mut h, cx) = harness(cx, commands);
    let u = h.store_mut().sessions().next().unwrap().id();
    h.store_mut().set_agent_indicator(
        AgentIndicator {
            status: AgentStatus::Blocked,
            ..AgentIndicator::default()
        },
        u,
    );
    h.settle(cx);
    let shown = h.shown(cx);
    assert_eq!(
        shown.model.attention.map(|bell| bell.bell),
        Some(AttentionBell::Blocked)
    );
    assert_eq!(
        shown.hints.attention,
        "Show sessions that need attention (Shift+Ctrl+I)"
    );
    assert_eq!(shown.blocked, BLOCKED_DEFAULT);
    for (selector, kind) in [
        ("recent-sessions-button", PopoverKind::Recent),
        ("attention-button", PopoverKind::Attention),
        ("custom-commands-button", PopoverKind::CustomCommands),
    ] {
        click(cx, selector);
        cx.run_until_parked();
        let event = h.events.borrow_mut().pop();
        let Some(TitlebarEvent::Popover { kind: asked, anchor }) = event else {
            panic!("{selector} asked for nothing");
        };
        assert_eq!(asked, kind, "{selector}");
        assert_eq!(
            anchor.y, NORMAL_HEIGHT,
            "{selector} hangs it from the bar's bottom edge"
        );
        let button = cx.debug_bounds(selector).unwrap();
        assert!(
            (anchor.x - button.center().x).abs() < px(1.),
            "{selector}: under the pointer"
        );
    }
}

/// The scratch and split toggles run their actions and their glyphs and hover texts follow; the dashboard button opens
/// the dashboard, which strips the bar to its exit, and the exit closes it again.
#[gpui_kit::test]
fn the_pane_and_dashboard_buttons_run_their_actions(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx, |_| {});
    let hint = h.shown(cx).hints.scratch;
    assert!(hint.starts_with("Show scratch terminal"), "{hint}");
    click(cx, "scratch-toggle");
    h.settle(cx);
    let shown = h.shown(cx);
    assert!(
        shown.model.scratch.is_some_and(|scratch| scratch.active),
        "the scratch shows"
    );
    assert!(
        shown.hints.scratch.starts_with("Hide scratch terminal"),
        "{}",
        shown.hints.scratch
    );

    // over the scratch, the split toggle puts the scratch away first
    click(cx, "split-toggle");
    h.settle(cx);
    assert!(h.shown(cx).model.scratch.is_some_and(|scratch| !scratch.active));
    // a split on screen fills both halves of the glyph
    let session = h.store_mut().selected_session_id().unwrap();
    h.store_mut().set_split_visibility(session, true, None);
    h.settle(cx);
    let split = h.shown(cx).model.split.unwrap();
    assert_eq!((split.glyph, split.hint), (SplitGlyph::Both, "Hide split"));

    click(cx, "dashboard-toggle-button");
    h.settle(cx);
    assert!(h.state.core.dashboard(h.window).is_some(), "the dashboard opened");
    assert!(
        cx.debug_bounds("scratch-toggle").is_none(),
        "the cover strips the cluster"
    );
    click(cx, "dashboard-exit");
    h.settle(cx);
    assert!(h.state.core.dashboard(h.window).is_none(), "its exit closed it");
    assert!(cx.debug_bounds("dashboard-exit").is_none());
}

#[gpui_kit::test]
fn the_zooms_exit_ends_the_zoom(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx, |_| {});
    let session = h.store_mut().selected_session_id().unwrap();
    let target = ngterm_core::zoom::SurfaceId {
        session,
        surface: ngterm_core::zoom::ZoomSurface::Primary,
    };
    let window = h.window;
    h.state.core.set_zoom_target(window, Some(target));
    h.settle(cx);
    assert_eq!(h.shown(cx).cover, Cover::Zoom);
    click(cx, "terminal-zoom-exit");
    h.settle(cx);
    assert_eq!(h.state.core.zoom_target(window), None);
    assert_eq!(h.shown(cx).cover, Cover::None);
}

/// A remote session's title carries its host; the compact bar puts the session's context after the title on one line.
#[gpui_kit::test]
fn a_remote_hosts_title_names_the_host_and_the_compact_bar_its_context(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx, |core| {
        core.update_settings(|settings| settings.toolbar_mode = Some("compact".into()));
    });
    let store = h.store_mut();
    let workspace = store.workspaces()[0].id();
    let remote = store
        .add_session(
            workspace,
            ngterm_core::model::NewSession {
                remote_host: Some("deploy@build-runner-0042.eu-central.example.com".to_owned()),
                ..ngterm_core::model::NewSession::at_cwd("/srv")
            },
        )
        .unwrap();
    store.select_session(Some(remote));
    store.set_context(Some("deploying".to_owned()), remote);
    h.settle(cx);
    let label = h.shown(cx).model.label;
    assert_eq!(
        label.host.as_deref(),
        Some("deploy@build-runner-0042.eu-central.example.com")
    );
    assert!(label.tail.ends_with("deploying"), "{label:?}");
    assert!(label.subtitle.is_empty(), "compact: one line");
    let row = cx.debug_bounds("titlebar").unwrap();
    assert_eq!(row.size.height, COMPACT_HEIGHT);
}

/// A primary press on the bar may become a window move and its release ends it; a secondary press under the window
/// manager's own title bar opens no menu of the bar's.
#[gpui_kit::test]
fn a_press_on_the_bar_is_held_until_its_release(cx: &mut TestAppContext) {
    let (h, cx) = harness(cx, |_| {});
    let row = cx.debug_bounds("titlebar").unwrap();
    // the empty middle of the bar, between the title and the cluster
    let at = gpui_kit::point(row.origin.x + px(400.), row.center().y);
    let pressed = |h: &Harness, cx: &mut VisualTestContext| h.view.read_with(cx, |bar, _| bar.pressed);
    cx.simulate_mouse_down(at, MouseButton::Left, gpui_kit::Modifiers::default());
    assert!(pressed(&h, cx), "a primary press may move the window");
    cx.simulate_mouse_up(at, MouseButton::Left, gpui_kit::Modifiers::default());
    assert!(!pressed(&h, cx), "its release ends it");
    cx.simulate_mouse_down(at, MouseButton::Right, gpui_kit::Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, gpui_kit::Modifiers::default());
    assert!(!pressed(&h, cx), "a secondary press moves nothing");
    // a press elsewhere ends a press the bar saw
    cx.simulate_mouse_down(at, MouseButton::Left, gpui_kit::Modifiers::default());
    cx.simulate_mouse_down(
        gpui_kit::point(px(450.), px(400.)),
        MouseButton::Left,
        gpui_kit::Modifiers::default(),
    );
    assert!(!pressed(&h, cx));
}

/// The app menu's opening and closing ask the window for a sync, which hands the keyboard over and back.
#[gpui_kit::test]
fn the_app_menus_opening_and_closing_each_ask_for_a_sync(cx: &mut TestAppContext) {
    let (mut h, cx) = harness(cx, |_| {});
    click(cx, "app-menu-button");
    cx.run_until_parked();
    assert!(h.view.read_with(cx, |bar, _| bar.menu_open()));
    assert_eq!(h.posted.0.lock().unwrap().len(), 1, "the opening's sync");
    h.settle(cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!h.view.read_with(cx, |bar, _| bar.menu_open()));
    assert_eq!(h.posted.0.lock().unwrap().len(), 1, "the closing's sync");
    h.settle(cx);
}

/// After the quit save the bar's buttons change nothing.
#[gpui_kit::test]
fn the_buttons_do_nothing_after_the_quit_save(cx: &mut TestAppContext) {
    let (mut h, _cx) = harness(cx, |_| {});
    let window = h.window;
    h.state.core.toggle_dashboard(window);
    h.state.save_on_quit();
    let visible = h.store_mut().sidebar().visible;
    h.state.titlebar_action(window, TitlebarAction::ToggleSidebar);
    assert_eq!(h.store_mut().sidebar().visible, visible);
    h.state.exit_dashboard(window);
    assert!(h.state.core.dashboard(window).is_some(), "still open");
}
