//! Linux HTML overlay engine worker. The app never links `WebKit`.
fn main() {
    // SAFETY: these are the first calls in main, before any threads exist.
    unsafe { ngterm_wpe::launcher::prepare_worker() };
    // SAFETY: preparation has started no threads; the worker still owns its startup environment.
    unsafe { ngterm_wpe::worker::serve_if_asked(|| ngterm_app::logging::init(&ngterm_core::paths::ProcessEnv)) };
    eprintln!("ngterm-web: this worker must be started by ngterm");
    std::process::exit(2);
}
