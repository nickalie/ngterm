//! Worker launch and the shared socket protocol.
mod client;
#[doc(hidden)]
pub mod message;
#[doc(hidden)]
pub mod wire;
#[doc(hidden)]
pub use client::Client;
pub use client::{pin_worker, pin_worker_path, probe_system};
/// The variable that makes a process the worker: the number of the socket descriptor it inherited.
pub const WORKER_FD_ENV: &str = "NGTERM_WPE_WORKER_FD";
/// The descriptor of the pinned worker image (`/proc/self/fd/<n>`), closed on worker entry.
pub const WORKER_IMAGE_ENV: &str = "NGTERM_WPE_WORKER_IMAGE_FD";
/// Identity compiled into both the client and worker from the same build invocation.
#[must_use]
pub fn build_identity() -> String {
    format!(
        "{}:{}",
        env!("CARGO_PKG_VERSION"),
        option_env!("NGTERM_BUILD_COMMIT").unwrap_or("")
    )
}
