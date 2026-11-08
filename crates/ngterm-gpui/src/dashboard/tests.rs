//! The dashboard's grid shape, captions, keys, status chip and its pulse, and the font report rules (ported from the
//! GTK build's `dashboard.rs` and `surface.rs` tests).

use gpui_kit::{Keystroke, Modifiers};
use ngterm_core::model::{AgentIndicator, AgentStatus, NewSession, SessionSeed, SplitLayout};

use super::*;
use crate::terminal::font_report;

fn key(name: &str, modifiers: Modifiers) -> Keystroke {
    Keystroke {
        modifiers,
        key: name.to_owned(),
        key_char: None,
    }
}

#[test]
fn the_rows_fill_row_major_and_pad_a_ragged_last_row() {
    assert_eq!(rows(1), vec![vec![Some(0)]]);
    assert_eq!(rows(3), vec![vec![Some(0), Some(1)], vec![Some(2), None]]);
    assert_eq!(
        rows(5),
        vec![vec![Some(0), Some(1), Some(2)], vec![Some(3), Some(4), None]]
    );
    assert_eq!(
        rows(12).iter().flatten().flatten().count(),
        ngterm_core::dashboard::MAX_CELLS,
        "capped"
    );
    assert!(rows(0).is_empty());
}

#[test]
fn the_pane_marker_follows_the_split_axis() {
    let mut session = Session::new(SessionSeed::default());
    let id = session.id();
    assert_eq!(pane_marker(DashboardMember::primary(id), &session), "");
    session.split = SplitLayout {
        exists: true,
        ..SplitLayout::default()
    };
    assert_eq!(pane_marker(DashboardMember::primary(id), &session), " ◀");
    assert_eq!(pane_marker(DashboardMember::split(id), &session), " ▶");
    session.split.axis = SplitAxis::TopBottom;
    assert_eq!(pane_marker(DashboardMember::primary(id), &session), " ▲");
    assert_eq!(pane_marker(DashboardMember::split(id), &session), " ▼");
}

#[test]
fn the_grid_reads_bare_arrows_enter_and_escape() {
    let none = Modifiers::default();
    assert_eq!(grid_key(&key("left", none)), Some(GridKey::Move(Direction::Left)));
    assert_eq!(grid_key(&key("down", none)), Some(GridKey::Move(Direction::Down)));
    assert_eq!(grid_key(&key("enter", none)), Some(GridKey::Enter));
    assert_eq!(grid_key(&key("kp_enter", none)), Some(GridKey::Enter));
    assert_eq!(grid_key(&key("escape", none)), Some(GridKey::Close));
    assert_eq!(grid_key(&key("a", none)), None);
    assert_eq!(
        grid_key(&key("left", Modifiers::control())),
        None,
        "a chord is no grid key"
    );
}

#[test]
fn the_model_names_each_cell_marks_the_highlight_and_carries_the_status() {
    let mut store = AppStore::new();
    let workspace = store.add_workspace("work");
    let id = store.add_session(workspace, NewSession::at_cwd("/srv/app")).unwrap();
    store.set_split_visibility(id, true, None);
    store.session_mut(id).unwrap().agent_indicator = AgentIndicator {
        status: AgentStatus::Blocked,
        blink: true,
        ..AgentIndicator::default()
    };
    let model = GridModel {
        members: vec![DashboardMember::primary(id), DashboardMember::split(id)],
        highlighted: Some(DashboardMember::split(id)),
        font_size: None,
    };
    let cells = model.cells(Some(&store), &AppSettings::default());
    assert_eq!(cells[0].caption, "app ◀");
    assert_eq!(cells[1].caption, "app ▶");
    assert_eq!(
        cells.iter().map(|cell| cell.highlighted).collect::<Vec<_>>(),
        [false, true]
    );
    let status = cells[0].status.as_ref().expect("a blocked chip");
    assert_eq!(status.status, AgentStatus::Blocked);
    assert!(status.blink);
    assert_eq!(model.tokens(Some(&store)), [None, None], "no terminals yet");
    assert!(cells.iter().all(|cell| cell.cover.is_none()));
}

/// Upstream's `testACompanionCaptionNamesItsParent`.
#[test]
fn a_companion_caption_names_its_parent() {
    let mut store = AppStore::new();
    let workspace = store.add_workspace("work");
    let parent = store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap();
    store.rename_session(parent, "api");
    let options = ngterm_core::model::CompanionOptions {
        name: Some("logs".to_owned()),
        ..ngterm_core::model::CompanionOptions::default()
    };
    let companion = store.add_companion(parent, options, "/home/u").unwrap();
    let model = GridModel {
        members: vec![DashboardMember::primary(companion), DashboardMember::primary(parent)],
        highlighted: None,
        font_size: None,
    };
    let cells = model.cells(Some(&store), &AppSettings::default());
    assert_eq!(cells[0].caption, "api › logs");
    assert_eq!(cells[1].caption, "api");
}

#[test]
fn the_pulse_rises_to_its_peak_and_back() {
    assert!(pulse_at(0.0).abs() < 1e-6);
    assert!((pulse_at(0.5) - WASH_PEAK).abs() < 1e-6);
    assert!(pulse_at(1.0).abs() < 1e-6);
    assert!(pulse_at(0.25) > 0.0 && pulse_at(0.25) < WASH_PEAK);
}

/// Upstream's `dashboardPillColor` / `dashboardPillTextColor`: an idle chip is the theme's selection pill (the selected
/// sidebar row's colors), the foreground at 0.22 over the backdrop without one; a status chip is the status color.
#[test]
fn an_idle_chip_takes_the_themes_selection_colors_else_a_foreground_wash() {
    let rgb = |r, g, b| ngterm_core::chrome::Rgb::new(r, g, b);
    let mut cell = ChromeCell {
        member: DashboardMember::primary(uuid::Uuid::nil()),
        caption: "app".to_owned(),
        highlighted: false,
        cover: None,
        status: None,
    };
    let themed = Look {
        background: rgb(0x10, 0x10, 0x10),
        foreground: rgb(0xee, 0xee, 0xee),
        selection: Some((rgb(0x44, 0x47, 0x5a), rgb(0xf8, 0xf8, 0xf2))),
    };
    let (fill, text, wash) = chip_colors(&cell, themed, true);
    assert_eq!(fill, color(rgb(0x44, 0x47, 0x5a), 1.0), "the selection background");
    assert_eq!(
        text,
        color(rgb(0xf8, 0xf8, 0xf2), UNSELECTED_CAPTION),
        "the selection text, muted"
    );
    assert!(wash.is_none());
    cell.highlighted = true;
    let (_, text, _) = chip_colors(&cell, themed, true);
    assert_eq!(text, color(rgb(0xf8, 0xf8, 0xf2), 1.0), "full on the highlighted cell");
    let plain = Look {
        selection: None,
        ..themed
    };
    let (fill, text, _) = chip_colors(&cell, plain, true);
    assert_eq!(fill, color(rgb(0xee, 0xee, 0xee), 0.22), "the foreground wash");
    assert_eq!(text, color(rgb(0xee, 0xee, 0xee), 1.0));
    cell.status = Some(crate::status_glyph::GlyphLook {
        status: AgentStatus::Blocked,
        shape: ngterm_core::model::StatusShape::Circle,
        rgb: (1.0, 0.584, 0.0),
        blink: true,
        tooltip: String::new(),
    });
    let (fill, text, wash) = chip_colors(&cell, themed, true);
    assert_eq!(
        fill,
        crate::status_glyph::hsla((1.0, 0.584, 0.0)),
        "the status color, not the selection"
    );
    assert_eq!(text, gpui_kit::black(), "contrasting text");
    assert_eq!(wash, Some(gpui_kit::white()), "a blinking chip pulses");
    assert!(
        chip_colors(&cell, themed, false).2.is_none(),
        "not while animations are off"
    );
}

#[test]
fn contrast_picks_dark_text_over_a_light_fill() {
    assert!(luminance((1.0, 0.584, 0.0)) > 0.5, "orange takes black text");
    assert!(luminance((0.1, 0.1, 0.3)) < 0.5, "a dark fill takes white text");
}

#[test]
fn an_override_reports_nothing_and_its_revert_is_dropped_once() {
    // while overridden, nothing is the pane's
    assert_eq!(font_report(Some(9.0), None, 9.0), (false, None));
    // the revert to the pending size is dropped (within half a point), and the next report is the pane's again
    assert_eq!(font_report(None, Some(13.0), 13.0), (false, None));
    assert_eq!(font_report(None, Some(13.0), 13.4), (false, None));
    assert_eq!(font_report(None, None, 14.0), (true, None));
    // a report that is not the revert is the pane's, and ends the wait for it
    assert_eq!(font_report(None, Some(13.0), 16.0), (true, None));
}

/// A status chip over a dark color takes white text and a black wash.
#[test]
fn a_dark_status_chip_takes_white_text_and_a_dark_wash() {
    let look = Look {
        background: Rgb::new(0x10, 0x10, 0x10),
        foreground: Rgb::new(0xee, 0xee, 0xee),
        selection: None,
    };
    let cell = ChromeCell {
        member: DashboardMember::primary(Uuid::nil()),
        caption: "app".to_owned(),
        highlighted: false,
        cover: None,
        status: Some(crate::status_glyph::GlyphLook {
            status: AgentStatus::Active,
            shape: ngterm_core::model::StatusShape::Circle,
            rgb: (0.1, 0.1, 0.3),
            blink: true,
            tooltip: String::new(),
        }),
    };
    let (fill, text, wash) = chip_colors(&cell, look, true);
    assert_eq!(fill, crate::status_glyph::hsla((0.1, 0.1, 0.3)));
    assert_eq!(text, gpui_kit::white());
    assert_eq!(wash, Some(gpui_kit::black()));
}

// MARK: - the grid in a window

/// The main loop a cell's click posts to: the immediate tasks are kept, a delayed one is kept apart or, when `closed`,
/// refused as a closed loop refuses it.
#[derive(Default)]
struct Posted {
    now: std::sync::Mutex<Vec<ngterm_host::MainTask<AppState>>>,
    later: std::sync::Mutex<Vec<(Duration, ngterm_host::MainTask<AppState>)>>,
    closed: bool,
}

impl ngterm_host::MainLoop<AppState> for Posted {
    fn post(&self, task: ngterm_host::MainTask<AppState>) -> Result<(), ngterm_host::MainLoopClosed> {
        self.now.lock().unwrap().push(task);
        Ok(())
    }

    fn post_after(
        &self,
        delay: Duration,
        task: ngterm_host::MainTask<AppState>,
    ) -> Result<(), ngterm_host::MainLoopClosed> {
        if self.closed {
            return Err(ngterm_host::MainLoopClosed);
        }
        self.later.lock().unwrap().push((delay, task));
        Ok(())
    }
}

/// A view drawing [`grid`] over `chrome`'s cells, with no terminals.
struct GridView {
    cells: Vec<ChromeCell>,
    poster: Poster,
    window: Uuid,
    animate: bool,
}

impl gpui_kit::Render for GridView {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        let look = Look {
            background: Rgb::new(0x10, 0x10, 0x10),
            foreground: Rgb::new(0xee, 0xee, 0xee),
            selection: None,
        };
        let cells = self
            .cells
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, chrome)| CellParts {
                index,
                chrome,
                terminal: None,
                lead: None,
            })
            .collect();
        div()
            .size_full()
            .child(grid(self.window, cells, look, &self.poster, self.animate))
    }
}

fn chrome_cell(cover: Option<DashboardCover>) -> ChromeCell {
    ChromeCell {
        member: DashboardMember::primary(Uuid::new_v4()),
        caption: "app".to_owned(),
        highlighted: false,
        cover,
        status: None,
    }
}

fn grid_window(
    cx: &mut gpui_kit::TestAppContext,
    cells: Vec<ChromeCell>,
    poster: Poster,
) -> &mut gpui_kit::VisualTestContext {
    let (_, cx) = cx.add_window_view(move |_, _| GridView {
        cells,
        poster,
        window: Uuid::nil(),
        animate: true,
    });
    cx.simulate_resize(gpui_kit::size(px(800.), px(500.)));
    cx.run_until_parked();
    cx
}

/// Reads the cover's published lines by their identifiers, so swapping their text fails too.
fn assert_cover_lines(cx: &mut gpui_kit::VisualTestContext, expected: &[(&str, &str)]) {
    cx.activate_a11y();
    let tree = cx.a11y_tree_update().expect("the cover's accessibility tree");
    for (id, text) in expected {
        let node = tree
            .nodes
            .iter()
            .find(|(_, node)| node.author_id() == Some(*id))
            .map(|(_, node)| node)
            .expect("the cover line");
        assert_eq!(node.label(), Some(*text), "{id}");
        assert_eq!(node.value(), Some(*text), "{id}");
    }
}

/// Upstream's `DashboardOverlayCover`: a page names itself an HTML overlay with its source and its own title dimmed
/// beneath, a program a program overlay with its command; one shown from another instance has no command line.
#[gpui_kit::test]
fn a_covered_cell_names_its_cover_kind_source_and_title(cx: &mut gpui_kit::TestAppContext) {
    let page = DashboardCover::Page {
        identity: "example.com".to_owned(),
        title: Some("Build status".to_owned()),
    };
    let cx = grid_window(cx, vec![chrome_cell(Some(page))], crate::state::detached_poster());
    assert_cover_lines(
        cx,
        &[
            ("dashboard-overlay-cover", "HTML overlay"),
            ("dashboard-overlay-cover-source", "example.com"),
            ("dashboard-overlay-cover-title", "Build status"),
        ],
    );
}

#[gpui_kit::test]
fn a_program_cover_names_its_command_and_has_no_title(cx: &mut gpui_kit::TestAppContext) {
    let program = |command: Option<&str>| DashboardCover::Program {
        command: command.map(str::to_owned),
    };
    let cx = grid_window(
        cx,
        vec![chrome_cell(Some(program(Some("htop"))))],
        crate::state::detached_poster(),
    );
    assert_cover_lines(
        cx,
        &[
            ("dashboard-overlay-cover", "Program overlay"),
            ("dashboard-overlay-cover-source", "htop"),
        ],
    );
    assert!(
        cx.debug_bounds("dashboard-overlay-cover-title").is_none(),
        "a program has no title"
    );
}

#[gpui_kit::test]
fn a_program_shown_from_another_instance_names_no_command(cx: &mut gpui_kit::TestAppContext) {
    let cover = DashboardCover::Program { command: None };
    let cx = grid_window(cx, vec![chrome_cell(Some(cover))], crate::state::detached_poster());
    assert_cover_lines(cx, &[("dashboard-overlay-cover", "Program overlay")]);
    assert!(cx.debug_bounds("dashboard-overlay-cover-source").is_none());
    assert!(cx.debug_bounds("dashboard-overlay-cover-title").is_none());
}

/// Only a blinking chip draws a wash, and disabling animations removes it from the grid.
#[gpui_kit::test]
fn a_blinking_chip_pulses_in_the_grid(cx: &mut gpui_kit::TestAppContext) {
    let mut blinking = chrome_cell(None);
    blinking.status = Some(crate::status_glyph::GlyphLook {
        status: AgentStatus::Blocked,
        shape: ngterm_core::model::StatusShape::Circle,
        rgb: (1.0, 0.584, 0.0),
        blink: true,
        tooltip: String::new(),
    });
    let (view, cx) = cx.add_window_view(move |_, _| GridView {
        cells: vec![blinking, chrome_cell(None)],
        poster: crate::state::detached_poster(),
        window: Uuid::nil(),
        animate: true,
    });
    cx.simulate_resize(gpui_kit::size(px(800.), px(500.)));
    cx.run_until_parked();
    let washes = |cx: &mut gpui_kit::VisualTestContext| {
        cx.update(|window, _| {
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| {
                    quad.background
                        .as_solid()
                        .is_some_and(|fill| (fill.l - 1.0).abs() < f32::EPSILON)
                })
                .collect::<Vec<_>>()
        })
    };
    // The first animation sample is transparent; paint a later sample before inspecting the wash.
    std::thread::sleep(Duration::from_millis(10));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let drawn = washes(cx);
    assert_eq!(drawn.len(), 1, "only the blinking cell draws a white wash");
    let alpha = drawn[0].background.as_solid().unwrap().a;
    assert!(
        alpha > 0.0 && alpha <= WASH_PEAK,
        "the wash is animated, not an opaque fill: {alpha}"
    );
    let idle = cx.debug_bounds("dashboard-cell-1").unwrap();
    let scale = cx.update(|window, _| window.scale_factor());
    assert!(
        drawn[0].bounds.right().0 / scale < f32::from(idle.origin.x),
        "the wash belongs to the blinking cell"
    );
    view.update(cx, |grid, cx| {
        grid.animate = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(washes(cx).is_empty(), "animations are off");
    view.update(cx, |grid, cx| {
        grid.animate = true;
        grid.cells[0].status.as_mut().unwrap().blink = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(washes(cx).is_empty(), "a steady status draws no wash");
}

/// A click highlights its member in its window immediately and enters that member only after the flash.
#[gpui_kit::test]
fn a_click_posts_its_highlight_and_its_delayed_enter(cx: &mut gpui_kit::TestAppContext) {
    let (mut state, window) = dashboard_state();
    let model = GridModel::of(&state.core, window).unwrap();
    let member = model.members[1];
    state.dashboard_click(window, model.members[0]);
    let cells = model.cells(state.core.library().store(window), state.core.settings());
    let open = std::sync::Arc::new(Posted::default());
    let poster: Poster = std::sync::Arc::downgrade(&open) as Poster;
    let (_, cx) = cx.add_window_view(move |_, _| GridView {
        cells,
        poster,
        window,
        animate: true,
    });
    cx.simulate_resize(gpui_kit::size(px(800.), px(500.)));
    cx.run_until_parked();
    let at = cx.debug_bounds("dashboard-cell-1").unwrap().center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    let now = std::mem::take(&mut *open.now.lock().unwrap());
    assert_eq!(now.len(), 1, "the highlight");
    for task in now {
        task(&mut state);
    }
    assert_eq!(highlighted(&state, window), Some(member));
    assert!(
        state.core.dashboard(window).is_some(),
        "highlighting keeps the grid open"
    );
    let mut later = std::mem::take(&mut *open.later.lock().unwrap());
    assert_eq!(later.len(), 1, "the enter");
    let (delay, task) = later.pop().unwrap();
    assert_eq!(delay, CLICK_ENTER_DELAY);
    task(&mut state);
    assert!(
        state.core.dashboard(window).is_none(),
        "the delayed task enters the cell"
    );
    assert_eq!(
        state.core.library().store(window).unwrap().selected_session_id(),
        Some(member.session)
    );
}

/// A closing loop accepts only the highlight; a gone loop accepts nothing.
#[test]
fn a_click_on_a_closing_loop_posts_only_its_highlight_and_on_a_gone_one_nothing() {
    let member = DashboardMember::primary(Uuid::new_v4());
    let closing = std::sync::Arc::new(Posted {
        closed: true,
        ..Posted::default()
    });
    clicked(&(std::sync::Arc::downgrade(&closing) as Poster), Uuid::nil(), member);
    assert_eq!(closing.now.lock().unwrap().len(), 1);
    assert!(closing.later.lock().unwrap().is_empty());
    let gone = std::sync::Arc::new(Posted::default());
    let poster = std::sync::Arc::downgrade(&gone) as Poster;
    drop(gone);
    clicked(&poster, Uuid::nil(), member);
    assert!(poster.upgrade().is_none(), "nothing to post to, and no panic");
}

// MARK: - the actions

/// A window with two sessions and its dashboard open over them.
fn dashboard_state() -> (AppState, Uuid) {
    let mut library = ngterm_core::model::WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let core = Core::new(
        ngterm_core::CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState {
            library,
            ..ngterm_core::CoreState::default()
        },
    );
    let mut state = AppState::new(core);
    state.core.toggle_dashboard(window);
    assert!(state.core.dashboard(window).is_some(), "the dashboard opened");
    (state, window)
}

fn highlighted(state: &AppState, window: Uuid) -> Option<DashboardMember> {
    state.core.dashboard(window).and_then(DashboardController::highlighted)
}

#[test]
fn the_grid_keys_walk_enter_and_close() {
    let (mut state, window) = dashboard_state();
    let members = state.core.dashboard(window).unwrap().members().to_vec();
    let (first, second) = (members[0], members[1]);
    state.dashboard_click(window, first);
    state.dashboard_key(window, GridKey::Move(Direction::Right));
    assert_eq!(
        highlighted(&state, window),
        Some(second),
        "the arrow moved the highlight"
    );
    state.dashboard_key(window, GridKey::Enter);
    assert!(state.core.dashboard(window).is_none(), "Enter entered the cell");
    let selected = state.core.library().store(window).unwrap().selected_session_id();
    assert_eq!(selected, Some(second.session));

    state.core.toggle_dashboard(window);
    state.dashboard_key(window, GridKey::Close);
    assert!(state.core.dashboard(window).is_none(), "Escape closed it");
}

#[test]
fn a_clicks_enter_waits_for_its_cell_to_stay_highlighted() {
    let (mut state, window) = dashboard_state();
    let members = state.core.dashboard(window).unwrap().members().to_vec();
    let (first, second) = (members[0], members[1]);
    state.dashboard_click(window, first);
    assert_eq!(highlighted(&state, window), Some(first));
    state.dashboard_click(window, second);
    state.dashboard_click_enter(window, first);
    assert!(
        state.core.dashboard(window).is_some(),
        "a later click moved the highlight"
    );
    state.dashboard_click_enter(window, second);
    assert!(
        state.core.dashboard(window).is_none(),
        "the cell still ringed is entered"
    );
}

/// After the quit save the grid's actions change nothing.
#[test]
fn the_grid_does_nothing_after_the_quit_save() {
    let (mut state, window) = dashboard_state();
    let members = state.core.dashboard(window).unwrap().members().to_vec();
    let before = highlighted(&state, window);
    state.save_on_quit();
    state.dashboard_key(window, GridKey::Move(Direction::Right));
    state.dashboard_key(window, GridKey::Close);
    state.dashboard_click(window, members[1]);
    assert_eq!(highlighted(&state, window), before);
    state.dashboard_click_enter(window, members[0]);
    assert!(state.core.dashboard(window).is_some(), "still open");
}

// MARK: - fonts

/// A terminal not started yet takes no grid font: it gets its size on a later sync, once started.
#[gpui_kit::test]
fn a_terminal_not_started_yet_takes_no_grid_font(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    let spawn: crate::terminal::Spawn = Box::new(|config: ngterm_term::PaneConfig| {
        ngterm_term::Pane::spawn(&ngterm_term::PaneCommand::new("true"), &config)
    });
    let terminal = cx.new(|cx| {
        crate::terminal::Terminal::new(
            crate::terminal::TerminalSpec::default(),
            crate::terminal::TerminalConfig::default(),
            spawn,
            cx,
        )
    });
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    assert!(!shared.is_realized());
    let mut overrides = FontOverrides::default();
    overrides.sync(&[("t1".to_owned(), Rc::clone(&shared))], Some(9.0));
    assert!(overrides.tokens().is_empty());
    assert_eq!(overrides.applied("t1"), None);
    assert_eq!(shared.font_override(), None);
}
