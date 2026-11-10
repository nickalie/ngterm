//! An upload over the fake transport: the command it runs, the bytes paced by the channel's backlog and ended, the
//! origin's path or words as the answer, a cancel or a stall closing the channel.

use std::cell::RefCell;
use std::future;
use std::time::Duration;

use crate::api::{Machine, MobileError};
use crate::chat::upload::{CHUNK, STALL, STALLED, UploadCancel, WINDOW, upload, upload_tail};
use crate::transport::fake::{FakeConnection, FakeTransport, Opened, Script};
use crate::transport::{ChannelEnd, ChannelInput, Transport};

fn machine() -> Machine {
    Machine {
        id: "m1".to_owned(),
        label: "box".to_owned(),
        host: "box".to_owned(),
        port: 22,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
    }
}

async fn connected() -> (FakeTransport, FakeConnection) {
    let transport = FakeTransport::new();
    let connection = transport.connect(&machine()).await.expect("connects");
    (transport, connection)
}

/// The one channel the upload opened, once it has.
async fn opened(transport: &FakeTransport) -> Opened {
    loop {
        if let Some(opened) = transport.take_opened().pop() {
            return opened;
        }
        tokio::task::yield_now().await;
    }
}

/// Reads the core's input until its end: the bytes, and whether the end came.
async fn take_all(opened: &mut Opened) -> (Vec<u8>, bool) {
    let mut bytes = Vec::new();
    while let Some(input) = opened.peer.input().await {
        match input {
            ChannelInput::Data(data) => bytes.extend_from_slice(&data),
            ChannelInput::Eof => return (bytes, true),
            ChannelInput::Close => return (bytes, false),
            ChannelInput::Resize(_) => {}
        }
    }
    (bytes, false)
}

/// `file` sent as `big.bin`, its progress unheard, never cancelled.
async fn send_quietly(connection: FakeConnection, file: Vec<u8>) -> Result<String, MobileError> {
    upload(&connection, "big.bin", &file, &|_, _| {}, future::pending()).await
}

#[test]
fn the_command_quotes_the_name_for_the_remote_shell() {
    // each word single-quoted for the inner shell, the whole line again for `sh -c`: the name's own quote survives
    assert_eq!(
        upload_tail(ngterm_core::remote::origin::OriginShell::Posix, "it's a file.png", 42),
        r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl '\''agent'\'' '\''upload'\'' '\''--name=it'\''\'\'''\''s a file.png'\'' '\''--size'\'' '\''42'\'''"#
    );
}

#[cfg(unix)]
#[test]
fn a_name_that_starts_with_a_dash_stays_the_names_value() {
    // `--name -notes.txt` is an unknown option to the far parser; `--name=-notes.txt` is the name
    assert_eq!(
        super::far_words(&upload_tail(
            ngterm_core::remote::origin::OriginShell::Posix,
            "-notes.txt",
            7
        )),
        "[agent][upload][--name=-notes.txt][--size][7]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn the_bytes_go_whole_then_the_end_and_the_printed_path_is_the_answer() {
    let (transport, connection) = connected().await;
    let file: Vec<u8> = (0..=255_u8).cycle().take(WINDOW * 3 + 17).collect();
    let told = RefCell::new(Vec::new());
    let progress = |sent, total| told.borrow_mut().push((sent, total));
    let local = tokio::task::LocalSet::new();
    let answer = local
        .run_until(async {
            let sending = upload(&connection, "shot.png", &file, &progress, future::pending());
            let origin = async {
                let mut opened = opened(&transport).await;
                assert_eq!(
                    opened.command,
                    upload_tail(ngterm_core::remote::origin::OriginShell::Posix, "shot.png", file.len())
                );
                let (got, ended) = take_all(&mut opened).await;
                assert!(ended);
                opened.peer.data(b"/state/uploads/1/shot.png\n".to_vec()).await;
                opened
                    .peer
                    .end(ChannelEnd::Exit {
                        code: Some(0),
                        signal: None,
                    })
                    .await;
                got
            };
            let (answer, got) = tokio::join!(sending, origin);
            assert_eq!(got, file);
            answer
        })
        .await;
    assert_eq!(answer, Ok("/state/uploads/1/shot.png".to_owned()));
    let told = told.into_inner();
    let total = file.len() as u64;
    assert_eq!(told.last(), Some(&(total, total)));
    assert!(
        told.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "progress only grows: {told:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_shut_window_holds_the_sender_at_its_window() {
    let (transport, connection) = connected().await;
    let file = vec![7_u8; WINDOW * 4];
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let sending = tokio::task::spawn_local(send_quietly(connection, file.clone()));
            let mut opened = opened(&transport).await;
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            // nothing taken yet: no more than the window (and the chunk that crossed it) is queued
            let mut queued = 0;
            while let Some(ChannelInput::Data(data)) = opened.peer.try_input() {
                queued += data.len();
            }
            assert!(queued <= WINDOW + CHUNK, "{queued} bytes queued");
            sending.abort();
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn the_origins_words_are_the_error_when_it_stores_nothing() {
    let (transport, connection) = connected().await;
    let local = tokio::task::LocalSet::new();
    let answer = local
        .run_until(async {
            let sending = upload(&connection, "a.txt", b"abc", &|_, _| {}, future::pending());
            let origin = async {
                let mut opened = opened(&transport).await;
                let _ = take_all(&mut opened).await;
                opened
                    .peer
                    .stderr(b"Error: the upload stopped after 1 of 3 bytes\n".to_vec())
                    .await;
                opened
                    .peer
                    .end(ChannelEnd::Exit {
                        code: Some(1),
                        signal: None,
                    })
                    .await;
            };
            tokio::join!(sending, origin).0
        })
        .await;
    assert_eq!(
        answer,
        Err(MobileError::Remote {
            reason: "the upload stopped after 1 of 3 bytes".to_owned()
        })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_closes_the_channel_so_the_origin_keeps_nothing() {
    let (transport, connection) = connected().await;
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let local = tokio::task::LocalSet::new();
    let answer = local
        .run_until(async {
            let file = vec![1_u8; WINDOW * 2];
            let sending = upload(&connection, "a.bin", &file, &|_, _| {}, async {
                let _ = cancelled.await;
            });
            let origin = async {
                let mut opened = opened(&transport).await;
                let _ = cancel.send(());
                let (_, ended) = take_all(&mut opened).await;
                assert!(!ended, "a cancelled upload sends no end of input");
            };
            tokio::join!(sending, origin).0
        })
        .await;
    assert_eq!(answer, Err(MobileError::Cancelled));
}

#[tokio::test(flavor = "current_thread")]
async fn a_cancel_token_stops_the_upload_as_a_dropped_call_does() {
    let (transport, connection) = connected().await;
    let token = UploadCancel::new();
    let local = tokio::task::LocalSet::new();
    let answer = local
        .run_until(async {
            let file = vec![1_u8; WINDOW * 2];
            let sending = upload(&connection, "a.bin", &file, &|_, _| {}, token.cancelled());
            let origin = async {
                let mut opened = opened(&transport).await;
                token.cancel();
                // a second cancel changes nothing
                token.cancel();
                let (_, ended) = take_all(&mut opened).await;
                assert!(!ended, "a cancelled upload sends no end of input");
            };
            tokio::join!(sending, origin).0
        })
        .await;
    assert_eq!(answer, Err(MobileError::Cancelled));
    // a token cancelled before the upload starts stops it before it opens anything
    let early = UploadCancel::new();
    early.cancel();
    let answer = upload(&connection, "b.bin", &[1, 2, 3], &|_, _| {}, early.cancelled()).await;
    assert_eq!(answer, Err(MobileError::Cancelled));
}

/// The point the app stops offering Cancel: progress tells every byte sent only once the input's end is queued
/// behind them, so the origin has the whole file and its end, and a cancel then can't take it back.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cancel_once_every_byte_has_gone_is_too_late_to_take_the_file_back() {
    let (transport, connection) = connected().await;
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let cancel = RefCell::new(Some(cancel));
    let file = vec![3_u8; CHUNK + 5];
    let total = file.len() as u64;
    let progress = |sent, all| {
        if sent == all
            && let Some(cancel) = cancel.borrow_mut().take()
        {
            let _ = cancel.send(());
        }
    };
    let local = tokio::task::LocalSet::new();
    let (answer, (got, ended, _open)) = local
        .run_until(async {
            let sending = upload(&connection, "a.bin", &file, &progress, async {
                let _ = cancelled.await;
            });
            let origin = async {
                let mut opened = opened(&transport).await;
                let (got, ended) = take_all(&mut opened).await;
                // the origin is still storing it: its channel stays open until the cancel
                (got, ended, opened)
            };
            tokio::join!(sending, origin)
        })
        .await;
    assert_eq!(answer, Err(MobileError::Cancelled));
    assert!(cancel.borrow().is_none(), "the progress told all {total} bytes sent");
    assert_eq!(
        (got.len() as u64, ended),
        (total, true),
        "the origin read the whole file and its end"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_link_that_moves_nothing_fails_the_upload() {
    let (transport, connection) = connected().await;
    let local = tokio::task::LocalSet::new();
    let answer = local
        .run_until(async {
            let file = vec![1_u8; 10];
            let sending = upload(&connection, "a.bin", &file, &|_, _| {}, future::pending());
            let origin = async {
                // the origin takes nothing and says nothing
                let opened = opened(&transport).await;
                tokio::time::sleep(STALL + Duration::from_secs(1)).await;
                opened
            };
            tokio::join!(sending, origin).0
        })
        .await;
    assert_eq!(
        answer,
        Err(MobileError::Remote {
            reason: STALLED.to_owned()
        })
    );
}

/// What an upload of three bytes as `a.txt` answers when the origin answers it with `script`.
async fn answered(script: Script) -> Result<String, MobileError> {
    let (transport, connection) = connected().await;
    transport.script(
        &upload_tail(ngterm_core::remote::origin::OriginShell::Posix, "a.txt", 3),
        script,
    );
    upload(&connection, "a.txt", b"abc", &|_, _| {}, future::pending()).await
}

fn remote(reason: &str) -> Result<String, MobileError> {
    Err(MobileError::Remote {
        reason: reason.to_owned(),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn an_origin_that_ends_without_its_words_is_answered_with_how_it_ended() {
    assert_eq!(
        answered(Script::ok("\r\n")).await,
        remote("the machine stored the file but named no path"),
        "a success that prints no path"
    );
    assert_eq!(answered(Script::failing(3, "  \n")).await, remote("exit 3"));
    let signalled = |signal: Option<&str>| Script {
        stdout: Vec::new(),
        stderr: Vec::new(),
        end: ChannelEnd::Exit {
            code: None,
            signal: signal.map(str::to_owned),
        },
    };
    assert_eq!(answered(signalled(Some("KILL"))).await, remote("ended by KILL"));
    assert_eq!(answered(signalled(None)).await, remote("ended by a signal"));
    let lost = Script {
        stdout: Vec::new(),
        stderr: Vec::new(),
        end: ChannelEnd::Lost("the connection was reset".to_owned()),
    };
    assert_eq!(answered(lost).await, remote("the connection was reset"));
}

#[tokio::test(flavor = "current_thread")]
async fn the_origins_words_win_over_how_it_ended() {
    let mut script = Script::failing(0, "Error: the file is over the 32 MiB cap\n");
    script.end = ChannelEnd::Lost("reset".to_owned());
    assert_eq!(answered(script).await, remote("the file is over the 32 MiB cap"));
}

#[tokio::test(flavor = "current_thread")]
async fn an_upload_on_a_closed_connection_fails_with_the_transports_words() {
    let (transport, connection) = connected().await;
    transport.drop_connections("m1");
    let answer = upload(&connection, "a.txt", b"abc", &|_, _| {}, future::pending()).await;
    assert_eq!(answer, remote("the connection is closed"));
    assert!(transport.take_opened().is_empty(), "nothing opened");
}
