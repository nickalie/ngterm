//! Hidden overlays on a real page (upstream's `ControlOverlayVisibilityUITests` and the deck side of the hide-overlays
//! change, `WindowContentView+Detail.swift`): a hidden program, pane overlay or page stays mounted, its program or
//! document running, with no placement; the terminal under it takes the keyboard and the pointer, and the washes follow
//! the visible layer; showing brings the same one back with the keyboard, nothing rebuilt. The keyboard moves only
//! when the visible cover changes: never for a HUD, a hidden overlay's close or the other pane's overlay.

use std::path::Path;

use ngterm_core::model::{OverlayPane, PanelSizing};

use super::overlay::{overlay_harness, pane_overlays, program, program_realized, quoted};
use super::*;
use crate::overlay::ProgramPlacement;

/// Runs `verb` (`session.overlay.hide`, `show` or `toggle`) on `session`, scoped to `pane`, and lands it.
fn visibility(h: &mut Harness<'_>, session: Uuid, verb: SessionCmd, pane: Option<&str>) {
    h.dispatch(Command::Session(verb), session, |args| {
        args.pane = pane.map(str::to_owned);
    });
    h.turn();
}

fn hidden(h: &Harness<'_>, session: Uuid) -> Vec<&'static str> {
    h.session(session).hidden_overlay_names()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The session-wide program's terminal state.
fn program_shared(h: &mut Harness<'_>, session: Uuid) -> Rc<Shared> {
    h.deck.read_with(h.cx, |deck, _| {
        Rc::clone(deck.page(session).unwrap().overlays().program().unwrap().shared())
    })
}

/// The program's grid width, as the program sees it.
fn program_cols(h: &mut Harness<'_>, session: Uuid) -> u16 {
    program_shared(h, session)
        .pane()
        .map_or(0, |pane| pane.frame().frame.cols)
}

/// Where the program shows, where it is laid out, and whether a backdrop washes the page behind it.
fn program_layout(h: &mut Harness<'_>, session: Uuid) -> (Option<ProgramPlacement>, Option<ProgramPlacement>, bool) {
    h.deck.read_with(h.cx, |deck, _| {
        let page = deck.page(session).unwrap();
        (
            page.overlays().placement(),
            page.overlays().frame(),
            page.washes().backdrop.is_some(),
        )
    })
}

/// Splits `session` and waits for both panes; their tokens.
fn split(h: &mut Harness<'_>, session: Uuid) -> (String, String) {
    h.dispatch(Command::Session(SessionCmd::Split), session, |_| {});
    h.until("both panes to start", |h| {
        h.realized(session, StatusPane::Left) && h.realized(session, StatusPane::Right)
    });
    (h.token(session, StatusPane::Left), h.token(session, StatusPane::Right))
}

/// Opens `exec cat` over `pane` of `session` and waits for it to start.
fn open_pane_overlay(h: &mut Harness<'_>, session: Uuid, pane: OverlayPane) {
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.command = Some("exec cat".to_owned());
        args.pane = Some(pane.wire_name().to_owned());
    });
    h.until("the pane overlay to start", |h| pane_overlay_realized(h, session, pane));
}

fn pane_overlay_realized(h: &Harness<'_>, session: Uuid, pane: OverlayPane) -> bool {
    h.session(session)
        .pane_overlays
        .slot(pane)
        .surface
        .get()
        .is_some_and(ngterm_host::Surface::is_realized)
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "runs a POSIX sh command line as its overlay program")]
fn a_hidden_program_hands_the_keys_to_the_pane_and_show_brings_the_same_program_back(cx: &mut TestAppContext) {
    let (mut h, session, token) = overlay_harness(cx);
    let dir = tempfile::tempdir().unwrap();
    let (keys, pids) = (dir.path().join("keys"), dir.path().join("pids"));
    let command = format!(
        "echo $$ >> {}; while IFS= read -r x; do printf '%s\\n' \"$x\" >> {}; done",
        quoted(&pids),
        quoted(&keys)
    );
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.command = Some(command);
    });
    h.until("the program to start", |h| program_realized(h, session));
    assert_eq!(program(&mut h, session), Some((true, true)));
    let shared = program_shared(&mut h, session);
    h.cx.simulate_keystrokes("o n e enter");
    h.until("the program to read its line", |_| read(&keys) == "one\n");

    visibility(&mut h, session, SessionCmd::OverlayHide, None);
    assert_eq!(hidden(&h, session), ["session"]);
    assert_eq!(
        program(&mut h, session),
        Some((false, false)),
        "still mounted, neither shown nor holding the keyboard"
    );
    assert!(h.shown(session, &token), "the pane shows again");
    assert!(h.focused(session, &token), "and takes the keyboard");
    h.cx.simulate_keystrokes("t w o enter");

    visibility(&mut h, session, SessionCmd::OverlayShow, None);
    assert!(hidden(&h, session).is_empty());
    assert_eq!(
        program(&mut h, session),
        Some((true, true)),
        "shown again with the keyboard"
    );
    assert!(
        Rc::ptr_eq(&shared, &program_shared(&mut h, session)),
        "the same terminal, never rebuilt"
    );
    h.cx.simulate_keystrokes("t r i enter");
    h.until("the program to read its next line", |_| read(&keys) == "one\ntri\n");
    assert_eq!(
        read(&pids).lines().count(),
        1,
        "one program read both lines and none of the pane's: hiding never restarted it"
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "runs a POSIX sh command line as its overlay program")]
fn a_hidden_pane_overlay_leaves_its_pane_clickable_and_typable(cx: &mut TestAppContext) {
    let (mut h, session, _) = overlay_harness(cx);
    let (primary, right) = split(&mut h, session);
    open_pane_overlay(&mut h, session, OverlayPane::Right);
    let overlays = pane_overlays(&mut h, session);
    let [(generation, true, true)] = overlays[..] else {
        panic!("one shown pane overlay holding the keyboard: {overlays:?}");
    };
    assert!(!h.shown(session, &right), "the pane hides under its overlay");

    visibility(&mut h, session, SessionCmd::OverlayHide, Some("right"));
    assert_eq!(hidden(&h, session), ["right"]);
    assert_eq!(
        pane_overlays(&mut h, session),
        [(generation, false, false)],
        "still mounted, neither shown nor holding the keyboard"
    );
    assert!(h.shown(session, &right) && h.focused(session, &right));
    assert!(
        pane_overlay_realized(&h, session, OverlayPane::Right),
        "its program runs on: a realized overlay is never dropped as stranded"
    );

    // the keyboard goes to the other pane; a click on the uncovered one brings it to that pane's own terminal
    click_focus(&mut h, session, &primary);
    h.turn();
    assert!(h.focused(session, &primary));
    let rect = h.deck.read_with(h.cx, |deck, _| {
        deck.page(session).unwrap().rect(Some(Slot::Split)).unwrap()
    });
    #[expect(clippy::cast_possible_truncation, reason = "test coordinates")]
    let center = gpui_kit::point(
        px((f64::from(rect.x) + f64::from(rect.width) / 2.0) as f32),
        px((f64::from(rect.y) + f64::from(rect.height) / 2.0) as f32),
    );
    h.cx.simulate_click(center, gpui_kit::Modifiers::default());
    h.turn();
    assert!(
        h.focused(session, &right),
        "a click on the uncovered pane focuses its own terminal"
    );
    assert!(h.session(session).split.focused, "the split's focus followed the click");

    // a bare toggle shows every hidden overlay: the pane overlay takes its pane's keyboard back
    visibility(&mut h, session, SessionCmd::OverlayToggle, None);
    assert!(hidden(&h, session).is_empty());
    assert_eq!(pane_overlays(&mut h, session), [(generation, true, true)]);
    assert!(!h.shown(session, &right));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "runs a POSIX sh command line as its overlay program")]
fn the_keyboard_moves_only_when_the_visible_cover_changes(cx: &mut TestAppContext) {
    let (mut h, session, _) = overlay_harness(cx);
    let (_, right) = split(&mut h, session);
    assert!(h.focused(session, &right), "the new split's pane has the keyboard");

    // the other pane's overlay coming, hiding and showing leaves the focused pane the keyboard
    open_pane_overlay(&mut h, session, OverlayPane::Left);
    let [(generation, true, false)] = pane_overlays(&mut h, session)[..] else {
        panic!("one shown pane overlay over the unfocused pane");
    };
    assert!(h.focused(session, &right));
    visibility(&mut h, session, SessionCmd::OverlayHide, Some("left"));
    assert!(h.focused(session, &right));
    visibility(&mut h, session, SessionCmd::OverlayShow, Some("left"));
    assert!(h.focused(session, &right));
    visibility(&mut h, session, SessionCmd::OverlayHide, Some("left"));
    assert_eq!(pane_overlays(&mut h, session), [(generation, false, false)]);

    // a HUD over the session takes nothing: a hidden pane overlay does not block it either
    install_hud(&mut h);
    h.dispatch(Command::Session(SessionCmd::HudOpen), session, |args| {
        args.message = Some("passing".to_owned());
    });
    h.turn();
    assert!(
        hud_generation(&mut h, session).is_some(),
        "the HUD opens over a hidden pane overlay"
    );
    assert!(h.focused(session, &right));
    h.dispatch(Command::Session(SessionCmd::HudClose), session, |_| {});
    h.turn();
    assert!(h.focused(session, &right));

    // a hidden overlay's close changes nothing the user types into: the keyboard stays wherever it is
    let elsewhere = focus_elsewhere(&mut h);
    h.dispatch(Command::Session(SessionCmd::OverlayClose), session, |args| {
        args.pane = Some("left".to_owned());
    });
    h.turn();
    assert!(pane_overlays(&mut h, session).is_empty(), "closed");
    assert!(
        h.cx.update(|window, _| elsewhere.is_focused(window)),
        "the close yanked nothing"
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "runs a POSIX sh command line as its overlay program")]
fn a_hidden_floating_program_drops_its_backdrop_and_keeps_its_size(cx: &mut TestAppContext) {
    let (mut h, session, token) = overlay_harness(cx);
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.command = Some("exec cat".to_owned());
        args.size_percent = Some(50);
    });
    h.until("the program to start", |h| program_realized(h, session));
    let half = Some(ProgramPlacement {
        sizing: Some(PanelSizing::percent(50)),
    });
    assert_eq!(program_layout(&mut h, session), (half, half, true));
    h.until("the program's grid", |h| program_cols(h, session) > 0);
    let cols = program_cols(&mut h, session);

    visibility(&mut h, session, SessionCmd::OverlayHide, None);
    assert_eq!(
        program_layout(&mut h, session),
        (None, half, false),
        "no placement and no backdrop, laid out where it shows again"
    );
    let washed = h
        .deck
        .read_with(h.cx, |deck, _| deck.page(session).unwrap().washes().clone());
    assert_eq!(washed, Washes::default(), "the washes follow the visible layer");
    assert_eq!(program(&mut h, session), Some((false, false)));
    assert!(h.focused(session, &token));
    // a resize would reach the program's grid within a few frames, as the divider's does
    for _ in 0..10 {
        h.cx.update(|window, _| window.refresh());
        h.turn();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(program_cols(&mut h, session), cols, "hiding never resized the program");

    visibility(&mut h, session, SessionCmd::OverlayShow, None);
    assert_eq!(program_layout(&mut h, session), (half, half, true));
    assert_eq!(program(&mut h, session), Some((true, true)));
    assert_eq!(program_cols(&mut h, session), cols);
}

// MARK: - HTML pages (`crate::html_overlay`, on its recording engine)

/// The recording engine and the page host on `h`'s core.
#[cfg(all(feature = "html", target_os = "linux"))]
fn page_host(h: &mut Harness<'_>) -> Rc<crate::html_overlay::testing::Recorder> {
    let web = crate::html_overlay::testing::install();
    let poster = h.state.poster().clone();
    crate::html_overlay::install(&mut h.state.core, h.state_dir.path().to_path_buf(), poster);
    web
}

#[cfg(all(feature = "html", target_os = "linux"))]
#[gpui_kit::test]
fn a_hidden_page_keeps_its_document_and_comes_back_without_a_reload(cx: &mut TestAppContext) {
    use crate::html_overlay::testing::Call;

    use super::overlay::program_page;

    let (mut h, session, token) = overlay_harness(cx);
    let web = page_host(&mut h);
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.url = Some("https://example.invalid/".to_owned());
    });
    h.turn();
    assert_eq!(program_page(&mut h, session), Some((true, true)));
    let element = |h: &mut Harness<'_>| {
        h.deck.read_with(h.cx, |deck, _| {
            deck.page(session)
                .and_then(|page| page.overlays().program_page())
                .map(|page| page.element().entity_id())
        })
    };
    let mounted = element(&mut h);
    let page_id = |h: &Harness<'_>| {
        h.session(session)
            .html_overlays()
            .into_iter()
            .map(|(_, page)| page.id)
            .collect::<Vec<_>>()
    };
    let id = page_id(&h);
    web.take();

    visibility(&mut h, session, SessionCmd::OverlayHide, None);
    assert_eq!(
        program_page(&mut h, session),
        Some((false, false)),
        "still mounted, off screen, without the keyboard"
    );
    assert!(h.focused(session, &token), "the pane under it takes the keyboard");
    assert!(
        web.take().contains(&Call::Visible(false)),
        "an unplaced page renders nothing"
    );

    visibility(&mut h, session, SessionCmd::OverlayShow, None);
    assert_eq!(program_page(&mut h, session), Some((true, true)));
    let calls = web.take();
    assert!(calls.contains(&Call::Visible(true)));
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::Load(_) | Call::Reload | Call::CloseView(_) | Call::CreateView { .. }
        )),
        "the same document came back: nothing reloaded or rebuilt: {calls:?}"
    );
    assert_eq!(element(&mut h), mounted, "the same element");
    assert_eq!(page_id(&h), id, "the same page");
}

#[cfg(all(feature = "html", target_os = "linux"))]
#[gpui_kit::test]
fn the_strips_hide_hides_only_its_own_pane(cx: &mut TestAppContext) {
    let (mut h, session, _) = overlay_harness(cx);
    let _web = page_host(&mut h);
    let (_, right) = split(&mut h, session);
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.command = Some("exec cat".to_owned());
        args.size_percent = Some(40);
    });
    h.dispatch(Command::Session(SessionCmd::OverlayOpen), session, |args| {
        args.url = Some("https://example.invalid/".to_owned());
        args.pane = Some("right".to_owned());
    });
    h.turn();
    visibility(&mut h, session, SessionCmd::OverlayHide, None);
    visibility(&mut h, session, SessionCmd::OverlayShow, Some("right"));
    assert_eq!(hidden(&h, session), ["session"]);

    let hide =
        h.cx.debug_bounds("htmlOverlay.hide")
            .expect("the shown page's strip carries Hide");
    h.cx.simulate_click(hide.center(), gpui_kit::Modifiers::default());
    h.turn();
    assert_eq!(
        hidden(&h, session),
        ["session", "right"],
        "its own pane hid, the rest stayed as it was"
    );
    assert!(h.focused(session, &right), "the pane under it takes the keyboard");
}
