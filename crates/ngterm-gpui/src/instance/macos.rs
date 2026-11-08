//! Owner-only Unix socket, serialized by a file lock. The lock also makes stale socket recovery safe.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Activation, BusRequest, BusRequests, RegisterError, Registration};

pub struct Primary {
    _lock: File,
    socket: PathBuf,
}

impl Drop for Primary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

pub struct Remote(UnixStream);

/// Register this launch or reach the primary.
///
/// # Errors
/// The lock, socket, or serving thread could not be opened.
pub fn register(app_id: &str, state_dir: &Path, requests: BusRequests) -> Result<Registration, RegisterError> {
    let failed = |error| RegisterError::new(app_id, "on its instance socket", error);
    std::fs::create_dir_all(state_dir).map_err(failed)?;
    let stem = format!("instance-{}", ngterm_core::live::fnv1a64(app_id.as_bytes()));
    let socket = state_dir.join(format!("{stem}.sock"));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(state_dir.join(format!("{stem}.lock")))
        .map_err(failed)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            // The owner may still be publishing its listener.
            for _ in 0..100 {
                if let Ok(stream) = UnixStream::connect(&socket) {
                    return Ok(Registration::Remote(Remote(stream)));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return Err(failed(io::Error::other("the primary's instance socket is unavailable")));
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(failed(error)),
    }
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    let listener = UnixListener::bind(&socket).map_err(failed)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(failed)?;
    std::thread::Builder::new()
        .name("ngterm-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut byte = [0];
                if stream.read_exact(&mut byte).is_ok() && byte[0] == 1 {
                    let _ = requests.try_send(BusRequest::Activate(Activation::default()));
                    let _ = stream.write_all(&[1]);
                }
            }
        })
        .map_err(failed)?;
    Ok(Registration::Primary(Primary { _lock: lock, socket }))
}

impl Remote {
    /// Forward activation and wait for acknowledgement.
    ///
    /// # Errors
    /// The primary disconnected or did not acknowledge within the timeout.
    pub fn activate(&self, _: &Activation, _: Option<&Path>) -> io::Result<()> {
        self.0.set_read_timeout(Some(Duration::from_secs(3)))?;
        (&self.0).write_all(&[1])?;
        let mut ack = [0];
        (&self.0).read_exact(&mut ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn another_launch_forwards_and_a_stale_socket_is_recovered() {
        // Short roots also exercise macOS's 104-byte sun_path limit.
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let (send, receive) = async_channel::unbounded();
        let Registration::Primary(primary) = register("com.example.test", root.path(), send.clone()).unwrap() else {
            panic!("primary")
        };
        let Registration::Remote(remote) = register("com.example.test", root.path(), send.clone()).unwrap() else {
            panic!("remote")
        };
        remote.activate(&Activation::default(), None).unwrap();
        assert!(matches!(receive.recv_blocking().unwrap(), BusRequest::Activate(_)));
        let socket = primary.socket.clone();
        assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
        drop(primary);
        let _stale = UnixListener::bind(socket).unwrap();
        assert!(
            registers_as_primary(root.path(), &send),
            "the stale socket is recovered"
        );
    }

    const APP_ID: &str = "com.example.test";

    fn stem() -> String {
        format!("instance-{}", ngterm_core::live::fnv1a64(APP_ID.as_bytes()))
    }

    fn requests() -> (BusRequests, async_channel::Receiver<BusRequest>) {
        async_channel::unbounded()
    }

    /// What `register` in `state` fails with.
    fn refusal(state: &Path) -> String {
        match register(APP_ID, state, requests().0) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("{} registered", state.display()),
        }
    }

    /// The lock file under `state`, taken the way a running primary holds it.
    fn held_lock(state: &Path) -> File {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(state.join(format!("{}.lock", stem())))
            .unwrap();
        lock.try_lock().unwrap();
        lock
    }

    #[test]
    fn a_state_directory_under_a_file_refuses_the_launch() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            refusal(&file.join("state")),
            "failed to register com.example.test on its instance socket: Not a directory (os error 20)"
        );
    }

    #[test]
    fn an_unopenable_lock_refuses_the_launch() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::create_dir(root.path().join(format!("{}.lock", stem()))).unwrap();
        assert_eq!(
            refusal(root.path()),
            "failed to register com.example.test on its instance socket: Is a directory (os error 21)"
        );
    }

    #[test]
    fn a_primary_that_never_publishes_its_socket_refuses_the_launch() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _held = held_lock(root.path());
        assert_eq!(
            refusal(root.path()),
            "failed to register com.example.test on its instance socket: the primary's instance socket is unavailable"
        );
    }

    #[test]
    fn a_launch_waits_for_a_primary_still_publishing_its_socket() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let _held = held_lock(root.path());
        let socket = root.path().join(format!("{}.sock", stem()));
        let publisher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            UnixListener::bind(socket).unwrap()
        });
        let registered = register(APP_ID, root.path(), requests().0).unwrap();
        let _listener = publisher.join().unwrap();
        assert!(matches!(registered, Registration::Remote(_)));
    }

    #[test]
    fn a_socket_path_that_cannot_be_cleared_refuses_the_launch() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::create_dir(root.path().join(format!("{}.sock", stem()))).unwrap();
        assert_eq!(
            refusal(root.path()),
            "failed to register com.example.test on its instance socket: Operation not permitted (os error 1)"
        );
    }

    #[test]
    fn a_socket_path_over_the_platform_limit_refuses_the_launch() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("s".repeat(120));
        let refused = refusal(&state);
        assert!(
            refused.starts_with("failed to register com.example.test on its instance socket: "),
            "{refused}"
        );
    }

    #[test]
    fn a_primary_creates_its_state_directory_and_an_owner_only_lock() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("a/b");
        let Registration::Primary(_primary) = register(APP_ID, &state, requests().0).unwrap() else {
            panic!("primary")
        };
        let lock = std::fs::metadata(state.join(format!("{}.lock", stem()))).unwrap();
        assert_eq!(lock.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_primary_activates_only_on_the_activation_byte() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let (send, receive) = requests();
        let Registration::Primary(primary) = register(APP_ID, root.path(), send).unwrap() else {
            panic!("primary")
        };
        let mut stranger = UnixStream::connect(&primary.socket).unwrap();
        stranger.write_all(&[2]).unwrap();
        let mut answer = Vec::new();
        stranger.read_to_end(&mut answer).unwrap();
        assert_eq!(answer, Vec::<u8>::new(), "another byte is not acknowledged");
        drop(UnixStream::connect(&primary.socket).unwrap());
        let mut launch = UnixStream::connect(&primary.socket).unwrap();
        launch.write_all(&[1]).unwrap();
        let mut ack = [0];
        launch.read_exact(&mut ack).unwrap();
        assert_eq!(ack, [1]);
        assert_eq!(
            receive.try_recv(),
            Ok(BusRequest::Activate(Activation::default())),
            "the forward is the first request"
        );
        assert_eq!(receive.try_recv(), Err(async_channel::TryRecvError::Empty));
    }

    #[test]
    fn a_forward_reaches_the_primary_and_fails_when_it_hangs_up_unacknowledged() {
        let (launch, mut primary) = UnixStream::pair().unwrap();
        let reader = std::thread::spawn(move || {
            let mut byte = [0];
            primary.read_exact(&mut byte).unwrap();
            byte[0]
        });
        let error = Remote(launch).activate(&Activation::default(), None).unwrap_err();
        assert_eq!(reader.join().unwrap(), 1, "the forward's activation byte");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_forward_fails_when_its_connection_cannot_carry_it() {
        let (launch, _primary) = UnixStream::pair().unwrap();
        launch.shutdown(std::net::Shutdown::Write).unwrap();
        let error = Remote(launch).activate(&Activation::default(), None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn a_forward_fails_when_the_primary_is_gone() {
        let (launch, primary) = UnixStream::pair().unwrap();
        drop(primary);
        assert!(Remote(launch).activate(&Activation::default(), None).is_err());
    }

    #[test]
    fn a_forward_gives_up_on_a_silent_primary() {
        let (launch, _primary) = UnixStream::pair().unwrap();
        let started = std::time::Instant::now();
        let error = Remote(launch).activate(&Activation::default(), None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(
            started.elapsed() >= Duration::from_millis(2500),
            "{:?}",
            started.elapsed()
        );
    }

    /// Whether a launch becomes the primary within two seconds. Tests in this binary start shells on PTYs, which
    /// `fork` before they `exec`: until then the child holds a copy of every descriptor, the dropped primary's lock
    /// included, and a launch in that window finds the lock taken and reaches the stale listener as a remote.
    fn registers_as_primary(root: &Path, send: &BusRequests) -> bool {
        (0..100).any(|_| {
            let registered = register("com.example.test", root, send.clone()).unwrap();
            let primary = matches!(registered, Registration::Primary(_));
            if !primary {
                std::thread::sleep(Duration::from_millis(20));
            }
            primary
        })
    }
}
