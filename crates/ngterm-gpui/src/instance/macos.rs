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
