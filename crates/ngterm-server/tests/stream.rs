//! [`LineStream`] over connected pairs, and through the control server over its real endpoint (an `AF_UNIX` socket, a
//! named pipe on Windows) named by a path in a temp dir. A pair is a connection the server accepted and handed over,
//! with its client, so the streams run on what the app's do on every OS.

use std::cell::Cell;
use std::error::Error;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ngterm_host::MainLoop;
use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::{Command, Request, Response};
use ngterm_server::{Connection, ControlServer, LineStream, LineStreamLimits, LineStreamOwner, Reply, StreamOwner};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const TIMEOUT: Duration = Duration::from_secs(20);
const VERSION: &[u8] = br#"{"cmd":"version"}"#;
/// The request the test handler answers with a stream.
const OPEN: &[u8] = br#"{"cmd":"tree"}"#;
const LIMITS: LineStreamLimits = LineStreamLimits {
    max_line_bytes: 1024,
    max_pending_lines: 16,
    write_timeout: Duration::from_secs(5),
};

/// What an echoing stream reports.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    /// The owner took the connection, with these bytes read past the request line.
    Adopted(Vec<u8>),
    /// `on_close` ran.
    Closed,
}

/// Runs `stream` over `socket`, answering every line with `echo:<line>` and reporting to `events`.
fn echo(stream: &LineStream, socket: Connection, pending: &[u8], events: &Sender<Event>) {
    events.send(Event::Adopted(pending.to_vec())).ok();
    let replies = stream.clone();
    let on_line = |line: &[u8]| {
        let mut reply = b"echo:".to_vec();
        reply.extend_from_slice(line);
        if !replies.send(reply) {
            replies.shutdown();
        }
    };
    stream.run(socket, pending, on_line, || {
        events.send(Event::Closed).ok();
    });
}

/// The only holder of its stream's event sender, so the events channel disconnects once the owner is done.
struct EchoOwner {
    stream: LineStream,
    events: Sender<Event>,
}

impl StreamOwner for EchoOwner {
    fn adopt(self: Box<Self>, socket: Connection, pending: Vec<u8>) {
        echo(&self.stream, socket, &pending, &self.events);
    }
}

/// One stream the server opened, as the app would register it.
struct Opened {
    stream: LineStream,
    events: Receiver<Event>,
}

impl Opened {
    fn next(&self) -> TestResult<Event> {
        Ok(self.events.recv_timeout(TIMEOUT)?)
    }

    /// `on_close` ran, and nothing follows it: the owner has let go of the only sender.
    fn assert_closed_once(&self) -> TestResult {
        assert_eq!(self.next()?, Event::Closed);
        assert_eq!(self.events.recv_timeout(TIMEOUT), Err(RecvTimeoutError::Disconnected));
        assert!(!self.stream.send(b"late".to_vec()), "a closed stream took a line");
        Ok(())
    }
}

/// Fields drop top to bottom: the server stops before the temp dir holding its socket is deleted.
struct Fixture {
    _server: ControlServer,
    opened: Receiver<Opened>,
    socket: PathBuf,
    _main_loop: Arc<FakeMainLoop<()>>,
    _dir: TempDir,
}

/// A short temp dir: socket paths must stay under 104 bytes whatever `TMPDIR` is (Unix; Windows has no such limit).
fn short_dir() -> io::Result<TempDir> {
    let builder = tempfile::Builder::new().prefix("ngt").clone();
    if cfg!(unix) {
        builder.tempdir_in("/tmp")
    } else {
        builder.tempdir()
    }
}

/// A server answering [`OPEN`] with an echoing [`LineStream`] and everything else with a plain `ok`.
fn start(limits: LineStreamLimits) -> TestResult<Fixture> {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let main_loop = Arc::new(FakeMainLoop::spawn(())?);
    let (opened_tx, opened) = mpsc::channel();
    let handler = move |(): &mut (), request: Request| -> Reply {
        if request.cmd != Command::Tree {
            return Response::success().into();
        }
        let stream = LineStream::new(limits);
        let (events_tx, events) = mpsc::channel();
        opened_tx
            .send(Opened {
                stream: stream.clone(),
                events,
            })
            .ok();
        let owner = EchoOwner {
            stream,
            events: events_tx,
        };
        Reply::streaming(Response::success(), Box::new(owner))
    };
    let mut server = ControlServer::new(&socket);
    server.start(Arc::clone(&main_loop) as Arc<dyn MainLoop<()>>, handler)?;
    Ok(Fixture {
        _server: server,
        opened,
        socket,
        _main_loop: main_loop,
        _dir: dir,
    })
}

/// A plain request on a fresh connection.
fn assert_ordinary_request_answered(fixture: &Fixture) -> TestResult {
    let mut stream = ngterm_server::connect(&fixture.socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.write_all(VERSION)?;
    stream.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    assert_eq!(serde_json::from_str::<Response>(&line)?, Response::success());
    Ok(())
}

/// The client side of a stream connection.
struct Client {
    writer: Connection,
    reader: BufReader<Connection>,
}

impl Client {
    fn new(socket: Connection) -> io::Result<Self> {
        socket.set_read_timeout(Some(TIMEOUT))?;
        Ok(Self {
            reader: BufReader::new(socket.try_clone()?),
            writer: socket,
        })
    }

    /// Sends [`OPEN`] with `first` right behind it in the same write, and takes the `ok`.
    fn open(fixture: &Fixture, first: &[u8]) -> TestResult<(Self, Opened)> {
        let mut client = Self::new(ngterm_server::connect(&fixture.socket)?)?;
        let mut request = OPEN.to_vec();
        request.push(b'\n');
        request.extend_from_slice(first);
        client.writer.write_all(&request)?;
        assert_eq!(serde_json::from_str::<Response>(&client.line()?)?, Response::success());
        let opened = fixture.opened.recv_timeout(TIMEOUT)?;
        Ok((client, opened))
    }

    fn say(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(format!("{text}\n").as_bytes())
    }

    /// The next line, newline included; empty at EOF.
    fn line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        Ok(line)
    }
}

/// Hands the connection it is given to the test.
struct Handoff(Sender<(Connection, Vec<u8>)>);

impl StreamOwner for Handoff {
    fn adopt(self: Box<Self>, socket: Connection, pending: Vec<u8>) {
        self.0.send((socket, pending)).ok();
    }
}

/// A connected pair: the client's end and the end the server accepted, handed over after the `ok` of its request.
/// The server is gone by then; the connection outlives it.
fn pair() -> TestResult<(Connection, Connection)> {
    let dir = short_dir()?;
    let socket = dir.path().join("pair.sock");
    let main_loop = Arc::new(FakeMainLoop::spawn(())?);
    let (handed_tx, handed) = mpsc::channel();
    let handed_tx = Mutex::new(handed_tx);
    let mut server = ControlServer::new(&socket);
    server.start(
        Arc::clone(&main_loop) as Arc<dyn MainLoop<()>>,
        move |(), _: Request| {
            let Ok(tx) = handed_tx.lock().map(|tx| tx.clone()) else {
                return Response::failure("poisoned").into();
            };
            Reply::streaming(Response::success(), Box::new(Handoff(tx)))
        },
    )?;
    let mut client = ngterm_server::connect(&socket)?;
    client.set_read_timeout(Some(TIMEOUT))?;
    client.write_all(VERSION)?;
    client.write_all(b"\n")?;
    // byte by byte, so nothing past the response is taken from the peer
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while line.last() != Some(&b'\n') {
        client.read_exact(&mut byte)?;
        line.extend_from_slice(&byte);
    }
    assert_eq!(serde_json::from_slice::<Response>(&line)?, Response::success());
    let (ours, pending) = handed.recv_timeout(TIMEOUT)?;
    assert_eq!(pending, b"");
    client.set_read_timeout(None)?;
    server.stop();
    Ok((client, ours))
}

/// Runs an echoing `stream` over one end of a pair on a thread of its own; the other end is the peer.
fn run_on_pair(stream: &LineStream) -> TestResult<(Client, Opened, JoinHandle<()>)> {
    let (peer, ours) = pair()?;
    // The client's timeout goes on before the runner starts: a stream shut down beforehand closes `ours` at once, and
    // macOS refuses a socket option on a socket whose peer has shut down (`EINVAL`), where Linux takes it.
    let client = Client::new(peer)?;
    let (events_tx, events) = mpsc::channel();
    let runner = {
        let stream = stream.clone();
        thread::spawn(move || echo(&stream, ours, &[], &events_tx))
    };
    let opened = Opened {
        stream: stream.clone(),
        events,
    };
    assert_eq!(opened.next()?, Event::Adopted(Vec::new()));
    Ok((client, opened, runner))
}

#[test]
fn a_stream_survives_other_requests_and_closes_on_client_hang_up() -> TestResult {
    let fixture = start(LIMITS)?;
    let (mut client, opened) = Client::open(&fixture, b"")?;
    assert_eq!(opened.next()?, Event::Adopted(Vec::new()));
    client.say("one")?;
    assert_eq!(client.line()?, "echo:one\n");

    for _ in 0..3 {
        assert_ordinary_request_answered(&fixture)?;
    }
    // the app may speak first as well
    assert!(opened.stream.send(b"pushed".to_vec()));
    assert_eq!(client.line()?, "pushed\n");
    client.say("two")?;
    assert_eq!(client.line()?, "echo:two\n");

    drop(client);
    opened.assert_closed_once()?;
    assert_ordinary_request_answered(&fixture)
}

#[test]
fn bytes_sent_with_the_request_reach_on_line_first() -> TestResult {
    let fixture = start(LIMITS)?;
    let (mut client, opened) = Client::open(&fixture, b"first\nsec")?;
    assert_eq!(opened.next()?, Event::Adopted(b"first\nsec".to_vec()));
    client.writer.write_all(b"ond\n")?;
    assert_eq!(client.line()?, "echo:first\n");
    assert_eq!(client.line()?, "echo:second\n");
    Ok(())
}

#[test]
fn server_side_shutdown_gives_the_client_eof() -> TestResult {
    let fixture = start(LIMITS)?;
    let (mut client, opened) = Client::open(&fixture, b"")?;
    assert_eq!(opened.next()?, Event::Adopted(Vec::new()));
    client.say("ping")?;
    assert_eq!(client.line()?, "echo:ping\n");

    opened.stream.shutdown();
    opened.stream.shutdown();
    assert_eq!(client.line()?, "", "the client did not see EOF");
    opened.assert_closed_once()?;
    assert_ordinary_request_answered(&fixture)
}

#[test]
fn lines_queued_before_run_are_bounded_and_written_once_it_runs() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        max_pending_lines: 2,
        ..LIMITS
    });
    assert!(stream.send(b"a".to_vec()));
    assert!(stream.send(b"b".to_vec()));
    assert!(!stream.send(b"c".to_vec()), "a full queue took a line");

    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    assert_eq!(peer.line()?, "a\n");
    assert_eq!(peer.line()?, "b\n");
    // drained, it takes lines again
    assert!(stream.send(b"d".to_vec()));
    assert_eq!(peer.line()?, "d\n");

    stream.shutdown();
    assert_eq!(peer.line()?, "");
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

/// Upstream's `ControlStreamOwner.sendWaiting`: a producer off the main loop waits for room instead of being refused,
/// and the writer draining the queue wakes it.
#[test]
fn a_waiting_send_waits_for_room_and_the_writer_makes_it() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        max_pending_lines: 1,
        ..LIMITS
    });
    assert!(stream.send(b"a".to_vec()));
    let producer = stream.clone();
    let (sent_tx, sent) = mpsc::channel();
    let waiting = thread::spawn(move || {
        for line in [b"b".to_vec(), b"c".to_vec()] {
            sent_tx.send(producer.send_waiting(line)).ok();
        }
    });
    assert_eq!(
        sent.recv_timeout(Duration::from_millis(200)),
        Err(RecvTimeoutError::Timeout),
        "a full queue took a line"
    );
    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    for line in ["a\n", "b\n", "c\n"] {
        assert_eq!(peer.line()?, line);
    }
    assert!(sent.recv_timeout(TIMEOUT)?);
    assert!(sent.recv_timeout(TIMEOUT)?);
    waiting.join().map_err(|_| "producer panicked")?;
    stream.shutdown();
    assert_eq!(peer.line()?, "");
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

#[test]
fn a_waiting_send_returns_false_once_the_stream_closes() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        max_pending_lines: 1,
        ..LIMITS
    });
    assert!(stream.send(b"a".to_vec()));
    let producer = stream.clone();
    let waiting = thread::spawn(move || producer.send_waiting(b"b".to_vec()));
    thread::sleep(Duration::from_millis(100));
    stream.shutdown();
    assert!(!waiting.join().map_err(|_| "producer panicked")?);
    assert!(!stream.send_waiting(b"late".to_vec()));
    Ok(())
}

#[test]
fn shutdown_before_run_closes_at_once() -> TestResult {
    let stream = LineStream::new(LIMITS);
    assert!(stream.send(b"never written".to_vec()));
    stream.shutdown();
    assert!(!stream.send(b"refused".to_vec()));

    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    let mut rest = Vec::new();
    peer.reader.read_to_end(&mut rest)?;
    assert_eq!(rest, b"");
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

#[test]
fn a_peer_that_never_reads_is_dropped_at_the_write_timeout() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        write_timeout: Duration::from_millis(100),
        ..LIMITS
    });
    let (peer, opened, runner) = run_on_pair(&stream)?;
    let started = Instant::now();
    // far more than an AF_UNIX socket or the control pipe buffers, so the writer blocks on a peer that takes nothing
    let big = vec![b'x'; 4 << 20];
    assert!(stream.send(big.clone()));
    assert!(stream.send(big));
    opened.assert_closed_once()?;
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    runner.join().map_err(|_| "runner panicked")?;
    drop(peer);
    Ok(())
}

/// A peer that hangs up with a line it never read: Linux resets the connection (the read fails with `ECONNRESET`, where
/// a plain hang-up reads an end), another Unix may read an end. Either way the stream closes once. The line is in the
/// peer's buffer before it hangs up, so which of the two the read meets depends on the OS alone, not on timing.
#[cfg(unix)]
#[test]
fn a_peer_hanging_up_on_an_unread_line_closes_the_stream_once() -> TestResult {
    use std::os::fd::AsRawFd as _;

    let stream = LineStream::new(LIMITS);
    let (peer, opened, runner) = run_on_pair(&stream)?;
    assert!(stream.send(b"never read".to_vec()));
    let mut ready = libc::pollfd {
        fd: peer.writer.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = i32::try_from(TIMEOUT.as_millis())?;
    // SAFETY: one pollfd for a descriptor the peer holds open for the call
    let polled = unsafe { libc::poll(&raw mut ready, 1, timeout) };
    assert_eq!(polled, 1, "the line never reached the peer");
    drop(peer);
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

#[test]
fn lines_split_across_reads_are_joined() -> TestResult {
    let stream = LineStream::new(LIMITS);
    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    for part in [&b"ab"[..], b"c\nd", b"\n\n"] {
        peer.writer.write_all(part)?;
    }
    assert_eq!(peer.line()?, "echo:abc\n");
    assert_eq!(peer.line()?, "echo:d\n");
    assert_eq!(peer.line()?, "echo:\n");
    drop(peer);
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

#[test]
fn an_over_long_line_closes_the_stream() -> TestResult {
    let limits = LineStreamLimits {
        max_line_bytes: 8,
        ..LIMITS
    };
    // a finished line over the limit, and an unfinished one already past it
    for over in [&b"123456789\n"[..], b"123456789"] {
        let stream = LineStream::new(limits);
        let (mut peer, opened, runner) = run_on_pair(&stream)?;
        peer.say("12345678")?;
        assert_eq!(peer.line()?, "echo:12345678\n", "a line at the limit is taken");
        peer.writer.write_all(over)?;
        assert_eq!(peer.line()?, "", "{over:?} did not close the stream");
        opened.assert_closed_once()?;
        runner.join().map_err(|_| "runner panicked")?;
    }
    Ok(())
}

/// What a [`LineStreamOwner`]'s callbacks saw.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Line(Vec<u8>),
    Closed,
}

fn recording_owner(stream: &LineStream) -> (LineStreamOwner, Receiver<Seen>) {
    let (lines, seen) = mpsc::channel();
    let closes = lines.clone();
    let owner = LineStreamOwner::new(
        stream.clone(),
        move |line| {
            lines.send(Seen::Line(line)).ok();
        },
        move || {
            closes.send(Seen::Closed).ok();
        },
    );
    (owner, seen)
}

#[test]
fn a_line_stream_owner_hands_lines_and_the_close_to_its_callbacks() -> TestResult {
    let stream = LineStream::new(LIMITS);
    let (owner, seen) = recording_owner(&stream);
    assert!(
        stream.send(b"hello".to_vec()),
        "queued before the connection is adopted"
    );
    let (mut client, server) = pair()?;
    let runner = thread::spawn(move || Box::new(owner).adopt(server, b"first\n".to_vec()));
    let mut reader = BufReader::new(client.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    assert_eq!(line, "hello\n");
    client.write_all(b"second\n")?;
    assert_eq!(seen.recv_timeout(TIMEOUT)?, Seen::Line(b"first".to_vec()));
    assert_eq!(seen.recv_timeout(TIMEOUT)?, Seen::Line(b"second".to_vec()));
    drop(reader);
    drop(client);
    assert_eq!(seen.recv_timeout(TIMEOUT)?, Seen::Closed);
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

#[test]
fn an_abandoned_line_stream_owner_reports_its_close() -> TestResult {
    let stream = LineStream::new(LIMITS);
    let (owner, seen) = recording_owner(&stream);
    Box::new(owner).abandon();
    assert_eq!(seen.recv_timeout(TIMEOUT)?, Seen::Closed);
    assert!(!stream.send(b"late".to_vec()), "the stream is shut");
    Ok(())
}

#[test]
fn a_second_run_drops_its_connection_and_calls_nothing() -> TestResult {
    let stream = LineStream::new(LIMITS);
    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    // adoption is reported before run attaches; an echo proves the first run is serving
    peer.say("attached")?;
    assert_eq!(peer.line()?, "echo:attached\n");
    let (mut second_peer, second) = pair()?;
    let calls = Cell::new(0);
    stream.run(
        second,
        b"never\n",
        |_| calls.set(calls.get() + 1),
        || calls.set(calls.get() + 1),
    );
    assert_eq!(calls.get(), 0, "a second run called back");
    let mut rest = Vec::new();
    second_peer.read_to_end(&mut rest)?;
    assert_eq!(rest, b"", "the second connection was kept");

    // the first run carries on
    peer.say("still here")?;
    assert_eq!(peer.line()?, "echo:still here\n");
    stream.shutdown();
    assert_eq!(peer.line()?, "");
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

/// [`LineStreamLimits::write_timeout`]: a zero one cannot be set on the socket, and the stream closes as soon as it
/// runs, its queued lines unwritten.
#[test]
fn a_zero_write_timeout_closes_the_stream_as_it_runs() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        write_timeout: Duration::ZERO,
        ..LIMITS
    });
    assert!(stream.send(b"never written".to_vec()));
    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    let mut rest = Vec::new();
    peer.reader.read_to_end(&mut rest)?;
    assert_eq!(rest, b"");
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

/// A peer that hangs up with a line it never read still closes the stream, long before the write timeout would.
#[test]
fn a_peer_hanging_up_mid_line_closes_the_stream() -> TestResult {
    let stream = LineStream::new(LineStreamLimits {
        write_timeout: Duration::from_secs(600),
        ..LIMITS
    });
    let (mut peer, opened, runner) = run_on_pair(&stream)?;
    // far more than the connection buffers: the writer blocks with the rest of it queued at the peer
    assert!(stream.send(vec![b'x'; 4 << 20]));
    let mut first = [0_u8; 1];
    peer.writer.read_exact(&mut first)?;
    assert_eq!(&first, b"x");
    drop(peer);
    opened.assert_closed_once()?;
    runner.join().map_err(|_| "runner panicked")?;
    Ok(())
}

/// After a pending line shuts the stream down, input from the next socket read is not delivered.
#[test]
fn socket_input_is_not_delivered_after_a_pending_line_shuts_the_stream() -> TestResult {
    let stream = LineStream::new(LIMITS);
    let (mut peer, ours) = pair()?;
    peer.write_all(b"after\n")?;
    let mut lines = Vec::new();
    let mut closed = false;
    let stopper = stream.clone();
    stream.run(
        ours,
        b"stop\n",
        |line| {
            lines.push(line.to_vec());
            stopper.shutdown();
        },
        || closed = true,
    );
    assert_eq!(lines, vec![b"stop".to_vec()]);
    assert!(closed, "on_close never ran");
    let mut rest = Vec::new();
    peer.read_to_end(&mut rest)?;
    assert_eq!(rest, b"");
    Ok(())
}

#[test]
fn a_settled_reply_is_the_one_its_sender_sent() {
    let ready = Reply::from(Response::failure("ready")).settled();
    assert_eq!(ready.response, Response::failure("ready"));

    let (reply, sender) = Reply::deferred();
    sender.send(Response::failure("sent later"));
    assert_eq!(reply.settled().response, Response::failure("sent later"));

    // answered with another deferred reply, it waits that one out too
    let (outer, first) = Reply::deferred();
    let (inner, second) = Reply::deferred();
    first.send(inner);
    second.send(Response::with_result(serde_json::json!(7)));
    assert_eq!(outer.settled().response, Response::with_result(serde_json::json!(7)));
}

#[test]
fn a_deferred_reply_whose_sender_is_dropped_settles_as_shutting_down() {
    let (reply, sender) = Reply::deferred();
    drop(sender);
    let settled = reply.settled();
    assert_eq!(settled.response, Response::failure("the app is shutting down"));
    assert!(settled.stream.is_none());
}

/// A connection gone before its deferred answer drops the answer: sending it neither blocks nor panics.
#[test]
fn a_deferred_answer_to_a_gone_connection_is_dropped() {
    let (reply, sender) = Reply::deferred();
    drop(reply);
    sender.send(Response::success());
}

#[test]
fn replies_and_line_stream_owners_debug_print_what_they_carry() {
    let plain = format!("{:?}", Reply::from(Response::success()));
    assert!(plain.starts_with("Reply { response: "), "{plain}");
    assert!(plain.ends_with("streams: false, deferred: false }"), "{plain}");
    let deferred = format!("{:?}", Reply::deferred().0);
    assert!(deferred.ends_with("streams: false, deferred: true }"), "{deferred}");

    let stream = LineStream::new(LIMITS);
    let (owner, _seen) = recording_owner(&stream);
    let streaming = format!("{:?}", Reply::streaming(Response::success(), Box::new(owner)));
    assert!(streaming.ends_with("streams: true, deferred: false }"), "{streaming}");

    let (owner, _seen) = recording_owner(&stream);
    let owner = format!("{owner:?}");
    assert!(owner.starts_with("LineStreamOwner { stream: LineStream { "), "{owner}");
    assert!(owner.ends_with(", .. }"), "{owner}");
}
