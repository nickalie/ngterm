//! A request already buffered when the client hangs up still reaches the handler.

use super::*;

use ngterm_host::fakes::FakeMainLoop;

/// macOS refuses socket options with EINVAL once the peer has closed, even with its request still buffered.
#[test]
fn a_request_from_a_peer_that_already_hung_up_is_handled() -> io::Result<()> {
    assert_closed_peer_request_is_handled(b"{\"cmd\":\"version\"}\n")
}

#[test]
fn a_request_without_a_newline_from_a_peer_that_already_hung_up_is_handled() -> io::Result<()> {
    assert_closed_peer_request_is_handled(b"{\"cmd\":\"version\"}")
}

fn assert_closed_peer_request_is_handled(request: &[u8]) -> io::Result<()> {
    let (server, mut client) = crate::endpoint::pair()?;
    client.write_all(request)?;
    drop(client);
    let main_loop = Arc::new(FakeMainLoop::spawn(Vec::<String>::new())?);
    let hop = Hop {
        main_loop: Arc::clone(&main_loop) as Arc<dyn MainLoop<Vec<String>>>,
        handler: Arc::new(|seen, request| {
            seen.push(request.cmd.to_string());
            Response::success().into()
        }),
        deadlines: ConnectionDeadlines::default(),
    };
    serve(server, &hop);
    assert_eq!(main_loop.shutdown(), Some(vec!["version".to_owned()]));
    Ok(())
}

/// A disconnected peer's write fails at the write itself, rather than at the timeout's socket option.
#[test]
fn a_write_to_a_peer_that_already_hung_up_reports_broken_pipe() -> io::Result<()> {
    let (mut server, client) = crate::endpoint::pair()?;
    drop(client);
    let error = write_all_by(&mut server, b"reply\n", &ConnectionDeadlines::default()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    Ok(())
}
