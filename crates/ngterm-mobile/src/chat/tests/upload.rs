//! An upload over the fake transport: the command it runs, the bytes paced by the channel's backlog and ended, the
//! origin's path or words as the answer, a cancel or a stall closing the channel.

use std::cell::RefCell;
use std::future;
use std::time::Duration;

use crate::api::{Machine, MobileError};
use crate::chat::upload::{CHUNK, STALL, STALLED, WINDOW, upload, upload_tail};
use crate::transport::fake::{FakeConnection, FakeTransport, Opened};
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
        upload_tail("it's a file.png", 42),
        r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl '\''agent'\'' '\''upload'\'' '\''--name'\'' '\''it'\''\'\'''\''s a file.png'\'' '\''--size'\'' '\''42'\'''"#
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
                assert_eq!(opened.command, upload_tail("shot.png", file.len()));
                let (got, ended) = take_all(&mut opened).await;
                assert!(ended);
                opened.peer.data(b"/state/uploads/1/shot.png\n".to_vec()).await;
                opened.peer.end(ChannelEnd::Exit { code: Some(0), signal: None }).await;
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
    assert!(told.windows(2).all(|pair| pair[0].0 < pair[1].0), "progress only grows: {told:?}");
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
                opened.peer.stderr(b"Error: the upload stopped after 1 of 3 bytes\n".to_vec()).await;
                opened.peer.end(ChannelEnd::Exit { code: Some(1), signal: None }).await;
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
