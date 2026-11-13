//! The pipe's listener and ends, as clients that close early meet them.

use std::sync::mpsc;
use std::thread;

use super::*;

/// How long an accept that should return at once may take before the test calls it lost.
const ACCEPT_DEADLINE: Duration = Duration::from_secs(10);

fn socket_in(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("ngterm.sock")
}

/// A client that connects, writes `bytes` and closes its handle at once, as a process that exits does, before the
/// server's accept.
fn hang_up_after(socket: &Path, bytes: &[u8]) -> io::Result<()> {
    let pipe = DuplexPipeStream::<Bytes>::connect_by_path(pipe_path(socket).as_str())?;
    (&pipe).write_all(bytes)?;
    // nothing to linger on: the handle closes now, not once the server has read
    pipe.assume_flushed();
    Ok(())
}

/// Accepts on a thread of its own, so an accept that never returns fails the test instead of hanging it.
fn accept_within_deadline(listener: Listener) -> io::Result<(Connection, Listener)> {
    let (sent, accepted) = mpsc::channel();
    thread::spawn(move || {
        let connection = accept(&listener);
        sent.send(connection.map(|connection| (connection, listener))).ok();
    });
    accepted
        .recv_timeout(ACCEPT_DEADLINE)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the accept never returned"))?
}

fn read_all(connection: &Connection) -> io::Result<Vec<u8>> {
    let mut read = Vec::new();
    let mut reader = connection;
    reader.read_to_end(&mut read)?;
    Ok(read)
}

#[test]
fn a_client_gone_before_its_accept_is_accepted_with_its_request() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let socket = socket_in(&dir);
    let listener = bind(&socket)?;
    hang_up_after(&socket, b"request\n")?;
    let (gone, listener) = accept_within_deadline(listener)?;
    assert_eq!(read_all(&gone)?, b"request\n");
    let mut answer = &gone;
    assert_eq!(
        answer.write(b"answer\n").map_err(|error| error.kind()),
        Err(io::ErrorKind::BrokenPipe),
        "an answer to a client that is gone"
    );

    // the listener serves on
    let mut next = connect_unchecked(&socket)?;
    let (served, _listener) = accept_within_deadline(listener)?;
    next.write_all(b"next\n")?;
    let mut line = [0_u8; 5];
    (&served).read_exact(&mut line)?;
    assert_eq!(&line, b"next\n");
    Ok(())
}

#[test]
fn a_client_gone_before_its_accept_without_a_word_reads_as_the_end() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let socket = socket_in(&dir);
    let listener = bind(&socket)?;
    hang_up_after(&socket, b"")?;
    let (gone, _listener) = accept_within_deadline(listener)?;
    assert!(read_all(&gone)?.is_empty());
    Ok(())
}

/// An end closed right after its last write leaves that write for its peer to read, as a socket does.
#[test]
fn a_closed_end_leaves_its_last_write_to_its_peer() -> io::Result<()> {
    for server_writes in [true, false] {
        let (server, client) = pair()?;
        let (mut writer, reader) = if server_writes {
            (server, client)
        } else {
            (client, server)
        };
        let answer = vec![b'a'; 96 * 1024];
        let written = thread::spawn(move || writer.write_all(&answer).map(|()| answer));
        thread::sleep(Duration::from_millis(100));
        let read = read_all(&reader)?;
        let answer = written.join().map_err(|_| io::Error::other("the writer panicked"))??;
        assert_eq!(read.len(), answer.len(), "server writes: {server_writes}");
    }
    Ok(())
}

/// The listener hands out server ends: a shutdown disconnects the client, which reads the end at once.
#[test]
fn an_accepted_end_is_the_servers() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let socket = socket_in(&dir);
    let listener = bind(&socket)?;
    let client = connect_unchecked(&socket)?;
    let (server, _listener) = accept_within_deadline(listener)?;
    (&server).write_all(b"unread")?;
    server.shutdown(Shutdown::Both)?;
    assert!(
        read_all(&client)?.is_empty(),
        "a disconnect drops what the client had not read"
    );
    Ok(())
}

/// A read issued on `handle` and left pending, for a test to start before something else happens on the pipe; it
/// owns its buffer, `OVERLAPPED` and event until the read is done.
struct PendingRead {
    handle: HANDLE,
    overlapped: Box<OVERLAPPED>,
    buf: Box<[u8; 16]>,
    _event: OwnedHandle,
    done: bool,
}

impl PendingRead {
    fn start(handle: HANDLE) -> io::Result<Self> {
        // SAFETY: a manual-reset event, unnamed and unshared; the handle is checked before it is owned.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateEventW returned a fresh handle that nothing else owns.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        // SAFETY: an all-zero OVERLAPPED is a valid one.
        let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
        overlapped.hEvent = event.as_raw_handle();
        let mut read = Self {
            handle,
            overlapped,
            buf: Box::new([0; 16]),
            _event: event,
            done: true,
        };
        let ptr = read.buf.as_mut_ptr();
        let len = u32::try_from(read.buf.len()).map_err(io::Error::other)?;
        // SAFETY: the buffer and the OVERLAPPED are boxed, so they stay put, and `Drop` waits for the read to end.
        if unsafe { ReadFile(handle, ptr, len, std::ptr::null_mut(), &raw mut *read.overlapped) } != 0 {
            return Err(io::Error::other("the read finished at once"));
        }
        let error = io::Error::last_os_error();
        if !is_os_error(&error, ERROR_IO_PENDING) {
            return Err(error);
        }
        read.done = false;
        Ok(read)
    }

    /// Waits for the read to end.
    fn wait(&mut self) -> io::Result<usize> {
        let mut moved = 0;
        // SAFETY: the OVERLAPPED is the pending read's, alive in `self`; this waits for the read to end.
        let finished = unsafe { GetOverlappedResult(self.handle, &raw const *self.overlapped, &raw mut moved, 1) };
        self.done = true;
        if finished == 0 {
            return Err(io::Error::last_os_error());
        }
        usize::try_from(moved).map_err(io::Error::other)
    }

    /// What the read got, or its error.
    fn finish(mut self) -> io::Result<Vec<u8>> {
        let moved = self.wait()?;
        Ok(self.buf.get(..moved).unwrap_or_default().to_vec())
    }
}

impl Drop for PendingRead {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // SAFETY: the read is pending on the open handle through this OVERLAPPED, which the wait below keeps alive.
        unsafe { CancelIoEx(self.handle, &raw const *self.overlapped) };
        self.wait().ok();
    }
}

/// A write past its timeout ends alone: a read pending on the same end goes on and gets the peer's next bytes.
#[test]
fn a_write_timeout_leaves_the_pending_read_alone() -> io::Result<()> {
    let (server, client) = pair()?;
    let read = PendingRead::start(server.handle())?;
    server.set_write_timeout(Some(Duration::from_millis(200)))?;
    let more_than_the_pipe_holds = vec![b'w'; 256 * 1024];
    let written = (&server).write(&more_than_the_pipe_holds);
    assert_eq!(written.map_err(|error| error.kind()), Err(io::ErrorKind::TimedOut));

    (&client).write_all(b"ping")?;
    assert_eq!(read.finish()?, b"ping");
    Ok(())
}
