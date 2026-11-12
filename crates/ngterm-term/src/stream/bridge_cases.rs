use super::*;
use crate::{
    ClipboardContent, ClipboardLocation, ClipboardPermissionScope, ClipboardWriteResult, PasteSource, WriteCompletion,
};
use ngterm_core::attach_bridge::{Decoder, Envelope, Message, Part, VERSION};
use ngterm_core::clipboard_wire::{ClipboardStatus, DelegatedReply, DelegatedRequest};

fn envelope(nonce: &str, message: Message) -> Envelope {
    Envelope {
        version: VERSION,
        nonce: nonce.into(),
        message,
    }
}
fn peer() -> Fixture {
    let fixture = stream(PaneConfig {
        attach_bridge: Some("connection".into()),
        ..PaneConfig::new(SIZE)
    });
    fixture.feed.deliver(&envelope("connection", Message::Hello).encode());
    fixture.feed.deliver(b"ready");
    fixture.screen_until("ready");
    fixture
}
fn outgoing(fixture: &Fixture) -> Envelope {
    let mut decoder = Decoder::default();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let Some(Outgoing::Bytes(bytes)) = fixture
            .out
            .next(Some(deadline.saturating_duration_since(Instant::now())))
            .unwrap()
        else {
            panic!("no bridge reply");
        };
        for part in decoder.feed(&bytes) {
            if let Part::Envelope(envelope) = part {
                return envelope;
            }
        }
    }
}
fn read(bridge: Option<String>, mimes: Vec<String>) -> DelegatedRequest {
    DelegatedRequest::ClipboardReadMime {
        bridge,
        selection: "c".into(),
        mimes,
        list: false,
        name: "Evil Program".into(),
        granted: true,
        can_remember: true,
    }
}
#[test]
fn bridge_preserves_authoritative_consent_and_real_binary_native_completion() {
    let fixture = peer();
    let contents = vec![ngterm_core::clipboard_wire::ClipboardRepresentation {
        mime: "image/png".into(),
        data: include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").to_vec(),
    }];
    let request = DelegatedRequest::ClipboardWrite {
        selection: "p".into(),
        contents: contents.clone(),
        name: "Evil Program".into(),
        granted: true,
        can_remember: true,
        kitty: true,
    };
    let bytes = envelope(
        "connection",
        Message::Request {
            id: 9,
            epoch: 8,
            request,
        },
    )
    .encode();
    for chunk in bytes.chunks(7) {
        fixture.feed.deliver(chunk);
    }
    let PaneEvent::ClipboardWrite(write) = fixture.event(|event| matches!(event, PaneEvent::ClipboardWrite(_))) else {
        unreachable!();
    };
    assert!(write.granted && write.can_remember);
    assert_eq!(write.permission_scope, ClipboardPermissionScope::Kitty);
    assert_eq!(write.location, ClipboardLocation::Primary);
    assert_eq!(write.contents[0].data, contents[0].data);
    assert!(fixture.pane.read_text(TextRegion::Viewport).unwrap().contains("ready"));
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
    fixture.pane.answer_clipboard_write(
        write.id,
        WriteCompletion {
            status: ClipboardWriteResult::Denied,
            remember: false,
        },
    );
    assert_eq!(
        outgoing(&fixture),
        envelope(
            "connection",
            Message::Reply {
                id: 9,
                epoch: 8,
                reply: DelegatedReply::ClipboardWrite {
                    status: ClipboardStatus::Denied,
                    remember: false
                }
            }
        )
    );
    check_read_consent(&fixture, &contents);
}

fn check_read_consent(fixture: &Fixture, contents: &[ngterm_core::clipboard_wire::ClipboardRepresentation]) {
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Request {
                id: 10,
                epoch: 8,
                request: read(None, vec!["image/png".into()]),
            },
        )
        .encode(),
    );
    let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!();
    };
    assert!(read.granted && read.can_remember);
    fixture.pane.answer_clipboard_read(
        read.id,
        ClipboardAnswer::Data {
            contents: vec![ClipboardContent {
                mime: "image/png".into(),
                data: contents[0].data.clone(),
            }],
            available: vec!["image/png".into()],
            remember: true,
        },
    );
    assert!(
        matches!(outgoing(fixture).message, Message::Reply { id: 10, reply: DelegatedReply::ClipboardMime { remember: true, contents: returned, .. }, .. } if returned == contents)
    );
}
#[test]
fn lazy_bridge_advertises_sixteen_types_and_fetches_a_later_type_once_without_a_prompt() {
    let fixture = peer();
    fixture.feed.deliver(b"\x1b[?5522h\x1b[5n");
    fixture.sent_until(b"\x1b[0n");
    let contents: Vec<_> = (0..16)
        .map(|index| ClipboardContent {
            mime: format!("application/type{index}"),
            data: vec![0, index, 255],
        })
        .collect();
    fixture
        .pane
        .paste_contents(&contents, PasteSource::Clipboard, ClipboardLocation::Standard, false)
        .unwrap();
    let Message::Paste(paste) = outgoing(&fixture).message else {
        panic!("a paste was expected");
    };
    assert!(paste.contents.is_empty());
    let bridge = paste.bridge.as_ref().unwrap();
    assert_eq!(bridge.available.len(), 16);
    assert!(!format!("{paste:?}").contains("pw="));
    // A forged Hello (including an absent nonce) must leave the negotiated peer and lazy sources intact.
    fixture.feed.deliver(&envelope("forged", Message::Hello).encode());
    fixture.feed.deliver(&envelope("", Message::Hello).encode());
    let request = read(Some(bridge.id.clone()), vec![contents[12].mime.clone()]);
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Request {
                id: 1,
                epoch: 1,
                request: request.clone(),
            },
        )
        .encode(),
    );
    assert!(
        matches!(outgoing(&fixture).message, Message::Reply { reply: DelegatedReply::ClipboardMime { contents: returned, .. }, .. } if returned.len() == 1 && returned[0].data == contents[12].data)
    );
    assert!(
        !fixture
            .pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::ClipboardRead(_)))
    );
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Request {
                id: 2,
                epoch: 1,
                request,
            },
        )
        .encode(),
    );
    assert!(matches!(
        outgoing(&fixture).message,
        Message::Reply {
            reply: DelegatedReply::Denied,
            ..
        }
    ));
}
#[test]
fn only_designated_viewers_interpret_envelopes_and_old_clis_keep_native_clipboard_reads() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(&envelope("connection", Message::Hello).encode());
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Request {
                id: 1,
                epoch: 1,
                request: read(None, vec!["image/png".into()]),
            },
        )
        .encode(),
    );
    fixture.feed.deliver(b"ready");
    fixture.screen_until("ready");
    assert!(
        !fixture
            .pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::ClipboardRead(_)))
    );
    fixture
        .pane
        .paste_contents(
            &[ClipboardContent {
                mime: "text/plain".into(),
                data: b"ordinary-paste".to_vec(),
            }],
            PasteSource::Text,
            ClipboardLocation::Standard,
            false,
        )
        .unwrap();
    fixture.sent_until(b"ordinary-paste");
    let fixture = stream(PaneConfig {
        attach_bridge: Some("connection".into()),
        ..PaneConfig::new(SIZE)
    });
    fixture.feed.deliver(b"\x1b]52;c;?\x07");
    let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!();
    };
    assert_eq!(read.permission_scope, ClipboardPermissionScope::Osc52);
    fixture.pane.answer_clipboard_read(read.id, ClipboardAnswer::Text(None));
    fixture.sent_until(b"\x1b]52;c;\x07");
}

#[test]
fn a_viewer_reconnect_rejects_old_connection_requests() {
    let fixture = stream(PaneConfig {
        attach_bridge: Some("replacement".into()),
        ..PaneConfig::new(SIZE)
    });
    fixture.feed.deliver(&envelope("replacement", Message::Hello).encode());
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Request {
                id: 1,
                epoch: 1,
                request: read(None, vec!["image/png".into()]),
            },
        )
        .encode(),
    );
    fixture.feed.deliver(b"replacement");
    fixture.screen_until("replacement");
    assert!(
        !fixture
            .pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::ClipboardRead(_)))
    );
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
}

#[test]
fn bridge_pastes_report_backpressure_and_an_empty_queue_accepts_a_large_paste() {
    let fixture = peer();
    fixture.pane.write(b"queued").unwrap();
    let contents = vec![ClipboardContent {
        mime: "text/plain".into(),
        data: vec![b'x'; 10 * 1024 * 1024],
    }];
    assert!(matches!(
        fixture
            .pane
            .paste_contents(&contents, PasteSource::Text, ClipboardLocation::Standard, false),
        Err(PaneError::Backpressure)
    ));
    assert_eq!(
        fixture.out.next(Some(DEADLINE)).unwrap(),
        Some(Outgoing::Bytes(b"queued".to_vec()))
    );
    assert_eq!(
        fixture
            .pane
            .paste_contents(&contents, PasteSource::Text, ClipboardLocation::Standard, false)
            .unwrap(),
        crate::Paste::Written
    );
    let Message::Paste(paste) = outgoing(&fixture).message else {
        panic!("expected paste")
    };
    assert_eq!(paste.contents[0].data, contents[0].data);
}

#[test]
fn a_large_bridge_read_reply_uses_the_input_queue_and_reports_refusal() {
    for full in [false, true] {
        let fixture = peer();
        fixture.feed.deliver(
            &envelope(
                "connection",
                Message::Request {
                    id: 20,
                    epoch: 1,
                    request: read(None, vec!["image/png".into()]),
                },
            )
            .encode(),
        );
        let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
            unreachable!()
        };
        if full {
            fixture.pane.write(b"queued").unwrap();
        }
        let contents = vec![ClipboardContent {
            mime: "image/png".into(),
            data: vec![255; 10 * 1024 * 1024],
        }];
        fixture.pane.answer_clipboard_read(
            read.id,
            ClipboardAnswer::Data {
                contents: contents.clone(),
                available: Vec::new(),
                remember: false,
            },
        );
        fixture.feed.deliver(b"reply-complete");
        fixture.screen_until("reply-complete");
        let Message::Reply {
            reply:
                DelegatedReply::ClipboardMime {
                    contents: returned,
                    status,
                    ..
                },
            ..
        } = outgoing(&fixture).message
        else {
            panic!("expected MIME reply")
        };
        if full {
            assert_eq!(status, ClipboardStatus::Busy);
            assert!(returned.is_empty());
        } else {
            assert_eq!(status, ClipboardStatus::Success);
            assert_eq!(returned[0].data, contents[0].data);
        }
    }
}

#[test]
fn expected_nonce_requires_a_matching_hello_before_requests_or_pastes_use_the_bridge() {
    let fixture = stream(PaneConfig {
        attach_bridge: Some("connection".into()),
        ..PaneConfig::new(SIZE)
    });
    for nonce in ["forged", "connection"] {
        if nonce == "forged" {
            fixture.feed.deliver(&envelope(nonce, Message::Hello).encode());
        }
        fixture.feed.deliver(
            &envelope(
                nonce,
                Message::Request {
                    id: 1,
                    epoch: 1,
                    request: read(None, vec!["text/plain".into()]),
                },
            )
            .encode(),
        );
    }
    fixture.feed.deliver(b"ready");
    fixture.screen_until("ready");
    assert!(
        !fixture
            .pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::ClipboardRead(_)))
    );
    fixture
        .pane
        .paste_contents(
            &[ClipboardContent {
                mime: "text/plain".into(),
                data: b"ordinary-paste".to_vec(),
            }],
            PasteSource::Text,
            ClipboardLocation::Standard,
            false,
        )
        .unwrap();
    fixture.sent_until(b"ordinary-paste");
}

#[cfg(feature = "local")]
#[test]
fn follower_and_reattaching_relays_forward_lazy_pastes_and_fetch_the_viewers_later_type() {
    use ngterm_session::attach_tty::{
        input::Answer,
        owner::{Effects, Outgoing as RelayOutgoing, Relay},
    };
    for reattach in [false, true] {
        let fixture = peer();
        fixture.feed.deliver(b"\x1b[?5522h\x1b[5n");
        fixture.sent_until(b"\x1b[0n");
        let contents: Vec<_> = (0..16)
            .map(|index| ClipboardContent {
                mime: format!("application/type{index}"),
                data: vec![0, index, 255],
            })
            .collect();
        let mut relay = Relay::new(None, ngterm_session::protocol::GridSize { cols: 20, rows: 4 }, None);
        relay.enable_bridge(Some("connection".into()));
        if reattach {
            relay.reply_owner(true);
            relay.reattached();
        }
        fixture
            .pane
            .paste_contents(&contents, PasteSource::Clipboard, ClipboardLocation::Standard, false)
            .unwrap();
        let now = Instant::now();
        let mut fx = Effects::default();
        relay.answer(Answer::Bridge(outgoing(&fixture)), now, &mut fx);
        let [RelayOutgoing::Paste(paste)] = fx.outgoing.as_slice() else {
            panic!("{:?}", fx.outgoing)
        };
        assert!(paste.contents.is_empty());
        let bridge = paste.bridge.as_ref().unwrap();
        assert_eq!(bridge.available.len(), 16);
        let request = read(Some(bridge.id.clone()), vec![contents[12].mime.clone()]);
        fx = Effects::default();
        relay.delegated(
            ngterm_session::client::Delegated {
                id: 9,
                epoch: 8,
                native_alias: None,
                request,
            },
            now,
            &mut fx,
        );
        fixture.feed.deliver(&fx.stdout);
        fx = Effects::default();
        relay.answer(Answer::Bridge(outgoing(&fixture)), now, &mut fx);
        assert!(
            matches!(fx.outgoing.as_slice(), [RelayOutgoing::Answer(_, DelegatedReply::ClipboardMime { contents: returned, .. })]
            if returned.len() == 1 && returned[0].data == contents[12].data)
        );
        assert!(
            !fixture
                .pane
                .take_events()
                .iter()
                .any(|event| matches!(event, PaneEvent::ClipboardRead(_)))
        );
    }
}
