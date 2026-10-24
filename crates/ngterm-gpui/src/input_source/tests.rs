//! The performer over a fake backend: it talks to nothing while the setting is off, engages and restores through the
//! backend, switches a manual change back, drops an override the system refused, releases a layout no longer enabled,
//! restores on stop, re-evaluates when a window closes, coalesces refresh requests, and offers the picker a backend's layouts or its reason.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestAppContext;

use super::*;

const US: &str = "English (US)";
const RU: &str = "Russian";

/// What the fake system holds and what was asked of it.
#[derive(Default)]
struct System {
    enabled: Vec<String>,
    current: String,
    selected: Vec<String>,
    refuse: bool,
    probes: usize,
    reads: usize,
}

struct Fake(Rc<RefCell<System>>);

impl Backend for Fake {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        let mut system = self.0.borrow_mut();
        system.reads += 1;
        Ok(system.enabled.iter().cloned().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        Ok(self.0.borrow().current.clone())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let mut system = self.0.borrow_mut();
        if system.refuse {
            return Err("refused".into());
        }
        system.selected.push(id.to_owned());
        system.current = id.to_owned();
        Ok(())
    }
}

/// The performer over a fake system on `current`, with US and Russian enabled.
fn installed(cx: &mut TestAppContext, current: &str) -> Rc<RefCell<System>> {
    let system = Rc::new(RefCell::new(System {
        enabled: vec![US.to_owned(), RU.to_owned()],
        current: current.to_owned(),
        ..System::default()
    }));
    let probed = Rc::clone(&system);
    cx.update(|cx| {
        install(
            Box::new(move || {
                probed.borrow_mut().probes += 1;
                Ok(Box::new(Fake(probed)) as Box<dyn Backend>)
            }),
            None,
            cx,
        );
    });
    system
}

/// Re-evaluates with a terminal holding the keyboard or not.
fn reconcile(cx: &mut TestAppContext, held: bool) {
    cx.update(|cx| cx.global_mut::<Performer>().reconcile(held));
}

fn target(cx: &mut TestAppContext, id: Option<&str>) {
    cx.update(|cx| cx.global_mut::<Performer>().target = id.map(str::to_owned));
}

#[gpui_kit::test]
fn the_setting_off_talks_to_nothing(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    reconcile(cx, true);
    cx.update(|cx| set_target(None, cx));
    cx.update(request_refresh);
    cx.run_until_parked();
    cx.update(stop);
    assert_eq!(system.borrow().probes, 0, "no backend was probed");
    assert!(system.borrow().selected.is_empty());
}

#[gpui_kit::test]
fn a_terminal_holding_the_keyboard_engages_and_losing_it_restores(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    target(cx, Some(US));
    reconcile(cx, true);
    assert_eq!(system.borrow().selected, [US]);
    // a manual switch while engaged is switched back
    system.borrow_mut().current = RU.to_owned();
    reconcile(cx, true);
    assert_eq!(system.borrow().selected, [US, US]);
    reconcile(cx, false);
    assert_eq!(system.borrow().selected, [US, US, RU]);
    // released: nothing more to restore
    reconcile(cx, false);
    assert_eq!(system.borrow().selected.len(), 3);
    assert_eq!(system.borrow().probes, 1, "probed once");
}

#[gpui_kit::test]
fn turning_the_setting_off_restores_at_once(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    target(cx, Some(US));
    reconcile(cx, true);
    // `set_target` re-evaluates now, reading the windows: none holds a terminal here
    cx.update(|cx| set_target(None, cx));
    assert_eq!(system.borrow().selected, [US, RU]);
}

#[gpui_kit::test]
fn a_refused_selection_leaves_nothing_to_restore(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    target(cx, Some(US));
    system.borrow_mut().refuse = true;
    reconcile(cx, true);
    system.borrow_mut().refuse = false;
    reconcile(cx, false);
    cx.update(stop);
    assert!(system.borrow().selected.is_empty());
}

#[gpui_kit::test]
fn a_layout_disabled_while_engaged_releases_and_a_missing_one_never_engages(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    target(cx, Some(US));
    reconcile(cx, true);
    system.borrow_mut().enabled = vec![RU.to_owned()];
    reconcile(cx, true);
    assert_eq!(system.borrow().selected, [US, RU]);
    target(cx, Some("German"));
    reconcile(cx, true);
    assert_eq!(system.borrow().selected, [US, RU], "German is not selectable");
}

#[gpui_kit::test]
fn stop_restores_and_ends_the_lock(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    target(cx, Some(US));
    reconcile(cx, true);
    cx.update(stop);
    assert_eq!(system.borrow().selected, [US, RU]);
    reconcile(cx, true);
    cx.update(request_refresh);
    cx.run_until_parked();
    assert_eq!(system.borrow().selected, [US, RU], "nothing after the stop");
}

/// An empty window's root.
struct Blank;

impl gpui_kit::Render for Blank {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

#[gpui_kit::test]
fn a_closed_window_re_evaluates_while_another_stays(cx: &mut TestAppContext) {
    let system = installed(cx, RU);
    let closing = cx.add_window(|_, _| Blank);
    let _staying = cx.add_window(|_, _| Blank);
    target(cx, Some(US));
    reconcile(cx, true);
    assert_eq!(system.borrow().selected, [US]);
    // a removed window reports no blur or deactivation to the terminal it held: its closing alone must re-evaluate
    closing
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(system.borrow().selected, [US, RU], "no terminal holds the keyboard now");
}

#[gpui_kit::test]
fn refresh_requests_coalesce_into_one_turn(cx: &mut TestAppContext) {
    let system = installed(cx, US);
    target(cx, Some(US));
    for _ in 0..3 {
        cx.update(request_refresh);
    }
    cx.run_until_parked();
    assert_eq!(system.borrow().reads, 1, "one evaluation for three requests");
}

#[gpui_kit::test]
fn the_picker_offers_the_backends_layouts_or_its_reason(cx: &mut TestAppContext) {
    let system = installed(cx, US);
    cx.update(window_activated);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| layouts(cx)),
        Layouts::default(),
        "nothing read before Settings opens"
    );
    assert_eq!(
        system.borrow().probes,
        0,
        "an activation before Settings opens talks to nothing"
    );
    cx.update(reload_layouts);
    let read = cx.update(|cx| layouts(cx));
    assert_eq!(read.selectable, [KeyboardLayout::named(US), KeyboardLayout::named(RU)]);
    assert_eq!(read.unavailable, None);
    assert!(system.borrow().selected.is_empty());
    // once read, an activation re-reads it: a layout enabled meanwhile shows
    system.borrow_mut().enabled.push("German".to_owned());
    cx.update(window_activated);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| layouts(cx)).selectable.len(), 3);

    cx.update(|cx| install(Box::new(|| Err("Not available here.".to_owned())), None, cx));
    cx.update(reload_layouts);
    let read = cx.update(|cx| layouts(cx));
    assert!(read.selectable.is_empty());
    assert_eq!(read.unavailable.as_deref(), Some("Not available here."));
}
