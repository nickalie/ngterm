//! The single instance on Windows (plan > Decisions > Single instance): a named pipe per app id and state directory
//! ([`instance_pipe_name`]), owner-only and closed to remote clients, as the control socket's pipe is
//! (`ngterm-server`'s `endpoint/windows.rs`, sharing [`ngterm_core::pipe_security`]).
//!
//! The launch that creates the pipe's first instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`) is the primary: a thread of its
//! own serves the pipe for the whole run and hands each forward to the main loop as [`BusRequest::Activate`]. A launch
//! that finds the pipe held connects, checks that its server runs as this user, allows the primary to take the
//! foreground (`AllowSetForegroundWindow`: Windows lets a process raise its window only while the foreground's owner
//! allows it, and a launch from Explorer or a console holds that right), writes one line and waits until the primary
//! read it (`FlushFileBuffers`), then exits. All of it happens in `main` before GPUI starts and before any state read.
//!
//! The line is a JSON object, `{"cwd": <the launch's directory or null>}`, which the primary logs; an activation on
//! Windows carries no token.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::windows::io::{AsHandle as _, AsRawHandle as _};
use std::path::Path;
use std::thread;
use std::time::Duration;

use interprocess::os::windows::named_pipe::{DuplexPipeStream, PipeListener, PipeListenerOptions, pipe_mode::Bytes};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

use super::{Activation, BusRequest, BusRequests, RegisterError, Registration};

/// The prefix of the instance pipe's name ([`instance_pipe_name`]).
pub const INSTANCE_PIPE_PREFIX: &str = "ngterm-instance-";

/// Where a failed registration happened, for [`RegisterError`].
const PLACE: &str = "on its instance pipe";

/// The most a forward's line may hold; a longer one is cut there.
const LINE_LIMIT: u64 = 64 * 1024;

/// How long a forwarding launch keeps trying a pipe that is busy or between primaries.
const CONNECT_ATTEMPTS: u32 = 50;
const CONNECT_PAUSE: Duration = Duration::from_millis(20);

type Pipe = DuplexPipeStream<Bytes>;

/// The instance pipe of `app_id` over `state_dir`, without the `\\.\pipe\` prefix: [`ngterm_core::paths::pipe_name`]
/// of [`INSTANCE_PIPE_PREFIX`] and the state directory made absolute, then `-` and the app id's FNV-1a 64 hash, so two
/// ids over one directory and one id over two directories each get their own primary.
#[must_use]
pub fn instance_pipe_name(app_id: &str, state_dir: &Path) -> String {
    let absolute = std::path::absolute(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    format!(
        "{}-{}",
        ngterm_core::paths::pipe_name(INSTANCE_PIPE_PREFIX, &absolute),
        ngterm_core::live::fnv1a64(app_id.as_bytes())
    )
}

fn pipe_path(app_id: &str, state_dir: &Path) -> String {
    format!(r"\\.\pipe\{}", instance_pipe_name(app_id, state_dir))
}

/// The primary's registration: the thread serving the instance pipe, which runs as long as the process.
pub struct Primary {
    _serving: thread::JoinHandle<()>,
}

/// Registers `app_id` over `state_dir`: the primary when this launch creates the instance pipe, else the
/// [`Remote`] that holds it.
///
/// # Errors
///
/// The pipe could be neither created nor reached, its server runs as another user, or the serving thread did not
/// start.
pub fn register(app_id: &str, state_dir: &Path, requests: BusRequests) -> Result<Registration, RegisterError> {
    let path = pipe_path(app_id, state_dir);
    let failed = |source: io::Error| RegisterError::new(app_id, PLACE, source);
    let mut last = None;
    for _ in 0..CONNECT_ATTEMPTS {
        match bind(&path) {
            Ok(listener) => {
                let serving = thread::Builder::new()
                    .name("ngterm-instance".to_owned())
                    .spawn(move || serve(&listener, &requests))
                    .map_err(failed)?;
                return Ok(Registration::Primary(Primary { _serving: serving }));
            }
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
            Err(error) => return Err(failed(error)),
        }
        match Remote::connect(&path) {
            Ok(remote) => return Ok(Registration::Remote(remote)),
            // the primary is between two pipe instances, or it just quit: try again
            Err(error) if is_transient(&error) => last = Some(error),
            Err(error) => return Err(failed(error)),
        }
        thread::sleep(CONNECT_PAUSE);
    }
    Err(failed(last.unwrap_or_else(|| {
        io::Error::other("the instance pipe stayed out of reach")
    })))
}

fn is_transient(error: &io::Error) -> bool {
    let code = error.raw_os_error().and_then(|code| u32::try_from(code).ok());
    matches!(code, Some(ERROR_PIPE_BUSY | ERROR_FILE_NOT_FOUND))
}

/// The pipe's first instance, owner-only; a pipe another process holds fails with `ERROR_ACCESS_DENIED`
/// ([`io::ErrorKind::PermissionDenied`]).
fn bind(path: &str) -> io::Result<PipeListener<Bytes, Bytes>> {
    let sddl = U16CString::from_str(ngterm_core::pipe_security::owner_only_pipe_sddl()?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let mut options = PipeListenerOptions::new().path(path);
    options.security_descriptor = Some(SecurityDescriptor::deserialize(&sddl)?);
    options.create_duplex::<Bytes>()
}

/// Serves forwards until the process ends: each connection's line is read on a thread of its own, so a client that
/// never writes holds up no other launch.
fn serve(listener: &PipeListener<Bytes, Bytes>, requests: &BusRequests) {
    loop {
        let pipe = match listener.accept() {
            Ok(pipe) => pipe,
            Err(error) => {
                tracing::warn!(%error, "instance: the instance pipe failed; later launches run on their own");
                return;
            }
        };
        let requests = requests.clone();
        let spawned = thread::Builder::new()
            .name("ngterm-instance-forward".to_owned())
            .spawn(move || receive(pipe, &requests));
        if let Err(error) = spawned {
            tracing::warn!(%error, "instance: a forward was dropped");
        }
    }
}

/// Reads one forward and hands it to the main loop.
fn receive(pipe: Pipe, requests: &BusRequests) {
    let mut line = String::new();
    if let Err(error) = BufReader::new((&pipe).take(LINE_LIMIT)).read_line(&mut line) {
        tracing::debug!(%error, "instance: an unreadable forward");
        return;
    }
    let cwd = serde_json::from_str::<serde_json::Value>(&line)
        .ok()
        .and_then(|value| value.get("cwd").and_then(serde_json::Value::as_str).map(str::to_owned));
    tracing::info!(cwd, "instance: activation requested over the instance pipe");
    if requests.try_send(BusRequest::Activate(Activation::default())).is_err() {
        tracing::debug!("instance: activation dropped, the app is gone");
    }
}

/// The primary another process runs, as a forwarding launch reaches it.
pub struct Remote {
    pipe: Pipe,
    server_pid: u32,
}

impl Remote {
    fn connect(path: &str) -> io::Result<Self> {
        let pipe = Pipe::connect_by_path(path)?;
        let server_pid = pipe.server_process_id()?;
        if !ngterm_core::pipe_security::runs_as_current_user(server_pid)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the pipe at {path} belongs to a process of another user (pid {server_pid})"),
            ));
        }
        Ok(Self { pipe, server_pid })
    }

    /// Lets the primary take the foreground, then sends this launch's forward and waits until the primary has read it.
    /// The activation carries nothing on Windows (see the module docs).
    ///
    /// # Errors
    ///
    /// The forward could not be written.
    pub fn activate(&self, activation: &Activation, cwd: Option<&Path>) -> io::Result<()> {
        let _ = activation;
        // SAFETY: a plain call on a process id; a refusal (this launch holds no foreground right) only leaves the
        // primary to flash instead of rising.
        if unsafe { AllowSetForegroundWindow(self.server_pid) } == 0 {
            tracing::info!(
                error = %io::Error::last_os_error(),
                "instance: the primary may not take the foreground"
            );
        }
        let cwd = cwd.map(|cwd| cwd.to_string_lossy().into_owned());
        let mut line = serde_json::json!({ "cwd": cwd }).to_string();
        line.push('\n');
        (&self.pipe).write_all(line.as_bytes())?;
        // SAFETY: the open pipe end `self.pipe` owns, borrowed for the call.
        if unsafe { FlushFileBuffers(self.pipe.as_handle().as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_is_the_app_ids_over_the_state_dirs() {
        let name = instance_pipe_name("com.nickalie.ngterm", Path::new(r"C:\Users\u\AppData\Local\ngterm"));
        assert!(name.starts_with(INSTANCE_PIPE_PREFIX), "{name}");
        assert_eq!(
            name,
            instance_pipe_name("com.nickalie.ngterm", Path::new(r"c:/users/u/appdata/local/ngterm/")),
            "one directory, one pipe, however it is spelled"
        );
        assert_ne!(
            name,
            instance_pipe_name("com.nickalie.ngterm.dev", Path::new(r"C:\Users\u\AppData\Local\ngterm"))
        );
        assert_ne!(
            name,
            instance_pipe_name("com.nickalie.ngterm", Path::new(r"C:\Users\u\other"))
        );
    }

    #[test]
    fn a_second_launch_forwards_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let app_id = "com.nickalie.ngterm.test.forward";
        let (requests, received) = async_channel::unbounded();
        let Ok(Registration::Primary(_primary)) = register(app_id, dir.path(), requests) else {
            panic!("the first launch is the primary");
        };
        let (other, _unused) = async_channel::unbounded();
        let Ok(Registration::Remote(remote)) = register(app_id, dir.path(), other) else {
            panic!("the second launch forwards");
        };
        remote.activate(&Activation::default(), Some(dir.path())).unwrap();
        let request = received.recv_blocking().unwrap();
        assert_eq!(request, BusRequest::Activate(Activation::default()));
        let (third, _unused) = async_channel::unbounded();
        assert!(
            matches!(
                register("com.nickalie.ngterm.test.other", dir.path(), third),
                Ok(Registration::Primary(_))
            ),
            "another id over the same directory is a primary of its own"
        );
    }
}
