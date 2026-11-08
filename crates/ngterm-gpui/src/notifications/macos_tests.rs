//! The banner task against a recording notification center, since `UserNotifications` needs the `.app` bundle; the
//! bare test binary shows the real one refusing to start.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char};
use std::sync::{Arc, Mutex};

use futures_lite::future::{block_on, poll_once};

use super::*;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Notify(String, String, String),
    Withdraw(String),
}

thread_local! {
    /// Whether the recording center's bundle check passes.
    static BUNDLED: Cell<bool> = const { Cell::new(true) };
    static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
}

fn owned(value: *const c_char) -> String {
    // SAFETY: `run_on` passes NUL-terminated strings alive for the call.
    unsafe { CStr::from_ptr(value) }.to_str().unwrap().to_owned()
}

extern "C" fn start(_: extern "C" fn(*const c_char)) -> bool {
    BUNDLED.get()
}

extern "C" fn notify(identity: *const c_char, title: *const c_char, body: *const c_char) {
    let call = Call::Notify(owned(identity), owned(title), owned(body));
    CALLS.with_borrow_mut(|calls| calls.push(call));
}

extern "C" fn withdraw(identity: *const c_char) {
    let call = Call::Withdraw(owned(identity));
    CALLS.with_borrow_mut(|calls| calls.push(call));
}

const RECORDING: Center = Center {
    start,
    notify,
    withdraw,
};

fn send(identity: &str, title: &str, body: &str) -> Banner {
    Banner::Send {
        identity: identity.to_owned(),
        title: title.to_owned(),
        body: body.to_owned(),
    }
}

fn withdrawal(identity: &str) -> Banner {
    Banner::Withdraw {
        identity: identity.to_owned(),
    }
}

/// Runs the task over the recording center until `banners` end, with their sender already closed.
fn deliver(banners: Vec<Banner>) -> Vec<Call> {
    let (sender, receiver) = async_channel::unbounded();
    for banner in banners {
        sender.try_send(banner).unwrap();
    }
    drop(sender);
    let (_clicks_sender, clicks) = async_channel::unbounded();
    block_on(run_on(RECORDING, receiver, clicks, |_| {}));
    CALLS.take()
}

#[test]
fn banners_reach_the_center_in_order() {
    let calls = deliver(vec![send("a", "Title", "Body"), withdrawal("b"), send("c", "T", "")]);
    assert_eq!(
        calls,
        [
            Call::Notify("a".to_owned(), "Title".to_owned(), "Body".to_owned()),
            Call::Withdraw("b".to_owned()),
            Call::Notify("c".to_owned(), "T".to_owned(), String::new()),
        ]
    );
}

#[test]
fn nul_bytes_are_dropped_rather_than_cutting_a_field_short() {
    let calls = deliver(vec![send("i\0d", "ti\0tle", "bo\0dy"), withdrawal("\0gone")]);
    assert_eq!(
        calls,
        [
            Call::Notify("id".to_owned(), "title".to_owned(), "body".to_owned()),
            Call::Withdraw("gone".to_owned()),
        ]
    );
}

#[test]
fn a_center_that_will_not_start_gets_no_banner() {
    BUNDLED.set(false);
    assert_eq!(deliver(vec![send("a", "Title", "Body"), withdrawal("a")]), []);
}

#[test]
fn a_click_reports_its_identity_with_no_activation_token() {
    let (banners_sender, banners) = async_channel::unbounded();
    let (clicks_sender, clicks) = async_channel::unbounded();
    let seen: Arc<Mutex<Vec<Click>>> = Arc::default();
    let recorded = Arc::clone(&seen);
    clicks_sender.try_send("pane-1".to_owned()).unwrap();
    clicks_sender.try_send("pane-2".to_owned()).unwrap();
    let mut task = Box::pin(run_on(RECORDING, banners, clicks, move |click| {
        recorded.lock().unwrap().push(click);
    }));
    assert!(
        block_on(poll_once(task.as_mut())).is_none(),
        "the task waits for banners"
    );
    let seen_now: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|click| (click.identity.clone(), click.token.clone()))
        .collect();
    assert_eq!(seen_now, [("pane-1".to_owned(), None), ("pane-2".to_owned(), None)]);
    drop(banners_sender);
    block_on(task);
}

/// Both halves touch the process's one click slot, so they run as one test: the native callback queues what it is
/// given, lossily if it is not UTF-8, and the real center refuses a binary outside an `.app` bundle.
#[test]
fn native_clicks_queue_their_identity_and_a_bare_binary_runs_no_banners() {
    let (sender, clicks) = async_channel::unbounded();
    CLICKS.set(sender).unwrap();
    clicked(c"pane-7".as_ptr());
    clicked(c"caf\xFF".as_ptr());
    assert_eq!(clicks.try_recv().as_deref(), Ok("pane-7"));
    assert_eq!(clicks.try_recv().as_deref(), Ok("caf\u{FFFD}"));

    let (banners_sender, banners) = async_channel::unbounded();
    banners_sender.try_send(send("a", "Title", "Body")).unwrap();
    let mut task = Box::pin(run(banners, String::new(), |_| {}));
    assert!(
        block_on(poll_once(task.as_mut())).is_some(),
        "with no bundle the task ends at once, without waiting for banners"
    );
    assert_eq!(banners_sender.len(), 1, "and leaves them unread");
}
