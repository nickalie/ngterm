//! The LLVM coverage profile of an instrumented build, written before an exit that skips the exit handlers (the app's
//! `exit_now`, `ngterm-headless`'s relaunch by `exec`): `ngterm_session::coverage`'s, for the programs above the session
//! crate.

pub use ngterm_session::coverage::{write_profile, write_profile_on_termination};
