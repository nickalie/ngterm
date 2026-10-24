//! Hiding overlays in the window (upstream's `ControlOverlayVisibilityUITests` and the window side of the hide-overlays
//! change): the title bar's overlay toggle and its states, the `toggle_overlay` chord from a terminal and from a
//! focused page, the View menu's row, the close confirmations naming the hidden overlays, and the alert sound for an
//! Edit item a hidden overlay refuses.

use ngterm_core::keymap::BuiltinAction;
use ngterm_core::palette_catalog::PaletteCommand;
use ngterm_core::titlebar::OverlayToggleButton;
use ngterm_proto::command::{PickCmd, SessionCmd};
use ngterm_proto::{Args, Command, Request};

use super::*;
use crate::keymap_dispatch::{Fired, KeyOrigin};

impl Shell<'_> {
    /// Runs `cmd` on `session` with `args` and lands it.
    fn session_request(&mut self, cmd: SessionCmd, session: Uuid, args: Args) {
        let response = self.state.core.dispatch(&Request {
            cmd: Command::Session(cmd),
            target: Some(ngterm_core::pane_env::uuid_string(session)),
            args: Some(args),
        });
        assert!(response.ok, "{response:?}");
        self.state.settle();
        self.settle();
    }

    /// Opens a program over `session` (its terminal starts nowhere here, so it stays up).
    fn open_program(&mut self, session: Uuid) {
        let args = Args {
            command: Some("cat".to_owned()),
            ..Args::default()
        };
        self.session_request(SessionCmd::OverlayOpen, session, args);
    }

    fn hidden(&self, session: Uuid) -> Vec<&'static str> {
        self.state
            .core
            .library()
            .store(self.window)
            .and_then(|store| store.session(session))
            .map(Session::hidden_overlay_names)
            .unwrap_or_default()
    }

    /// The title bar's overlay toggle as last synced.
    fn overlay_toggle(&mut self) -> Option<OverlayToggleButton> {
        let bar = self.view.read_with(self.cx, |view, _| view.titlebar().clone());
        bar.read_with(self.cx, |bar, _| bar.bar().and_then(|bar| bar.model.overlay_toggle))
    }

    /// Whether `session`'s program terminal holds the window's keyboard.
    fn program_focused(&mut self, session: Uuid) -> bool {
        let view = self.view.clone();
        self.cx.update(|window, cx| {
            view.read(cx)
                .deck()
                .read(cx)
                .page(session)
                .and_then(|page| page.overlays().program())
                .is_some_and(|program| program.focus_handle(cx).is_focused(window))
        })
    }

    /// The app menu's item `id` as the bar's menu holds it now.
    fn menu_item(&mut self, id: &str) -> crate::app_menu::MenuItem {
        let bar = self.view.read_with(self.cx, |view, _| view.titlebar().clone());
        bar.read_with(self.cx, |bar, _| {
            bar.bar()
                .and_then(|bar| bar.menu.items().into_iter().find(|(_, item)| item.id == id))
                .map(|(_, item)| item.clone())
        })
        .unwrap_or_else(|| panic!("no menu item {id}"))
    }

    /// Presses `keys` and releases them, as a user's press does: the key monitor reads a held key's next press as its
    /// autorepeat.
    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        assert!(
            crate::keymap_dispatch::release_simulated(keys),
            "{keys} went to the keymap"
        );
        self.settle();
    }

    /// The front alert's body.
    fn alert_body(&mut self) -> Option<String> {
        self.cx.update(|window, cx| {
            crate::modal::front_alert(window.window_handle(), cx).map(|alert| alert.read(cx).spec().body.clone())
        })
    }
}

impl Shell<'_> {
    /// Shows the title bar's overlay toggle, as its Settings toggle does.
    fn show_overlay_toggle(&mut self) {
        self.state.core.update_settings(|settings| {
            settings.shown_interface_elements = Some(vec!["overlayToggle".to_owned()]);
        });
        self.state.settle();
        self.settle();
    }
}

#[gpui_kit::test]
fn the_title_bars_overlay_toggle_hides_and_shows_the_session_overlays(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.show_overlay_toggle();
    let toggle = s.overlay_toggle().expect("enabled in Settings, the button shows");
    assert!(!toggle.enabled, "with nothing to hide it is disabled, not gone");
    assert_eq!(toggle.state_name(), "none");
    assert!(s.cx.debug_bounds("overlay-toggle").is_some(), "drawn");

    let var = s.session("var").unwrap();
    s.open_program(var);
    let toggle = s.overlay_toggle().unwrap();
    assert!(toggle.enabled, "an overlay enables it");
    assert_eq!((toggle.label(), toggle.state_name()), ("Hide Overlays", "shown"));
    assert!(s.program_focused(var));

    s.click("overlay-toggle");
    s.settle();
    assert_eq!(s.hidden(var), ["session"]);
    let toggle = s.overlay_toggle().unwrap();
    assert_eq!((toggle.label(), toggle.state_name()), ("Show Overlays", "hidden"));
    assert!(toggle.any_hidden, "the glyph slashes while anything is hidden");
    assert!(
        s.terminal_focused(),
        "the hidden program hands the keyboard to the terminal"
    );

    s.click("overlay-toggle");
    s.settle();
    assert!(s.hidden(var).is_empty());
    assert_eq!(s.overlay_toggle().unwrap().state_name(), "shown");
    assert!(s.program_focused(var), "the program has the keyboard back");
}

#[gpui_kit::test]
fn an_open_picker_disables_the_overlay_toggle(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.show_overlay_toggle();
    let var = s.session("var").unwrap();
    s.open_program(var);
    assert!(s.overlay_toggle().unwrap().enabled);
    // as it does the menu item: the window's own covers disable it
    let pick = open_pick(&mut s);
    assert!(!s.overlay_toggle().unwrap().enabled);
    let response = s.state.core.dispatch(&Request {
        cmd: Command::Pick(PickCmd::Cancel),
        target: Some(pick),
        args: None,
    });
    assert!(response.ok, "{response:?}");
    s.state.settle();
    s.settle();
    assert!(s.overlay_toggle().unwrap().enabled);
}

#[gpui_kit::test]
fn the_toggle_overlay_chord_hides_and_shows_from_the_terminal(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.with_keys();
    let chord = s
        .state
        .core
        .keymap()
        .equivalent(BuiltinAction::ToggleOverlay)
        .cloned()
        .expect("bound by default");
    let keys = keystroke(&chord);
    let var = s.session("var").unwrap();
    s.open_program(var);
    assert!(s.program_focused(var));
    s.press(&keys);
    assert_eq!(s.hidden(var), ["session"], "{keys} over a program hides it");
    assert!(s.terminal_focused());
    s.press(&keys);
    assert!(s.hidden(var).is_empty(), "{keys} again shows it");
    assert!(s.program_focused(var));
}

#[cfg(all(feature = "html", target_os = "linux"))]
#[gpui_kit::test]
fn the_toggle_overlay_chord_reaches_the_keymap_from_a_focused_page(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    s.with_keys();
    let _web = crate::html_overlay::testing::install();
    let poster = s.state.poster().clone();
    let pages = tempfile::tempdir().unwrap();
    crate::html_overlay::install(&mut s.state.core, pages.path().to_path_buf(), poster);
    let keys = keystroke(
        s.state
            .core
            .keymap()
            .equivalent(BuiltinAction::ToggleOverlay)
            .expect("bound by default"),
    );
    let var = s.session("var").unwrap();
    let args = Args {
        url: Some("https://example.invalid/".to_owned()),
        ..Args::default()
    };
    s.session_request(SessionCmd::OverlayOpen, var, args);
    let view = s.view.clone();
    let page_focused = |s: &mut Shell<'_>| {
        s.cx.update(|window, cx| {
            view.read(cx)
                .deck()
                .read(cx)
                .page(var)
                .and_then(|page| page.overlays().program_page())
                .is_some_and(|page| gpui_kit::Focusable::focus_handle(page.element().read(cx), cx).is_focused(window))
        })
    };
    assert!(page_focused(&mut s), "the page holds the keyboard");
    // the key monitor runs the keymap before the page's engine sees the key: no special case for pages
    s.press(&keys);
    assert_eq!(s.hidden(var), ["session"], "{keys} from a focused page hides it");
    assert!(s.terminal_focused());
    s.press(&keys);
    assert!(s.hidden(var).is_empty());
    assert!(page_focused(&mut s), "and the page takes the keyboard back");
}

#[gpui_kit::test]
fn the_view_menus_overlays_row_names_what_it_will_do_and_toggles(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    let item = s.menu_item("toggleOverlay");
    // upstream's `anyOverlayShown(nil) ?? true`: with a session and nothing shown, the row offers to show
    assert_eq!(item.title, "Show Overlays");
    assert!(!item.enabled, "with nothing to hide it is greyed");
    let var = s.session("var").unwrap();
    s.open_program(var);
    let item = s.menu_item("toggleOverlay");
    assert!(item.enabled);
    assert_eq!(
        item.chord.as_ref(),
        s.state.core.keymap().equivalent(BuiltinAction::ToggleOverlay),
        "the chord is toggle_overlay's"
    );
    // tall enough for View's rows to fit on screen
    s.cx.simulate_resize(size(px(800.), px(1400.)));
    s.settle();
    s.click("app-menu-button");
    s.settle();
    // View is the second submenu: Down twice lands on it, Right opens it
    s.cx.simulate_keystrokes("down down right");
    s.cx.run_until_parked();
    // a menu row runs on a click once the pointer is over it, as a user's click comes
    let row =
        s.cx.debug_bounds("app-menu-item-toggleOverlay")
            .expect("View lists the overlays row")
            .center();
    s.cx.simulate_mouse_move(row, None, gpui_kit::Modifiers::default());
    s.click("app-menu-item-toggleOverlay");
    s.settle();
    assert_eq!(s.hidden(var), ["session"]);
    assert_eq!(s.menu_item("toggleOverlay").title, "Show Overlays");
    assert!(s.terminal_focused());
}

#[gpui_kit::test]
fn a_close_confirmation_names_the_hidden_overlays_that_close_with_the_session(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    let var = s.session("var").unwrap();
    s.open_program(var);
    s.session_request(SessionCmd::OverlayHide, var, Args::default());
    assert_eq!(s.hidden(var), ["session"]);
    let body = "The session will close immediately and can be reopened from File > Open Recent. Its hidden overlays \
                close with it.";

    s.close_var_from_its_menu();
    assert_eq!(s.alert_body().as_deref(), Some(body));
    s.cx.simulate_keystrokes("escape");
    s.settle();

    // Close Session with only a hidden overlay up skips it and asks to close the session, naming it
    let window = s.window;
    s.state.keymap_fired(
        KeyOrigin::Window(window),
        Fired::Builtin(BuiltinAction::CloseSession),
        None,
    );
    let actions = s.state.take_frontend_work().actions;
    assert_eq!(
        actions.iter().map(|(action, _)| action.clone()).collect::<Vec<_>>(),
        [FrontendAction::ConfirmCloseSession { window, session: var }]
    );
    let poster = s.state.poster().clone();
    for (action, grace) in actions {
        s.cx.cx
            .update(|cx| crate::keymap_actions::perform(&poster, action, grace, cx));
    }
    s.settle();
    assert_eq!(s.alert_body().as_deref(), Some(body));
    assert_eq!(
        s.hidden(var),
        ["session"],
        "the hidden overlay is still up behind the question"
    );
    s.cx.simulate_keystrokes("enter");
    s.settle();
    assert!(s.session("var").is_none(), "confirmed");
}

#[gpui_kit::test]
fn an_edit_item_a_hidden_overlay_refuses_sounds_the_alert_and_keeps_it_hidden(cx: &mut TestAppContext) {
    let mut s = shell(cx);
    let config = tempfile::tempdir().unwrap();
    let directory = config.path().display().to_string();
    s.state
        .core
        .update_settings(|settings| settings.config_directory = Some(directory));
    let var = s.session("var").unwrap();
    s.open_program(var);
    s.session_request(SessionCmd::OverlayHide, var, Args::default());

    // File > Edit Keymap…, as its menu item runs it
    let window = s.window;
    s.state
        .menu_activated(window, crate::app_menu::Item::Row(PaletteCommand::EditKeymap));
    let actions = s.state.take_frontend_work().actions;
    assert_eq!(
        actions.iter().map(|(action, _)| action.clone()).collect::<Vec<_>>(),
        [FrontendAction::AlertBell(s.window)],
        "the refusal has nothing on screen to explain it"
    );
    let before = crate::status_sound::alerts();
    let poster = s.state.poster().clone();
    for (action, grace) in actions {
        s.cx.cx
            .update(|cx| crate::keymap_actions::perform(&poster, action, grace, cx));
    }
    assert_eq!(crate::status_sound::alerts(), before + 1, "the alert sounded once");
    assert_eq!(s.hidden(var), ["session"], "the overlay stays hidden");
    let command = s
        .state
        .core
        .library()
        .store(s.window)
        .and_then(|store| store.session(var))
        .and_then(|session| session.overlay.command.clone());
    assert_eq!(command.as_deref(), Some("cat"), "no editor took the slot");
}
