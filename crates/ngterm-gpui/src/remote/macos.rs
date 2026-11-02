//! `NWPathMonitor` reports every path update; workspace wake notifications use the same retry seam.
use super::{LinkEvent, Report};
use std::sync::OnceLock;
static REPORT: OnceLock<Report> = OnceLock::new();
unsafe extern "C" {
    fn ngterm_network_start(report: extern "C" fn(i32));
}
extern "C" fn changed(value: i32) {
    if let Some(report) = REPORT.get() {
        report(if value == 2 {
            LinkEvent::Resumed
        } else {
            LinkEvent::Network { available: value == 1 }
        });
    }
}
/// Start once, on the application's main thread. The monitor retains only a static callback.
pub fn start(report: &Report) {
    if REPORT.set(report.clone()).is_ok() {
        // SAFETY: main-thread native initialization; reports may arrive on any thread.
        unsafe { ngterm_network_start(changed) };
    }
}
