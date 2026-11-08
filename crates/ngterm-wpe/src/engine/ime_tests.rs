use std::cell::RefCell;
use std::rc::Rc;

use super::*;

fn method() -> InputMethod {
    glib::Object::new()
}

fn preedit(method: &InputMethod) -> (String, u32) {
    let raw = method.upcast_ref::<WpeInputMethodContext>().to_glib_none().0;
    let mut text = ptr::null_mut();
    let mut cursor = u32::MAX;
    let mut underlines = ptr::dangling_mut();
    // SAFETY: a live context and writable out-pointers; GLib transfers ownership of the string.
    unsafe {
        platform::wpe_input_method_context_get_preedit_string(raw, &raw mut text, &raw mut underlines, &raw mut cursor);
    }
    assert!(underlines.is_null());
    // SAFETY: the getter returned an owned GLib string.
    let text: glib::GString = unsafe { glib::translate::from_glib_full(text) };
    (text.to_string(), cursor)
}

#[test]
fn preedit_reports_character_positions_and_reset_discards_the_composition() {
    let method = method();
    let signals = Rc::new(RefCell::new(Vec::new()));
    for signal in ["preedit-started", "preedit-changed", "preedit-finished"] {
        let signals = Rc::clone(&signals);
        method.connect_local(signal, false, move |_| {
            signals.borrow_mut().push(signal);
            None
        });
    }
    method.preedit("にほ", 2);
    assert_eq!(preedit(&method), ("にほ".to_owned(), 2));
    method.preedit("日本語", 1);
    assert_eq!(preedit(&method), ("日本語".to_owned(), 1));
    method.preedit("", 0);
    assert_eq!(preedit(&method), (String::new(), 0));
    assert_eq!(
        *signals.borrow(),
        [
            "preedit-started",
            "preedit-changed",
            "preedit-changed",
            "preedit-changed",
            "preedit-finished"
        ]
    );
    method.preedit("next", 3);
    let raw = method.upcast_ref::<WpeInputMethodContext>().to_glib_none().0;
    // SAFETY: a live input method context.
    unsafe { platform::wpe_input_method_context_reset(raw) };
    assert_eq!(preedit(&method), (String::new(), 0));
}

#[test]
fn a_commit_without_preedit_emits_the_text_and_optional_outputs_can_be_omitted() {
    let method = method();
    let commits = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&commits);
    method.connect_local("committed", false, move |values| {
        seen.borrow_mut().push(values[1].get::<String>().unwrap());
        None
    });
    method.commit("日本");
    assert_eq!(*commits.borrow(), ["日本"]);
    let raw = method.upcast_ref::<WpeInputMethodContext>().to_glib_none().0;
    // SAFETY: a live context; the vfunc accepts null optional outputs.
    unsafe { get_preedit_string(raw, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) };
    // SAFETY: a live input method context.
    unsafe { platform::wpe_input_method_context_focus_in(raw) };
    // SAFETY: as above.
    unsafe { platform::wpe_input_method_context_focus_out(raw) };
    // SAFETY: a live context, a valid UTF-8 string and positions within it.
    unsafe { platform::wpe_input_method_context_set_surrounding(raw, c"text".as_ptr(), 4, 2, 2) };
    // SAFETY: a live context; it has no view to report the cursor to.
    unsafe { set_cursor_area(raw, 1, 2, 3, 4) };
    assert!(method.view().is_none());
    assert_eq!(preedit(&method), (String::new(), 0));
}
