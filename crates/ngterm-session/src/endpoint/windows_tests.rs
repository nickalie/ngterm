//! The pipe endpoint's refusals and its peer probe, on real named pipes of temporary namespaces.

use std::thread;

use super::*;

fn namespace() -> (tempfile::TempDir, Endpoint) {
    let root = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::new(root.path().join("live"));
    (root, endpoint)
}

/// The listener, the client's end and the server's.
fn connected(endpoint: &Endpoint) -> (Listener, Stream, Stream) {
    let listener = endpoint.bind().unwrap();
    let accepting = thread::spawn(move || {
        let accepted = listener.accept().unwrap();
        (listener, accepted)
    });
    let client = endpoint.connect().unwrap();
    let (listener, server) = accepting.join().unwrap();
    (listener, client, server)
}

#[test]
fn taking_over_a_pipe_no_server_holds_is_refused_and_leaves_the_name_free() {
    let (_root, endpoint) = namespace();
    let error = endpoint.bind_beside().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        error.to_string(),
        format!("no ngterm-session server holds {endpoint} to take it over from")
    );
    // the probe's instance is gone with the refusal: a server binds the name, and then a replacement beside it
    let _server = endpoint.bind().unwrap();
    let _beside = endpoint.bind_beside().unwrap();
}

#[test]
fn a_client_whose_server_disconnected_it_reads_as_its_peer_closed() {
    let (_root, endpoint) = namespace();
    let (_listener, mut client, server) = connected(&endpoint);
    assert!(!peer_closed(&mut client).unwrap());
    disconnect(&server);
    assert!(peer_closed(&mut client).unwrap());
}

#[test]
fn a_server_that_disconnected_its_pipe_reads_as_its_peer_closed() {
    let (_root, endpoint) = namespace();
    let (_listener, _client, mut server) = connected(&endpoint);
    assert!(!peer_closed(&mut server).unwrap());
    disconnect(&server);
    assert!(peer_closed(&mut server).unwrap());
}

#[test]
fn a_pipe_end_the_probe_cannot_peek_fails_it() {
    // an anonymous pipe's write end: a pipe, without the read access a peek needs
    let (_reader, writer) = io::pipe().unwrap();
    let pipe = DuplexPipeStream::<Bytes>::try_from(OwnedHandle::from(writer)).unwrap();
    let mut stream = Stream::NamedPipe(pipe.into());
    assert_eq!(
        peer_closed(&mut stream).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn a_pipe_that_denies_this_user_reads_as_an_elevated_server() {
    let (_root, endpoint) = namespace();
    // the namespace's pipe with a descriptor that denies everyone, as an elevated server's refuses a process below
    // its integrity
    let sddl = U16CString::from_str("D:(D;;GA;;;WD)").unwrap();
    let path = pipe_path(&endpoint);
    let mut options = PipeListenerOptions::new().path(path.as_str());
    options.security_descriptor = Some(SecurityDescriptor::deserialize(&sddl).unwrap());
    let _denying = options.create_duplex::<Bytes>().unwrap();
    let expected = format!("the ngterm-session server at {endpoint} runs elevated; connect from an elevated process: ");
    for error in [
        endpoint.connect().unwrap_err(),
        endpoint.connect_timeout(Duration::from_secs(5)).unwrap_err(),
    ] {
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().starts_with(&expected), "{error}");
    }
}

#[test]
fn only_a_denied_first_instance_or_connection_is_reworded() {
    let (_root, endpoint) = namespace();
    // ERROR_PIPE_BUSY
    let busy = name_taken_error(&endpoint, io::Error::from_raw_os_error(231));
    assert_eq!(busy.raw_os_error(), Some(231));
    let missing = connection_error(&endpoint, io::Error::from(io::ErrorKind::NotFound));
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        missing.to_string(),
        io::Error::from(io::ErrorKind::NotFound).to_string()
    );
}
