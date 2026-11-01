use std::cell::RefCell;

use gpui_kit::{TestAppContext, hsla};

use super::*;

/// Two menu buttons, `outside` out of the Tab order and `inside` in it, recording which menu opened.
struct Probe {
    opened: Rc<RefCell<Vec<&'static str>>>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str| {
            let opened = Rc::clone(&self.opened);
            Button::new(id)
                .size(px(24.))
                .wash(hsla(0., 0., 1., 1.))
                .menu(Anchor::TopLeft, |menu, _, _| menu)
                .on_menu_open_change(move |open, _, _| {
                    if *open {
                        opened.borrow_mut().push(id);
                    }
                })
        };
        div()
            .size_full()
            .flex()
            .child(button("outside"))
            .child(button("inside").in_tab_order())
    }
}

/// A menu button takes the keyboard's focus: Tab reaches the one in the Tab order and never the other, and Enter
/// opens the focused one's menu, as the library's own button did.
#[gpui_kit::test]
fn tab_reaches_a_menu_button_in_the_tab_order_and_enter_opens_its_menu(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let probe_opened = Rc::clone(&opened);
    let (_view, cx) = cx.add_window_view(move |_, _| Probe { opened: probe_opened });
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update(Window::focus_next);
        cx.run_until_parked();
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), vec!["inside"]);
}
