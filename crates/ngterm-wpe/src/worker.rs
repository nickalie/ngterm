//! The engine in a process of its own: the worker, which a vendored WPE `WebKit` needs (crate `launcher`) and
//! [`crate::Host::start_worker`] asks for anywhere.
//!
//! The vendored package's helpers sit at a path compiled into its library, reachable only inside a mount namespace
//! that overlays the package on `/usr/lib` (`bwrap --ro-overlay`). That namespace comes with a user namespace and
//! `PR_SET_NO_NEW_PRIVS`, which every process started inside it inherits, so the app itself must never run there: its
//! terminals, their sessions and everything they start would lose `sudo` and every setuid program, and see root's
//! files as `nobody`'s. Only `WebKit` goes there: the client launches the sibling `ngterm-web` image
//! pinned by descriptor at app startup; the worker re-executes itself under the vendored overlay when needed, and drives the engine through a [`crate::Host`] whose
//! calls travel over a socket. The worker runs the ordinary in-process engine and nothing else.
//!
//! The worker's environment is the app's plus `PULSE_PROP` naming a least request size for `pipewire-pulse`, which
//! `WebKit`'s web process inherits: its Web Audio stream otherwise stalls on a Bluetooth A2DP sink (`client.rs` has the
//! measurements). Only the worker carries it, so an engine in the consumer's own process ([`crate::Host::start`] with
//! the system's package) plays without it, and nothing else the consumer starts sees it.
//! A downloaded runtime gives only the worker its GIO, `GStreamer` and injected-bundle paths;
//! these module settings never enter the app's terminal or session-server environment.
//!
//! - The `ngterm-web` binary calls [`serve_if_asked`] after preparing its worker environment.
//! - The transport (`wire`): an `AF_UNIX` `SOCK_SEQPACKET` socket pair, close-on-exec on both sides (the worker's end
//!   is kept open across its exec alone and made close-on-exec again first thing), one JSON message per packet
//!   (`message`), large bodies and blobs in sealed `memfd`s, a dmabuf's planes passed as descriptors. A call on the
//!   app's side only queues: a writer thread and a reader thread own the socket, so neither GPUI's main thread nor the
//!   engine's ever blocks on it. The app's queue is bounded: a worker that stops reading breaks the transport, as its
//!   death would.
//! - The start: the worker says hello (protocol, build, pid) and the app hands it the configuration; the engine's start
//!   comes back as the in-process one would (a missing render node fails as `StartError::Display`), all within
//!   `START_TIMEOUT`, after which the worker is killed. The worker runs in a process group of its own, so a terminal's
//!   Ctrl-C reaches the app alone; it ends when the app quits it or goes away (the end of its socket), never through
//!   `bwrap --die-with-parent`, which would kill it with the thread that started it.
//! - The end: [`crate::Host::shutdown`] quits the engine (pages closed and every web process ended, its last answer
//!   flushed before the worker exits) and waits `STOP_TIMEOUT` for it, then closes the socket, which frees a writer
//!   stuck on a worker that stopped reading, and reaps the worker, killing its group when it does not end at once. A
//!   worker whose app went away (the end of its socket) stops its engine the same way before it ends. A worker that
//!   ends unasked answers every evaluation still waiting with an error and closes the event channel: the consumer's
//!   pages cannot come back in this process.
//! - Faults: [`FAULT_ENV`], set by the conformance suite alone, makes the worker break its protocol on purpose
//!   (`serve::Fault`), so the app's handling of a broken worker runs end to end; unset, the worker keeps to it.

pub use ngterm_wpe_client::worker::{message, wire};
mod serve;

use std::os::fd::{FromRawFd as _, OwnedFd};

pub(crate) use ngterm_wpe_client::worker::Client;

pub use ngterm_wpe_client::worker::{WORKER_FD_ENV, WORKER_IMAGE_ENV};

/// The variable that names a fault the worker plays (`serve::Fault`), for the conformance suite alone: the app's
/// environment reaches the worker, so a test sets it at the app's launch.
pub const FAULT_ENV: &str = "NGTERM_TEST_WEB_WORKER_FAULT";

/// Serves the engine and ends the process when this process was started as the worker ([`WORKER_FD_ENV`]); returns
/// at once otherwise. `setup` runs first in the worker (the app's logging, say).
///
/// # Safety
///
/// Call it first thing in `main`, while the process has one thread: it changes the process's environment.
pub unsafe fn serve_if_asked(setup: impl FnOnce()) {
    let Some(value) = std::env::var_os(WORKER_FD_ENV) else {
        return;
    };
    let image = std::env::var(WORKER_IMAGE_ENV).ok();
    // SAFETY: the caller guarantees the process has one thread.
    unsafe { std::env::remove_var(WORKER_FD_ENV) };
    // SAFETY: as above.
    unsafe { std::env::remove_var(WORKER_IMAGE_ENV) };
    // the image this process was started from, inherited for the exec alone: nothing WebKit starts gets it
    if let Some(image) = image.and_then(|image| image.parse::<i32>().ok()) {
        // SAFETY: `close` on a descriptor the app named as ours alone; a closed one fails with EBADF.
        unsafe { libc::close(image) };
    }
    let Some(fd) = value.to_str().and_then(|value| value.parse::<i32>().ok()) else {
        eprintln!(
            "ngterm: {WORKER_FD_ENV} is not a descriptor number: {}",
            value.display()
        );
        crate::exit_process(2);
    };
    if !is_socket(fd) {
        eprintln!("ngterm: {WORKER_FD_ENV}={fd} names no socket this process inherited");
        crate::exit_process(2);
    }
    // SAFETY: the descriptor is open (checked), inherited for this process alone, and owned by nothing else here.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    // Retain the shared flock in this process, with no writable handle or inheritance into WebKit children.
    let lock_fd = std::env::var(ngterm_wpe_client::RUNTIME_LOCK_ENV).ok();
    // SAFETY: the caller guarantees single-threaded startup.
    unsafe { std::env::remove_var(ngterm_wpe_client::RUNTIME_LOCK_ENV) };
    let _runtime_lock = match lock_fd.map(|value| value.parse::<i32>().ok().and_then(|fd| retain_runtime_lock(fd).ok()))
    {
        Some(Some(lock)) => Some(lock),
        None => None,
        Some(None) => {
            eprintln!("ngterm: invalid inherited web engine runtime lock");
            crate::exit_process(2);
        }
    };
    setup();
    let fault = std::env::var(FAULT_ENV)
        .ok()
        .and_then(|name| serve::Fault::named(&name));
    let code = serve::serve(socket, fault);
    crate::exit_process(code)
}

/// Adopt the inherited, read-only runtime lock and stop it at the worker's next exec boundary.
fn retain_runtime_lock(fd: i32) -> std::io::Result<OwnedFd> {
    // SAFETY: fcntl inspects the provided descriptor; invalid descriptors fail with EBADF.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY {
        return Err(std::io::Error::other("runtime lock must be open read-only"));
    }
    // SAFETY: changes only this process's descriptor flags, before WebKit or its threads start.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the descriptor was inherited for the worker alone and validated above.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Whether descriptor `fd` is an open socket.
fn is_socket(fd: i32) -> bool {
    // SAFETY: `fstat` only reads the descriptor's metadata into `stat`, which it fully initializes on success.
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: as above; a closed descriptor fails with EBADF.
    let ok = unsafe { libc::fstat(fd, &raw mut stat) } == 0;
    ok && (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::os::fd::AsRawFd as _;
    use std::os::unix::ffi::OsStringExt as _;
    use std::process::{Command, Stdio};

    // A subprocess entrypoint before libtest starts its threads: serve_if_asked changes the environment and
    // must not run in a test thread. The probe marker is private to the child commands below.
    #[used]
    #[unsafe(link_section = ".init_array")]
    static WORKER_ENTRY: extern "C" fn() = worker_entry;

    extern "C" fn worker_entry() {
        if std::env::var_os("WPE_WORKER_ENTRY_PROBE").is_some() {
            #[cfg(coverage)]
            {
                unsafe extern "C" {
                    fn __llvm_profile_initialize_file();
                }
                // SAFETY: initializes the profile filename before this early entrypoint can exit; LLVM's own
                // initializer can run later than ours, whereas an ordinary main runs after both.
                unsafe { __llvm_profile_initialize_file() };
            }
            if std::env::var("WPE_WORKER_ENTRY_PROBE").as_deref() == Ok("prepare") {
                // SAFETY: the ELF initializer runs before main, while this process still has one thread.
                unsafe { crate::launcher::prepare_worker() };
                assert_prepared_environment();
                crate::exit_process(77);
            }
            // SAFETY: the ELF initializer runs before main, while this process still has one thread.
            unsafe {
                super::serve_if_asked(|| {
                    if std::env::var("WPE_WORKER_ENTRY_PROBE").as_deref() == Ok("lock") {
                        assert_runtime_lock();
                        crate::exit_process(77);
                    }
                    eprintln!("worker setup ran");
                });
            };
        }
    }

    fn assert_runtime_lock() {
        let fd: i32 = std::env::var("WPE_EXPECT_LOCK_FD").unwrap().parse().unwrap();
        assert!(std::env::var_os(ngterm_wpe_client::RUNTIME_LOCK_ENV).is_none());
        // SAFETY: only inspect this child's inherited descriptor flags.
        let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_eq!(descriptor_flags & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
        // SAFETY: as above.
        let status_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert_eq!(status_flags & libc::O_ACCMODE, libc::O_RDONLY);
        let lock = std::fs::File::open(std::env::var_os("WPE_LOCK_PATH").unwrap()).unwrap();
        assert!(lock.try_lock().is_err(), "the worker must retain its shared flock");
        let status = Command::new("/bin/sh")
            .args(["-c", &format!("test ! -e /proc/self/fd/{fd}")])
            .status()
            .unwrap();
        assert!(status.success(), "worker children must not inherit the lock descriptor");
    }

    #[test]
    fn runtime_lock_is_read_only_and_stops_at_the_workers_exec_boundary() {
        use std::os::unix::process::CommandExt as _;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".in-use");
        std::fs::File::create_new(&path).unwrap();
        let lock = std::fs::File::open(&path).unwrap();
        lock.lock_shared().unwrap();
        let fd = lock.as_raw_fd();
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let socket_fd = socket.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") {
                command.env_remove(key);
            }
        }
        command
            .env("WPE_WORKER_ENTRY_PROBE", "lock")
            .env("WPE_EXPECT_LOCK_FD", fd.to_string())
            .env("WPE_LOCK_PATH", &path)
            .env(ngterm_wpe_client::RUNTIME_LOCK_ENV, fd.to_string())
            .env(super::WORKER_FD_ENV, socket_fd.to_string());
        let inherit = move || {
            for fd in [fd, socket_fd] {
                // SAFETY: only changes the child's flags on descriptors it inherited.
                if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        };
        // SAFETY: only async-signal-safe fcntl runs between fork and exec.
        unsafe { command.pre_exec(inherit) };
        let mut child = command.spawn().unwrap();
        drop(lock);
        drop(command);
        assert_eq!(child.wait().unwrap().code(), Some(77));
        assert!(std::fs::File::open(path).unwrap().try_lock().is_ok());
    }

    fn assert_prepared_environment() {
        for name in [
            "GIO_EXTRA_MODULES",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_SCANNER",
            "WEBKIT_INJECTED_BUNDLE_PATH",
        ] {
            assert_eq!(
                std::env::var_os(name),
                Some(OsString::from("runtime-sentinel")),
                "{name}"
            );
        }
        assert!(std::env::var_os("LD_LIBRARY_PATH").is_none());
        assert!(std::env::var_os(ngterm_wpe_client::RUNTIME_ENV).is_none());
    }

    fn refused_worker(value: &OsStr) -> std::process::Output {
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") {
                command.env_remove(key);
            }
        }
        command
            .args(["--exact", "worker_entry_must_exit_before_libtest"])
            .env("WPE_WORKER_ENTRY_PROBE", "1")
            .env(super::WORKER_FD_ENV, value)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    #[test]
    fn worker_preparation_removes_the_loaders_path_before_starting_helpers() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("usr/bin/ngterm-web");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        ngterm_host::fakes::copy_executable(&std::env::current_exe().unwrap(), &executable).unwrap();
        let mut command = Command::new(executable);
        for name in [
            "GIO_EXTRA_MODULES",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_SCANNER",
            "WEBKIT_INJECTED_BUNDLE_PATH",
        ] {
            command.env(name, "runtime-sentinel");
        }
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") || key.to_string_lossy().starts_with("AGTERM_") {
                command.env_remove(key);
            }
        }
        command
            .env("WPE_WORKER_ENTRY_PROBE", "prepare")
            .env("LD_LIBRARY_PATH", "/no-such-loader-path");
        for runtime in [false, true] {
            if runtime {
                command.env(ngterm_wpe_client::RUNTIME_ENV, "1");
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(77),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn a_worker_with_a_malformed_descriptor_exits_before_setup() {
        for value in [OsString::from("not-a-number"), OsString::from_vec(vec![0xff])] {
            let output = refused_worker(&value);
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!(
                    "ngterm: NGTERM_WPE_WORKER_FD is not a descriptor number: {}\n",
                    value.display()
                )
            );
        }
    }

    #[test]
    fn a_worker_without_an_inherited_socket_exits_before_setup() {
        for value in ["-1", "0"] {
            let output = refused_worker(OsStr::new(value));
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!("ngterm: NGTERM_WPE_WORKER_FD={value} names no socket this process inherited\n")
            );
        }
    }

    /// The test binary started as a worker playing `fault` (`super::FAULT_ENV`), and the app's end of its socket.
    fn faulty_worker(fault: &str) -> (std::os::fd::OwnedFd, std::process::Child) {
        use std::os::unix::process::CommandExt as _;

        let (app, worker) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let fd = worker.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") {
                command.env_remove(key);
            }
        }
        command
            .args(["--exact", "worker_entry_must_exit_before_libtest"])
            .env("WPE_WORKER_ENTRY_PROBE", "1")
            .env(super::WORKER_FD_ENV, fd.to_string())
            .env(super::FAULT_ENV, fault)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        let inherit = move || {
            // SAFETY: only changes the child's flags on the descriptor it inherits.
            if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        };
        // SAFETY: only async-signal-safe fcntl runs between fork and exec.
        unsafe { command.pre_exec(inherit) };
        let child = command.spawn().unwrap();
        drop(worker);
        (app, child)
    }

    #[test]
    fn a_worker_asked_to_end_before_its_hello_ends_saying_nothing() {
        use std::os::fd::AsFd as _;

        let (app, mut child) = faulty_worker("end-before-hello");
        assert_eq!(child.wait().unwrap().code(), Some(0));
        let said = super::wire::recv(app.as_fd(), super::wire::Limits::FROM_WORKER).unwrap();
        assert!(said.is_none(), "{said:?}");
    }

    #[test]
    fn a_worker_whose_read_times_out_before_its_hello_ends_failed_saying_nothing() {
        use std::os::fd::AsFd as _;

        let (app, mut child) = faulty_worker("timeout-before-hello");
        assert_eq!(child.wait().unwrap().code(), Some(2));
        let said = super::wire::recv(app.as_fd(), super::wire::Limits::FROM_WORKER).unwrap();
        assert!(said.is_none(), "{said:?}");
    }

    #[test]
    fn a_worker_asked_to_stop_reading_after_its_hello_refuses_the_start_and_waits_to_be_killed() {
        use std::os::fd::AsFd as _;

        use super::message::{FromWorker, ToWorker};
        use super::wire::{self, Limits};

        let (app, mut child) = faulty_worker("deaf-after-hello");
        let hello = wire::recv(app.as_fd(), Limits::FROM_WORKER).unwrap().unwrap();
        assert!(matches!(
            super::message::body(&hello).unwrap(),
            FromWorker::Hello { .. }
        ));
        let start = ToWorker::Start(crate::HostConfig {
            frames: crate::FrameMode::Shm,
            schemes: Vec::new(),
        });
        let refused = wire::send(app.as_fd(), &start.packet(None).unwrap()).unwrap_err();
        assert!(
            matches!(&refused, wire::WireError::Io(error) if error.raw_os_error() == Some(libc::EPIPE)),
            "{refused}"
        );
        assert!(child.try_wait().unwrap().is_none(), "the worker ended by itself");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn only_open_sockets_qualify_as_worker_transports() {
        assert!(!super::is_socket(-1));
        let file = tempfile::tempfile().unwrap();
        assert!(!super::is_socket(file.as_raw_fd()));
        let (socket, _) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(super::is_socket(socket.as_raw_fd()));
    }
}
