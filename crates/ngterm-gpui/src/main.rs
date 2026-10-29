//! `ngterm`, the GPUI frontend: one instance per application id (`instance.rs`), the core and its control
//! socket on GPUI's main thread (`app.rs`).
//!
//! Startup order matters: a process started as the HTML overlays' engine worker (`ngterm_wpe::worker`, the vendored WPE
//! `WebKit`'s engine in a process of its own) serves the engine and ends before anything of the app starts; then the
//! activation token and the startup id are read while the process has one thread and before GPUI's platform exists
//! (GPUI removes the token from the environment; the startup id is removed here), and a launch that forwards to a
//! running ngterm exits before GPUI starts and before it reads or writes the model state (it only appends to the shared
//! log). A launch that stays then claims the state directory (`ngterm_core::state_lock`) and exits 1 when another
//! process owns it, still before any read or write there, the log aside: the log comes first so a forwarded launch
//! and a refused one say so in it, and it only ever appends to a log another process holds (it rotates only a file
//! whose own lock it gets, `ngterm_app::logging`). The claim comes after the bus registration, so a second launch of
//! the same app id is still forwarded with exit 0 rather than refused.
//!
//! On Windows a release build is a GUI program (no console window of its own when Explorer or the Start menu starts
//! it); a debug build keeps its console, where its log lines and the console's Ctrl events reach it (`signals.rs`).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use ngterm_core::paths::{self, ProcessEnv};
use ngterm_core::state_lock::StateDirectoryLock;
use ngterm_gpui::app::{self, Launch};
use ngterm_gpui::instance::{self, Activation, Registration};

fn main() -> ExitCode {
    // the pages' engine worker: it serves the engine (inside the vendored WPE WebKit's overlay, where the app never
    // goes) and ends here, appending to the app's log, and touches nothing else of the app
    #[cfg(all(feature = "html", target_os = "linux"))]
    // SAFETY: first thing in main, while the process has one thread.
    unsafe {
        ngterm_wpe::worker::serve_if_asked(|| ngterm_app::logging::init(&ProcessEnv));
    }
    // `scripts/stage.sh` asks the staged binary who it is: the answer is printed before the app reads or writes
    // anything (no log, no bus, no window)
    if is_build_identity_query(std::env::args_os().skip(1)) {
        print!("{}", app::build_identity_text());
        return ExitCode::SUCCESS;
    }
    // the Live sessions reset's relauncher on Windows (`live_runtime.rs`): it waits for the app and reopens it, and
    // touches nothing else of the app
    #[cfg(windows)]
    if let Some(launched) =
        ngterm_gpui::live_runtime::relaunch_if_asked(&std::env::args_os().skip(1).collect::<Vec<_>>())
    {
        return if launched { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    let activation = Activation::from_env(&ProcessEnv);
    // GDK unsets it once read, so no child (a hook, a custom command, a shell) claims this launch's sequence
    // SAFETY: first thing in main, while the process has one thread.
    unsafe { std::env::remove_var(instance::STARTUP_ID_ENV) };
    ngterm_app::logging::init(&ProcessEnv);
    let app_id = instance::app_id(&ProcessEnv);
    let (requests, received) = async_channel::unbounded();
    // kept for the whole run: dropping it gives the name up
    // the state directory names the instance on Windows; only its path is computed here, nothing in it is read
    let _bus = match instance::register(&app_id, &paths::state_dir(&ProcessEnv), requests) {
        Ok(Registration::Primary(connection)) => {
            tracing::info!(app_id, "instance: primary");
            Some(connection)
        }
        Ok(Registration::Remote(remote)) => return forward(&remote, &activation),
        Ok(Registration::Unique(error)) => {
            tracing::warn!(%error, "instance: no session bus; running without single-instance");
            None
        }
        Err(error) => {
            tracing::error!(%error, "instance: registration failed");
            return ExitCode::FAILURE;
        }
    };
    // held for the whole process (`exit_now` never drops it); the kernel releases it at exit, a crash included
    let _state_lock = match claim_state_dir(&paths::state_dir(&ProcessEnv)) {
        Ok(lock) => lock,
        Err(status) => return status,
    };
    let status = app::run(Launch {
        app_id,
        requests: received,
        activation,
    });
    exit_now(status)
}

/// Takes ownership of the state directory before anything reads or writes it (the log aside, see the module docs), or
/// says why not on the log and stderr and gives the exit status 1: a second instance there would load every window file
/// and later overwrite the owner's saves with its stale copy (upstream's `agtermApp.claimStateDirectory`, #749). A lock
/// that cannot be taken stops the launch too: it does not show that no other process uses the directory.
fn claim_state_dir(directory: &Path) -> Result<StateDirectoryLock, ExitCode> {
    StateDirectoryLock::acquire(directory)
        .claim(directory)
        .map_err(|reason| {
            tracing::error!("{reason}");
            eprintln!("ngterm: {reason}");
            ExitCode::FAILURE
        })
}

/// Whether the arguments (without the program's name) are the build identity query alone.
fn is_build_identity_query(mut args: impl Iterator<Item = OsString>) -> bool {
    args.next().is_some_and(|arg| arg == app::BUILD_IDENTITY_FLAG) && args.next().is_none()
}

/// Hands this launch to the running primary.
fn forward(remote: &instance::Remote, activation: &Activation) -> ExitCode {
    let cwd = std::env::current_dir().ok();
    match remote.activate(activation, cwd.as_deref()) {
        Ok(()) => {
            tracing::info!(
                token = activation.token.is_some(),
                startup_id = activation.startup_id.is_some(),
                "instance: forwarded to the running instance"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "instance: could not forward to the running instance");
            ExitCode::FAILURE
        }
    }
}

/// Ends the process after the orderly shutdown, skipping the exit handlers and thread-local destructors: wgpu's GLES
/// instance (GPUI creates its wgpu instance with the GL backend beside Vulkan) terminates EGL in a TLS destructor
/// after the Wayland connection is gone and crashes every exit with SIGSEGV (`docs/spikes/20261014-wpe-dmabuf.md`,
/// quirk 12; `docs/upstream-issues/gpui-wgpu-gles-exit-segfault.md`). Everything that must outlive the process is
/// written by then: the quit save, the socket removed, the log written unbuffered. Windows ends it the same way
/// (`TerminateProcess` of itself: no DLL detach, no TLS callbacks), so the threads still waiting in a quit handler
/// (`signals.rs`) end with it.
fn exit_now(status: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    #[cfg(unix)]
    // SAFETY: `_exit` only ends the process; nothing of this process runs after it, so no invariant can break.
    unsafe {
        libc::_exit(status)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // the exit code Windows records is the status's bits
        let code = u32::from_ne_bytes(status.to_ne_bytes());
        // SAFETY: the current process's pseudo-handle; nothing of this process runs after the call returns.
        unsafe { TerminateProcess(GetCurrentProcess(), code) };
        // TerminateProcess of the current process does not return; should it, the ordinary exit ends it
        std::process::exit(status)
    }
}
