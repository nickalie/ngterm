//! Pipe failures use private names and joined receivers; process-lifetime primaries live only in a child.

use super::*;

#[test]
fn only_busy_and_missing_pipes_are_retried() {
    for code in [ERROR_PIPE_BUSY, ERROR_FILE_NOT_FOUND] {
        assert!(is_transient(&io::Error::from_raw_os_error(code.cast_signed())));
    }
    for code in [-1, 0, 5, 6, 109] {
        assert!(!is_transient(&io::Error::from_raw_os_error(code)));
    }
    assert!(!is_transient(&io::Error::other("not an OS error")));
}

fn receive_bytes(bytes: &[u8], closed: bool) -> async_channel::Receiver<BusRequest> {
    let dir = tempfile::tempdir().unwrap();
    let path = pipe_path("test.receive", dir.path());
    let listener = bind(&path).unwrap();
    let (requests, received) = async_channel::unbounded();
    if closed {
        received.close();
    }
    let serving = thread::spawn(move || {
        let pipe = listener.accept().unwrap();
        receive(&pipe, &requests);
    });
    let mut client = Pipe::connect_by_path(path.as_str()).unwrap();
    client.write_all(bytes).unwrap();
    drop(client);
    serving.join().unwrap();
    received
}

#[test]
fn an_unreadable_forward_posts_no_activation() {
    let received = receive_bytes(b"\xff\xfe\n", false);
    assert_eq!(received.try_recv(), Err(async_channel::TryRecvError::Closed));
}

#[test]
fn a_valid_forward_after_shutdown_is_dropped_without_blocking() {
    let received = receive_bytes(b"{\"cwd\":null}\n", true);
    assert_eq!(received.try_recv(), Err(async_channel::TryRecvError::Closed));
}

#[test]
fn a_refused_foreground_handoff_still_delivers_the_launch_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = pipe_path("test.foreground", dir.path());
    let listener = bind(&path).unwrap();
    let (release, released) = std::sync::mpsc::channel();
    let serving = thread::spawn(move || {
        let pipe = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&pipe).read_line(&mut line).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
        line
    });
    let remote = Remote {
        pipe: Pipe::connect_by_path(path.as_str()).unwrap(),
        server_pid: 0,
    };
    let delivered = remote.activate(&Activation::default(), Some(dir.path()));
    release.send(()).unwrap();
    let line = serving.join().unwrap();
    delivered.unwrap();
    assert!(line.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap(),
        serde_json::json!({"cwd": dir.path().to_string_lossy()})
    );
}

#[test]
fn a_pipe_whose_security_refuses_this_user_reports_the_registration_error() {
    let dir = tempfile::tempdir().unwrap();
    let id = "test.denied";
    let path = pipe_path(id, dir.path());
    let denied = U16CString::from_str("D:P(D;;GA;;;WD)").unwrap();
    let mut options = PipeListenerOptions::new().path(path.as_str());
    options.security_descriptor = Some(SecurityDescriptor::deserialize(&denied).unwrap());
    let _listener = options.create_duplex::<Bytes>().unwrap();
    assert_eq!(
        Remote::connect(&path)
            .err()
            .expect("the pipe denies every client")
            .raw_os_error(),
        Some(5)
    );
    let (requests, _) = async_channel::unbounded();
    let error = register(id, dir.path(), requests)
        .err()
        .expect("the pipe cannot be reached");
    assert_eq!(
        error.to_string(),
        format!(
            "failed to register {id} on its instance pipe: {}",
            io::Error::from_raw_os_error(5)
        )
    );
}

#[test]
fn an_empty_state_path_still_has_a_stable_pipe_identity() {
    assert_eq!(
        instance_pipe_name("test", Path::new("")),
        "ngterm-instance-cbf29ce484222325-f9e6e6ef197c2b25"
    );
}
