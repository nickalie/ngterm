//! Surface actions over a real PTY, shared with the input and paint edge-case tests.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, point, px, size};
use ngterm_core::ghostty_config::{DEFAULTS, Loader};
use ngterm_host::TextScope;
use ngterm_term::{GridPoint, Pane, PaneCommand, PaneConfig, SelectionText};

use super::super::{Shared, Terminal, TerminalConfig, TerminalEvent, TerminalSpec, TerminalView, ViewRole};
use super::*;

pub(crate) const ECHO: &str = "stty raw -echo; printf 'ready\\r\\n'; exec cat -v";

pub(crate) fn config(extra: &str) -> TerminalConfig {
    let mut loader = Loader::new(None);
    loader.load_text("ghostty-defaults.conf", DEFAULTS);
    loader.load_text(
        "test.conf",
        &format!("font-size = 12\nwindow-padding-x = 2\nwindow-padding-y = 2\n{extra}"),
    );
    TerminalConfig::from_ghostty(&loader.finish().finalize(false, &[]).config)
}

pub(crate) struct Fixture<'a> {
    pub(crate) terminal: Entity<Terminal>,
    pub(crate) view: Entity<TerminalView>,
    pub(crate) events: Rc<RefCell<Vec<TerminalEvent>>>,
    pub(crate) cx: &'a mut VisualTestContext,
    _dir: tempfile::TempDir,
}

impl<'a> Fixture<'a> {
    pub(crate) fn new(cx: &'a mut TestAppContext, script: &str, extra: &str) -> Self {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let mut command = PaneCommand::new("/bin/sh").arg("-c").arg(script).cwd(dir.path());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") || key.to_string_lossy().starts_with("AGTERM_") {
                command = command.env_remove(key);
            }
        }
        let spawn: super::super::Spawn = Box::new(move |config: PaneConfig| Pane::spawn(&command, &config));
        // settle the test platform's initial scale before the child can draw a full screen
        let terminal = cx.new(|cx| Terminal::gated(TerminalSpec::default(), config(extra), Box::new(|| {}), cx));
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        cx.update(|cx| {
            cx.subscribe(&terminal, move |_, event: &TerminalEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let window = cx.open_window(size(px(640.), px(480.)), |_, cx| {
            TerminalView::new(&terminal, ViewRole::Owner, cx)
        });
        let view = window.root(cx).unwrap();
        let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
        cx.simulate_scale_factor_change(1.0);
        cx.run_until_parked();
        terminal.read_with(cx, |terminal, _| terminal.shared().launch_with(spawn, false));
        let mut fixture = Self {
            terminal,
            view,
            events,
            cx,
            _dir: dir,
        };
        fixture.wait("ready");
        fixture
    }

    pub(crate) fn shared(&self) -> Rc<Shared> {
        self.terminal
            .read_with(self.cx, |terminal, _| Rc::clone(terminal.shared()))
    }

    pub(crate) fn pane(&self) -> Rc<Pane> {
        self.shared().pane().unwrap()
    }

    pub(crate) fn settle(&mut self, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.cx.run_until_parked();
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out: {:?}; all: {:?}; grid: {:?}",
                self.text(),
                self.pane().read_text(ngterm_term::TextRegion::All),
                self.shared().laid.get()
            );
            std::thread::yield_now();
        }
    }

    pub(crate) fn text(&self) -> String {
        self.view.read_with(self.cx, |view, _| {
            view.painted().map(|frame| frame.lines().join("\n")).unwrap_or_default()
        })
    }

    pub(crate) fn wait(&mut self, needle: &str) {
        self.settle(|fixture| fixture.text().contains(needle));
    }

    pub(crate) fn configure(&mut self, extra: &str) {
        self.terminal
            .update(self.cx, |terminal, cx| terminal.set_config(config(extra), cx));
        self.cx.run_until_parked();
    }

    pub(crate) fn perform(&self, action: &str) -> bool {
        self.terminal.read_with(self.cx, |terminal, _| {
            terminal.surface().perform(&action.parse().unwrap())
        })
    }

    pub(crate) fn selection(&self) -> Option<String> {
        self.pane().selection_text(SelectionText::Raw)
    }

    pub(crate) fn at(col: u16, row: u16) -> gpui_kit::Point<gpui_kit::Pixels> {
        point(px(7.0 + f32::from(col) * 10.0), px(12.0 + f32::from(row) * 21.0))
    }

    pub(crate) fn bounds() -> gpui_kit::Bounds<gpui_kit::Pixels> {
        gpui_kit::Bounds::new(point(px(0.), px(0.)), size(px(640.), px(480.)))
    }
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn injected_text_commits_preedit_before_embedded_and_final_returns(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, ECHO, "");
    f.shared().set_preedit(Some("compose".into()));
    let mut surface = TerminalSurface::new(f.shared());
    assert!(surface.type_text("first\rsecond\n"));
    f.wait("composefirst^Msecond^M");
    assert_eq!(f.shared().preedit(), None);
    assert!(surface.binding_action("not_a_binding"));
    assert!(!surface.perform(&Action::CopyToClipboard(CopyFormat::Plain)));
}

#[test]
fn every_selection_adjustment_keeps_its_direction() {
    use binds::SelectionAdjust as Core;
    for (input, expected) in [
        (Core::Left, SelectionAdjust::Left),
        (Core::Right, SelectionAdjust::Right),
        (Core::Up, SelectionAdjust::Up),
        (Core::Down, SelectionAdjust::Down),
        (Core::PageUp, SelectionAdjust::PageUp),
        (Core::PageDown, SelectionAdjust::PageDown),
        (Core::Home, SelectionAdjust::Home),
        (Core::End, SelectionAdjust::End),
        (Core::BeginningOfLine, SelectionAdjust::BeginningOfLine),
        (Core::EndOfLine, SelectionAdjust::EndOfLine),
    ] {
        assert_eq!(selection_adjust(input), expected);
    }
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn scroll_actions_move_by_rows_pages_and_truncated_fractions(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, "seq 1 80; printf 'ready'; exec cat", "");
    assert!(f.perform("scroll_to_top"));
    f.settle(|f| f.text().starts_with("1\n2\n"));
    for (action, expected) in [
        ("scroll_page_down", "23\n24\n"),
        ("scroll_page_up", "1\n2\n"),
        ("scroll_page_lines:4", "5\n6\n"),
        ("scroll_page_fractional:0.25", "10\n11\n"),
        ("scroll_page_fractional:-0.25", "5\n6\n"),
    ] {
        assert!(f.perform(action), "{action}");
        f.settle(|f| f.text().starts_with(expected));
    }
    assert!(f.perform("jump_to_prompt:1"));
    assert!(f.perform("scroll_to_bottom"));
    f.wait("ready");
    let surface = TerminalSurface::new(f.shared());
    assert!(surface.read_text(TextScope::All).unwrap().starts_with("1\n2\n"));
    assert!(!surface.read_text(TextScope::Screen).unwrap().starts_with("1\n2\n"));
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn copy_clears_selection_only_when_configured(cx: &mut TestAppContext) {
    let f = Fixture::new(cx, "printf 'ready'; exec cat", "selection-clear-on-copy = true\n");
    f.pane()
        .select(GridPoint::viewport(0, 0), GridPoint::viewport(4, 0), false)
        .unwrap();
    assert!(f.perform("copy_to_clipboard"));
    f.cx.run_until_parked();
    assert_eq!(
        f.cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
        Some("ready")
    );
    assert_eq!(f.selection(), None);
    assert!(!f.perform("copy_to_clipboard"));
}
