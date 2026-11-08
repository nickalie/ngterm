//! ngterm's frontend-neutral app services: the host pieces every frontend shares that need no window or toolkit, so
//! a frontend keeps only its widget glue over them.
//!
//! - [`coverage`]: the coverage profile of an instrumented build, written before an exit that skips its handlers.
//! - [`documents`]: bounded local image and Markdown reads for document overlays.
//! - [`hook_runner`]: the [`ngterm_host::ProcessLauncher`] for hooks and custom commands.
//! - [`process_table`]: this OS's [`ngterm_host::ProcessTable`], `ngterm-session`'s.
//! - [`restore_launch`]: the restore decision a launch takes before its first terminal.
//! - [`log_follower`]: a log page's follower, which reads its file's end while the page follows it.
//! - [`logging`]: `tracing` to stderr and to the state directory's log file.
//! - [`quick_placement`]: the quick terminal's host and placement decisions.
//! - [`integrations`]: the agent integrations' planners and installers.
//! - [`remote`]: the presentation bridge to an attached row's origin as a child process.
//! - [`page_channel`]: the helper carrying a handed-over page's channel to its origin as a child process.

pub mod coverage;
pub mod documents;
pub mod hook_runner;
pub mod integrations;
pub mod log_follower;
pub mod logging;
pub mod page_channel;
pub mod process_table;
pub mod quick_placement;
pub mod remote;
pub mod restore_launch;
#[cfg(test)]
mod test_shell;
