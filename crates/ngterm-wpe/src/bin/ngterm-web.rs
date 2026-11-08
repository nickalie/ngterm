//! Linux HTML overlay engine worker. The app never links `WebKit`.
fn main() {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--probe") {
        // SAFETY: main has started no threads. The loader has already consumed this variable.
        unsafe { std::env::remove_var("LD_LIBRARY_PATH") };
        // SAFETY: these version getters have no preconditions and initialize no engine.
        let major = unsafe { ngterm_wpe_sys::webkit_get_major_version() };
        // SAFETY: as above.
        let minor = unsafe { ngterm_wpe_sys::webkit_get_minor_version() };
        // SAFETY: as above.
        let micro = unsafe { ngterm_wpe_sys::webkit_get_micro_version() };
        println!("{major}.{minor}.{micro}");
        return;
    }
    // SAFETY: these are the first calls in main, before any threads exist.
    unsafe { ngterm_wpe::launcher::prepare_worker() };
    // SAFETY: preparation has started no threads; the worker still owns its startup environment.
    unsafe { ngterm_wpe::worker::serve_if_asked(|| ngterm_app::logging::init(&ngterm_core::paths::ProcessEnv)) };
    eprintln!("ngterm-web: this worker must be started by ngterm");
    std::process::exit(2);
}
