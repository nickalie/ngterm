//! The panel's placement, config, painter and link modifier, ported from the GTK build's `hud.rs` tests. The panel on
//! its page (mounted, passive, replaced, closed, its Ctrl-click) is the deck's tests (`deck/tests.rs`).

use ngterm_core::hud::HudSpec;
use ngterm_core::model::HudState;

use super::*;

fn style(position: HudPosition, sticky: bool) -> PanelStyle {
    PanelStyle {
        width_fraction: 0.5,
        height_fraction: 0.1,
        height_points: Some(60.0),
        edge_margin: if sticky { 0.0 } else { 0.1 },
        position,
        frame: true,
    }
}

fn pane() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: 1000,
        height: 600,
    }
}

#[test]
fn a_centered_panel_sits_in_the_middle() {
    let frame = style(HudPosition::Center, false).panel_frame(pane());
    assert_eq!(
        frame,
        Rect {
            x: 250,
            y: 270,
            width: 500,
            height: 60
        }
    );
}

#[test]
fn an_anchored_panel_holds_the_edge_margin_unless_sticky() {
    let frame = style(HudPosition::TopLeft, false).panel_frame(pane());
    // across: (1000 * (0.25 - 0.1)) = 150 off center; down: 600 * ((1 - 0.1) / 2 - 0.1) = 210 up
    assert_eq!((frame.x, frame.y), (100, 60));
    let sticky = style(HudPosition::BottomRight, true).panel_frame(pane());
    assert_eq!(
        (sticky.x + sticky.width, sticky.y + sticky.height),
        (1000, 600),
        "flush with the corner"
    );
}

#[test]
fn a_scoped_panel_is_placed_in_its_pane() {
    let right = Rect {
        x: 501,
        y: 0,
        width: 499,
        height: 600,
    };
    let frame = style(HudPosition::Center, false).panel_frame(right);
    assert!(frame.x > right.x && frame.x + frame.width < right.x + right.width);
}

#[test]
fn the_style_follows_the_slot() {
    let mut slot = OverlaySlot {
        active: true,
        size_percent: Some(40),
        ..OverlaySlot::default()
    };
    assert_eq!(PanelStyle::of(&slot), None, "no hud");
    let mut spec = HudSpec::new("m");
    spec.sticky = true;
    spec.frame = false;
    spec.position = HudPosition::BottomCenter;
    slot.hud = Some(HudState {
        spec,
        pane_identity: None,
        file: PathBuf::from("/tmp/x"),
        height_percent: 12,
        height_points: Some(48.0),
        font_size: None,
        resized_width_percent: None,
    });
    let style = PanelStyle::of(&slot).unwrap();
    assert!((style.width_fraction - 0.4).abs() < 1e-9);
    assert_eq!(style.height_points, Some(48.0));
    assert!(style.edge_margin.abs() < f64::EPSILON && !style.frame);
    assert_eq!(style.position, HudPosition::BottomCenter);
}

/// The pane upstream's sticky cases lay a caption out in.
const CAPTION_PANE: Rect = Rect {
    x: 200,
    y: 100,
    width: 1000,
    height: 800,
};

/// A 48-point caption's style at `position` and `size_percent` (upstream's `captionSession`).
fn caption(position: HudPosition, size_percent: i64, sticky: bool, frame: bool) -> PanelStyle {
    let mut spec = HudSpec::new("caption");
    spec.position = position;
    spec.sticky = sticky;
    spec.frame = frame;
    let slot = OverlaySlot {
        active: true,
        size_percent: Some(size_percent),
        hud: Some(HudState {
            spec,
            pane_identity: None,
            file: PathBuf::from("/tmp/x"),
            height_percent: 10,
            height_points: Some(48.0),
            font_size: None,
            resized_width_percent: None,
        }),
        ..OverlaySlot::default()
    };
    PanelStyle::of(&slot).unwrap()
}

#[test]
fn a_sticky_top_panel_sits_flush_and_spans_the_pane() {
    let panel = caption(HudPosition::TopCenter, 100, true, false).panel_frame(CAPTION_PANE);
    assert_eq!(
        panel,
        Rect {
            x: 200,
            y: 100,
            width: 1000,
            height: 48
        }
    );
}

#[test]
fn a_sticky_bottom_panel_sits_flush_at_the_bottom() {
    let panel = caption(HudPosition::BottomCenter, 100, true, false).panel_frame(CAPTION_PANE);
    assert_eq!(panel.y + panel.height, CAPTION_PANE.y + CAPTION_PANE.height);
    assert_eq!(panel.height, 48);
}

#[test]
fn a_sticky_corner_panel_touches_both_edges() {
    let panel = caption(HudPosition::BottomRight, 40, true, false).panel_frame(CAPTION_PANE);
    assert_eq!(panel.x + panel.width, CAPTION_PANE.x + CAPTION_PANE.width);
    assert_eq!(panel.y + panel.height, CAPTION_PANE.y + CAPTION_PANE.height);
    assert_eq!(panel.width, 400);
}

#[test]
fn a_sticky_side_panel_touches_its_edge_and_stays_vertically_centered() {
    let panel = caption(HudPosition::CenterLeft, 40, true, false).panel_frame(CAPTION_PANE);
    assert_eq!(panel.x, CAPTION_PANE.x);
    assert_eq!(
        2 * panel.y + panel.height,
        2 * CAPTION_PANE.y + CAPTION_PANE.height,
        "the same middle"
    );
}

#[test]
fn sticky_changes_nothing_at_center() {
    let sticky = caption(HudPosition::Center, 40, true, false);
    let loose = caption(HudPosition::Center, 40, false, false);
    assert_eq!(sticky.panel_frame(CAPTION_PANE), loose.panel_frame(CAPTION_PANE));
}

#[test]
fn a_panel_that_is_not_sticky_keeps_its_edge_margin() {
    let panel = caption(HudPosition::TopLeft, 40, false, false).panel_frame(CAPTION_PANE);
    assert_eq!((panel.x, panel.y), (CAPTION_PANE.x + 100, CAPTION_PANE.y + 80));
}

#[test]
fn the_height_in_points_wins_over_the_fraction_whatever_the_pane_height() {
    let style = caption(HudPosition::Center, 40, false, true);
    assert!(style.height_fraction > 0.0, "a fraction is there to lose to");
    for height in [500, 1000] {
        let pane = Rect { height, ..CAPTION_PANE };
        assert_eq!(style.panel_frame(pane).height, 48, "{height}");
    }
}

#[test]
fn the_page_places_a_scoped_panel_in_its_pane_and_nothing_while_it_is_not_laid_out() {
    use crate::split::{Panes, geometry};
    use ngterm_core::model::SplitAxis;
    let placement = HudPlacement {
        style: caption(HudPosition::TopCenter, 100, true, false),
        scope: Some(Slot::Split),
    };
    let laid = geometry(Panes::Both, SplitAxis::LeftRight, 0.5, 1000, 600);
    let rect = crate::split::Place::Hud(placement);
    let page = |panes, geometry: &crate::split::Geometry| rect.rect(panes, geometry, SplitAxis::LeftRight, 1000, 600);
    let placed = page(Panes::Both, &laid);
    assert_eq!((placed.x, placed.y, placed.width), (laid.split.x, 0, laid.split.width));
    let alone = geometry(Panes::Alone(Slot::Primary), SplitAxis::LeftRight, 0.5, 1000, 600);
    assert!(
        page(Panes::Alone(Slot::Primary), &alone).is_empty(),
        "its pane is not laid out"
    );
    let whole = crate::split::Place::Hud(HudPlacement {
        scope: None,
        ..placement
    });
    assert_eq!(
        whole.rect(Panes::Both, &laid, SplitAxis::LeftRight, 1000, 600).width,
        1000
    );
}

#[test]
fn the_config_pins_the_padding_and_paints_the_background() {
    let plain = config_overlay(None, 1.0);
    assert!(plain.contains("window-padding-x = 8\n") && plain.contains("window-padding-y = 6\n"));
    assert!(!plain.contains("background ="));
    let colored = config_overlay(Some("#102030"), 1.0);
    assert!(colored.contains("background = #102030\n"));
    assert!(colored.contains("background-opacity = 1\n"));
    assert!(colored.contains("window-padding-balance = true"));
    let translucent = config_overlay(Some("#102030"), 0.7);
    assert!(
        translucent.contains("background-opacity = 0.7\n"),
        "the window's translucency baked in: {translucent}"
    );
}

/// Upstream's `HudLinkClick` test: ⌘ and the ⌘⌥ reveal chord click a link, ⌥ alone, ⌘⇧, ⌘⌥⇧ and ⌘⌃ do not (Ctrl or
/// Super standing for ⌘).
#[test]
fn ctrl_or_super_and_the_reveal_chord_are_the_link_modifiers() {
    assert!(is_link_modifier(&Modifiers::control()));
    assert!(is_link_modifier(&Modifiers {
        control: true,
        alt: true,
        ..Modifiers::default()
    }));
    assert!(is_link_modifier(&Modifiers {
        platform: true,
        ..Modifiers::default()
    }));
    assert!(is_link_modifier(&Modifiers {
        platform: true,
        alt: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers {
        platform: true,
        shift: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers {
        platform: true,
        alt: true,
        shift: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers {
        control: true,
        function: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers::control_shift()));
    assert!(!is_link_modifier(&Modifiers {
        control: true,
        alt: true,
        shift: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers {
        alt: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers {
        control: true,
        platform: true,
        ..Modifiers::default()
    }));
    assert!(!is_link_modifier(&Modifiers::default()));
}

#[gpui_kit::test]
fn the_cell_estimate_measures_the_terminals_face(cx: &mut gpui_kit::TestAppContext) {
    let text = cx.update(|cx| {
        crate::terminal::register_embedded_fonts(cx.text_system());
        Arc::clone(cx.text_system())
    });
    let estimate = cell_estimator(text);
    let (width, height) = estimate(None, 13.0);
    assert!(width >= 1.0 && height > width, "{width}x{height}");
    let (bigger, _) = estimate(None, 26.0);
    assert!(bigger > width, "twice the size is wider: {bigger} > {width}");
}

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

fn event_state(dir: &std::path::Path) -> (AppState, Arc<Posted>, Uuid, Uuid) {
    let library = ngterm_core::model::WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let session = library.store(window).unwrap().active_session().unwrap().id();
    let core = ngterm_core::Core::new(
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
    state.core.set_hud_body_dir(dir.to_path_buf());
    state.core.set_cell_estimator(Box::new(|_, _| (8.0, 16.0)));
    let posted = Arc::new(Posted::default());
    let main_loop: Arc<dyn ngterm_host::MainLoop<AppState>> = posted.clone();
    state.attach(Arc::downgrade(&main_loop));
    (state, posted, window, session)
}

fn send_event(state: &mut AppState, posted: &Posted, session: Uuid, generation: u64, event: &TerminalEvent) {
    panel_event(state.poster(), session, generation, event);
    let tasks = std::mem::take(&mut *posted.0.lock().unwrap());
    for task in tasks {
        task(state);
    }
}

fn open_hud(state: &mut AppState, session: Uuid, message: &str) -> (u64, PathBuf) {
    let response = state.core.dispatch(&ngterm_proto::Request {
        cmd: ngterm_proto::Command::Session(ngterm_proto::command::SessionCmd::HudOpen),
        target: Some(session.to_string()),
        args: Some(ngterm_proto::Args {
            message: Some(message.to_owned()),
            ..ngterm_proto::Args::default()
        }),
    });
    assert!(response.ok, "{response:?}");
    let slot = &state.core.library().held_session(session).unwrap().overlay;
    (slot.generation, slot.hud.as_ref().unwrap().file.clone())
}

#[test]
fn a_painters_late_exit_keeps_its_replacement_and_its_own_exit_removes_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, posted, _, session) = event_state(dir.path());
    let (old, _) = open_hud(&mut state, session, "old");
    let (current, file) = open_hud(&mut state, session, "replacement");
    let exited = TerminalEvent::Pane(ngterm_term::PaneEvent::Exited(ngterm_term::ExitStatus {
        code: 0,
        signal: None,
        held: false,
    }));
    send_event(&mut state, &posted, session, old, &exited);
    assert!(file.exists());
    assert!(state.core.library().held_session(session).unwrap().overlay.hud_active());
    send_event(&mut state, &posted, session, current, &exited);
    assert!(!file.exists());
    assert!(!state.core.library().held_session(session).unwrap().overlay.hud_active());
}

#[test]
fn font_and_spawn_events_refresh_the_body_and_hud_links_use_the_browser() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, posted, _, session) = event_state(dir.path());
    let (generation, file) = open_hud(&mut state, session, "still painting");
    for event in [TerminalEvent::Spawned, TerminalEvent::FontSize] {
        std::fs::remove_file(&file).unwrap();
        send_event(&mut state, &posted, session, generation, &event);
        assert!(std::fs::read_to_string(&file).unwrap().contains("still painting"));
    }
    let event = TerminalEvent::OpenUrl {
        url: "https://example.com/hud".to_owned(),
        intent: ngterm_core::link_policy::ClickIntent::Browser,
        kind: crate::terminal::LinkKind::Osc8,
    };
    send_event(&mut state, &posted, session, generation, &event);
    assert_eq!(state.take_frontend_work().open_urls, ["https://example.com/hud"]);
    send_event(
        &mut state,
        &posted,
        Uuid::new_v4(),
        generation,
        &TerminalEvent::FontSize,
    );
    assert!(file.exists(), "an event for a gone session leaves the live HUD alone");
}

#[gpui_kit::test]
fn an_explicit_font_family_measures_a_cell_and_blank_names_use_the_default(cx: &mut gpui_kit::TestAppContext) {
    let text = cx.update(|cx| {
        crate::terminal::register_embedded_fonts(cx.text_system());
        Arc::clone(cx.text_system())
    });
    let estimate = cell_estimator(text);
    let default = estimate(None, 14.0);
    assert_eq!(estimate(Some("  "), 14.0), default);
    let (width, height) = estimate(Some("JetBrains Mono"), 14.0);
    assert!(width >= 1.0 && height >= width);
}

struct PanelHarness<'a> {
    state: AppState,
    posted: Arc<Posted>,
    deck: Entity<crate::deck::Deck>,
    session: Uuid,
    cx: &'a mut gpui_kit::VisualTestContext,
}

impl PanelHarness<'_> {
    fn turn(&mut self) {
        let tasks = std::mem::take(&mut *self.posted.0.lock().unwrap());
        for task in tasks {
            task(&mut self.state);
        }
        let core = &mut self.state.core;
        self.deck.update_in(self.cx, |deck, window, cx| {
            let _ = deck.reconcile(core, false, window, cx);
        });
        self.cx.run_until_parked();
    }

    fn panel(&self) -> (Entity<Terminal>, Entity<TerminalView>) {
        self.deck.read_with(self.cx, |deck, _| {
            let panel = deck.page(self.session).unwrap().hud().panel().unwrap();
            (panel.terminal().clone(), panel.view().clone())
        })
    }
}

fn panel_harness<'a>(dir: &std::path::Path, cx: &'a mut gpui_kit::TestAppContext) -> PanelHarness<'a> {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let (mut state, posted, window, session) = event_state(dir);
    open_hud(&mut state, session, "https://example.com/panel");
    let env = Rc::new(DeckEnv::new(
        crate::pane::PaneContext {
            socket: dir.join("unused.sock").to_string_lossy().into_owned(),
            program_version: "1.2.3".to_owned(),
        },
        Rc::new(|launch| {
            let launch = launch.clone();
            Box::new(move |config| {
                if launch.painter.is_none() {
                    return Err(ngterm_term::PaneError::Closed);
                }
                let host =
                    ngterm_term::Host::new(&ngterm_core::ghostty_config::GhosttyConfig::default(), None, None, None);
                ngterm_term::Pane::launch(&launch, &host, &config)
            })
        }),
        crate::terminal::TerminalConfig::default(),
        state.poster().clone(),
        crate::deck::Watermarks {
            state_dir: dir.to_path_buf(),
            config_with: Rc::new(|overlay| {
                let mut loader = ngterm_core::ghostty_config::Loader::new(None);
                loader.load_text("hud", overlay);
                crate::terminal::TerminalConfig::from_ghostty(&loader.finish().finalize(false, &[]).config)
            }),
        },
    ));
    let (deck, cx) = cx.add_window_view(|_, _| crate::deck::Deck::new(window, env));
    cx.simulate_resize(gpui_kit::size(px(800.), px(600.)));
    let mut harness = PanelHarness {
        state,
        posted,
        deck,
        session,
        cx,
    };
    harness.turn();
    harness
}

/// Waits for the panel's link on its final row. A paint that lands before the panel has its final grid leaves the link
/// on the top row until the resize's repaint centers it, so the row comes from the painter's own header
/// (`<columns> <rows> ...`), which settles a one-line body in the middle.
fn await_settled_link(h: &mut PanelHarness<'_>, view: &Entity<TerminalView>) {
    let settled = {
        let slot = &h.state.core.library().held_session(h.session).unwrap().overlay;
        let body = std::fs::read_to_string(&slot.hud.as_ref().unwrap().file).unwrap();
        let rows: usize = body.split_whitespace().nth(1).unwrap().parse().unwrap();
        (rows - 1) / 2
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        h.turn();
        if view.read_with(h.cx, |view, _| {
            view.painted().is_some_and(|frame| {
                frame
                    .lines()
                    .get(settled)
                    .is_some_and(|line| line.contains("https://example.com/panel"))
            })
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the painter's link never settled on its row"
        );
        std::thread::yield_now();
    }
}

#[gpui_kit::test]
fn hovering_a_hud_link_marks_it_and_leaving_or_adding_a_modifier_clears_it(cx: &mut gpui_kit::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = panel_harness(dir.path(), cx);
    let (_, view) = h.panel();
    await_settled_link(&mut h, &view);
    let (terminal, _) = h.panel();
    let at = h.deck.read_with(h.cx, |deck, cx| {
        let page = deck.page(h.session).unwrap();
        let placement = page.hud().placement().unwrap();
        let rect = placement.style.panel_frame(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        let (cell_x, cell_y) = terminal.read(cx).shared().cell_size();
        let (row, column) = view
            .read(cx)
            .painted()
            .unwrap()
            .lines()
            .iter()
            .enumerate()
            .find_map(|(row, line)| line.find("https://").map(|column| (row, column)))
            .expect("the link has a painted cell");
        #[expect(clippy::cast_precision_loss, reason = "the panel's row and column fit f64")]
        let (row, column) = (row as f64, column as f64);
        #[expect(clippy::cast_possible_truncation, reason = "the test window's coordinates fit f32")]
        gpui_kit::point(
            px((f64::from(rect.x) + 8.0 + (column + 2.5) * cell_x) as f32),
            px((f64::from(rect.y) + 6.0 + (row + 0.75) * cell_y) as f32),
        )
    });
    h.cx.simulate_mouse_move(at, None, Modifiers::control());
    h.turn();
    assert!(view.read_with(h.cx, |view, _| view.hovered_link().is_some()));
    h.cx.simulate_mouse_move(at, None, Modifiers::control_shift());
    h.turn();
    assert!(view.read_with(h.cx, |view, _| view.hovered_link().is_none()));
    h.cx.simulate_mouse_move(at, None, Modifiers::control());
    h.turn();
    assert!(view.read_with(h.cx, |view, _| view.hovered_link().is_some()));
    // Control still held: only leaving the panel clears it
    h.cx.simulate_mouse_move(gpui_kit::point(px(0.), px(0.)), None, Modifiers::control());
    h.turn();
    assert!(view.read_with(h.cx, |view, _| view.hovered_link().is_none()));
    // an app config change: the panel takes it under its own overlay (its padding) again
    let padding =
        |cx: &gpui_kit::VisualTestContext| terminal.read_with(cx, |terminal, _| terminal.shared().config().padding);
    let own = padding(h.cx);
    terminal.update(h.cx, |terminal, cx| {
        terminal.set_config(crate::terminal::TerminalConfig::default(), cx);
    });
    assert_ne!(padding(h.cx), own, "the default config has no panel padding");
    h.deck.update(h.cx, |deck, cx| {
        deck.page(h.session)
            .unwrap()
            .hud()
            .panel()
            .unwrap()
            .apply_config(deck.env(), cx);
    });
    assert_eq!(padding(h.cx), own);
    terminal.read_with(h.cx, |terminal, _| terminal.shared().teardown());
}
