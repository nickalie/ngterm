---
worth: yes
where: crates/ngterm-server/src/endpoint/windows.rs:262
added: 2026-11-13
---
# A Windows control client gone before its accept is dropped, with its request

A client that connects to the control pipe, writes its request and closes before the accept thread reaches
`ConnectNamedPipe` loses the request: interprocess's `PipeListener::accept` (2.4.6, `block_on_connect_clearing_empty_conns`
in `named_pipe/listener.rs`) answers `ERROR_NO_DATA` by disconnecting the instance and waiting for the next client.
Upstream's Unix socket keeps such a client's bytes for the accept and runs the request. The window is the time between
one accept returning (its fresh instance is already connectable) and the next `ConnectNamedPipe`, so a fire-and-forget
client (a hook that writes and exits) can lose its request on a loaded machine. ngtermctl and agents wait for their
answer and are not affected. `test_a_request_runs_when_its_client_hangs_up_before_the_answer` (protocol) reaches this
on Windows by timing alone; it is not in `conformance/rust-passing-windows.txt`, so it does not gate there.

Found in task #32's CI-fix review (round 1, docs/plans/20261112-e2e-coverage-90.md). Not fixed there: the fix replaces
interprocess's pipe listener on Windows, and a first attempt (2026-11-13, on the win11 VM) showed it is not small:

- A unit test that binds, lets a client write `request\n` and close (its stream marked flushed, so the handle closes at
  once), then accepts, fails on the current code: the accept never returns.
- After `ConnectNamedPipe` returns `ERROR_NO_DATA`, `PeekNamedPipe` reports the client's 8 bytes, but `ReadFile` on the
  instance stayed pending. Peeking the bytes at the accept and serving them from the connection made that test pass.
- An own listener (instances from `CreateNamedPipeW` as ngterm-session's `create_instance` makes them, the instance
  wrapped by `DuplexPipeStream::try_from(OwnedHandle)`) broke every test in `crates/ngterm-server/tests/server.rs`
  (the client's write or read failed with `ERROR_NO_DATA`, 232), with an overlapped `ConnectNamedPipe` and with a
  blocking one alike. The difference from interprocess's own accept (which hands the instance out through
  `RawPipeStream::new_server`) was not found.

A fix needs that difference found first, then the unit test above and the server tests green on Windows CI.
