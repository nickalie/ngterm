//! Native callback values reach the report supplied at startup; later starts cannot replace it.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

use super::*;

#[test]
fn callbacks_keep_the_first_report_and_translate_network_and_wake_values() {
    // Before initialization a callback has no recipient.
    changed(2);
    let (sent, received) = mpsc::channel();
    let report: Report = Arc::new(move |event| {
        let _ = sent.send(event);
    });
    // Supply the recipient directly: starting NSWorkspace's observer requires AppKit's main thread.
    assert!(REPORT.set(report).is_ok());
    let replacements = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&replacements);
    let replacement: Report = Arc::new(move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
    });
    super::super::observe_links(crate::state::detached_poster());
    start(&replacement);
    std::thread::spawn(|| {
        for value in [0, 1, 2, -1] {
            changed(value);
        }
    })
    .join()
    .unwrap();
    assert_eq!(
        received.try_iter().collect::<Vec<_>>(),
        [
            LinkEvent::Network { available: false },
            LinkEvent::Network { available: true },
            LinkEvent::Resumed,
            LinkEvent::Network { available: false },
        ]
    );
    assert_eq!(replacements.load(Ordering::SeqCst), 0);
}
