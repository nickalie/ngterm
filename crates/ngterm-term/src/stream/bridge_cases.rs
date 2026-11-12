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

fn request(fixture: &Fixture, id: u64, request: DelegatedRequest) {
    fixture
        .feed
        .deliver(&envelope("connection", Message::Request { id, epoch: 1, request }).encode());
}

fn reply(fixture: &Fixture) -> (u64, DelegatedReply) {
    let Message::Reply { id, reply, .. } = outgoing(fixture).message else {
        panic!("expected a reply")
    };
    (id, reply)
}

fn clipboard_write(selection: &str, kitty: bool) -> DelegatedRequest {
    DelegatedRequest::ClipboardWrite {
        selection: selection.into(),
        contents: vec![ngterm_core::clipboard_wire::ClipboardRepresentation {
            mime: "text/plain".into(),
            data: b"copied".to_vec(),
        }],
        name: String::new(),
        granted: false,
        can_remember: false,
        kitty,
    }
}

#[test]
fn the_frontends_clipboard_failures_reach_the_viewer_as_their_wire_status() {
    let fixture = peer();
    for (id, answer, status) in [
        (0, ClipboardAnswer::Unsupported, ClipboardStatus::Unsupported),
        (1, ClipboardAnswer::Busy, ClipboardStatus::Busy),
        (2, ClipboardAnswer::IoError, ClipboardStatus::IoError),
    ] {
        request(&fixture, id, read(None, vec!["text/plain".into()]));
        let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
            unreachable!()
        };
        fixture.pane.answer_clipboard_read(read.id, answer);
        assert_eq!(
            reply(&fixture),
            (
                id,
                DelegatedReply::ClipboardMime {
                    contents: Vec::new(),
                    available: Vec::new(),
                    remember: false,
                    status
                }
            )
        );
    }
    for (id, result, status) in [
        (10, ClipboardWriteResult::Success, ClipboardStatus::Success),
        (11, ClipboardWriteResult::Unsupported, ClipboardStatus::Unsupported),
        (12, ClipboardWriteResult::Busy, ClipboardStatus::Busy),
        (13, ClipboardWriteResult::InvalidData, ClipboardStatus::InvalidData),
        (14, ClipboardWriteResult::IoError, ClipboardStatus::IoError),
    ] {
        request(&fixture, id, clipboard_write("s", false));
        let PaneEvent::ClipboardWrite(write) = fixture.event(|event| matches!(event, PaneEvent::ClipboardWrite(_)))
        else {
            unreachable!()
        };
        assert_eq!(write.permission_scope, ClipboardPermissionScope::Osc52);
        assert_eq!(write.location, ClipboardLocation::Selection);
        fixture.pane.answer_clipboard_write(
            write.id,
            WriteCompletion {
                status: result,
                remember: true,
            },
        );
        assert_eq!(
            reply(&fixture),
            (id, DelegatedReply::ClipboardWrite { status, remember: true })
        );
    }
}

#[test]
fn a_kitty_listing_asks_the_frontend_for_types_only() {
    let fixture = peer();
    request(
        &fixture,
        1,
        DelegatedRequest::ClipboardReadMime {
            bridge: None,
            selection: "p".into(),
            mimes: Vec::new(),
            list: true,
            name: String::new(),
            granted: false,
            can_remember: false,
        },
    );
    let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!()
    };
    assert_eq!(read.scope, crate::ReadScope::ListOnly);
    assert_eq!(read.location, ClipboardLocation::Primary);
    assert!(read.list && read.mimes.is_empty());
    assert_eq!(read.permission_scope, ClipboardPermissionScope::Kitty);
}

/// Paste `count` one-type values lazily; each paste's bridge id and its type.
fn lazy_pastes(fixture: &Fixture, count: u8) -> Vec<(String, ClipboardContent)> {
    (0..count)
        .map(|index| {
            let content = ClipboardContent {
                mime: format!("application/type{index}"),
                data: vec![index],
            };
            fixture
                .pane
                .paste_contents(
                    std::slice::from_ref(&content),
                    PasteSource::Clipboard,
                    ClipboardLocation::Standard,
                    false,
                )
                .unwrap();
            let Message::Paste(paste) = outgoing(fixture).message else {
                panic!("a paste was expected");
            };
            (paste.bridge.unwrap().id, content)
        })
        .collect()
}

fn fetch(bridge: &str, mimes: Vec<String>, list: bool) -> DelegatedRequest {
    DelegatedRequest::ClipboardReadMime {
        bridge: Some(bridge.into()),
        selection: "c".into(),
        mimes,
        list,
        name: String::new(),
        granted: false,
        can_remember: false,
    }
}

#[test]
fn lazy_pastes_list_their_types_keep_the_latest_thirty_two_and_expire() {
    let fixture = peer();
    fixture.feed.deliver(b"\x1b[?5522h\x1b[5n");
    fixture.sent_until(b"\x1b[0n");
    let pastes = lazy_pastes(&fixture, 33);
    // the oldest is gone once 32 newer ones wait
    request(&fixture, 1, fetch(&pastes[0].0, vec![pastes[0].1.mime.clone()], false));
    assert_eq!(reply(&fixture), (1, DelegatedReply::Denied));
    // a listing serves the types and keeps the paste for its data
    let (bridge, content) = &pastes[32];
    request(&fixture, 2, fetch(bridge, Vec::new(), true));
    assert_eq!(
        reply(&fixture),
        (
            2,
            DelegatedReply::ClipboardMime {
                contents: Vec::new(),
                available: vec![content.mime.clone()],
                remember: false,
                status: ClipboardStatus::Success,
            }
        )
    );
    request(&fixture, 3, fetch(bridge, vec![content.mime.clone()], false));
    let (
        3,
        DelegatedReply::ClipboardMime {
            contents, available, ..
        },
    ) = reply(&fixture)
    else {
        panic!("expected the paste's data");
    };
    assert_eq!(contents[0].data, content.data);
    assert!(available.is_empty());
    // a paste nobody fetched within 30 seconds is denied
    let (bridge, content) = &pastes[1];
    for (id, _, pasted) in &mut lock(&fixture.pane.shared().bridge).pastes {
        if id == bridge {
            *pasted = Instant::now().checked_sub(Duration::from_secs(31)).unwrap();
        }
    }
    request(&fixture, 4, fetch(bridge, vec![content.mime.clone()], false));
    assert_eq!(reply(&fixture), (4, DelegatedReply::Denied));
    assert!(
        !fixture
            .pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::ClipboardRead(_))),
        "no fetch asks the frontend"
    );
}

#[test]
fn a_text_paste_to_primary_goes_whole_over_the_bridge() {
    let fixture = peer();
    fixture
        .pane
        .paste("primary", PasteSource::Text, ClipboardLocation::Primary, true)
        .unwrap();
    let Message::Paste(paste) = outgoing(&fixture).message else {
        panic!("a paste was expected");
    };
    assert_eq!(paste.selection, "p");
    assert!(paste.text_source && paste.allow_unsafe && paste.bridge.is_none());
    assert_eq!(paste.contents[0].data, b"primary");
}

#[test]
fn a_reply_the_full_input_queue_refuses_is_replaced_by_its_status() {
    let fixture = stream(PaneConfig {
        attach_bridge: Some("connection".into()),
        write_limit: 8,
        ..PaneConfig::new(SIZE)
    });
    fixture.feed.deliver(&envelope("connection", Message::Hello).encode());
    fixture.pane.write(b"queued").unwrap();
    request(&fixture, 1, clipboard_write("c", true));
    let PaneEvent::ClipboardWrite(write) = fixture.event(|event| matches!(event, PaneEvent::ClipboardWrite(_))) else {
        unreachable!()
    };
    fixture.pane.answer_clipboard_write(
        write.id,
        WriteCompletion {
            status: ClipboardWriteResult::Success,
            remember: true,
        },
    );
    request(
        &fixture,
        2,
        DelegatedRequest::ClipboardRead {
            selection: "c".into(),
            list_only: false,
        },
    );
    let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!()
    };
    assert_eq!(read.permission_scope, ClipboardPermissionScope::Osc52);
    fixture
        .pane
        .answer_clipboard_read(read.id, ClipboardAnswer::Text(Some("copied".into())));
    // both replies were tried before the queued input leaves
    fixture.feed.deliver(b"replied");
    fixture.screen_until("replied");
    let mut decoder = Decoder::default();
    let mut parts = Vec::new();
    while parts.len() < 3 {
        let Ok(Some(Outgoing::Bytes(bytes))) = fixture.out.next(Some(DEADLINE)) else {
            panic!("sent only {parts:?}");
        };
        parts.extend(decoder.feed(&bytes).into_iter().map(|part| match part {
            Part::Bytes(bytes) => Err(bytes),
            Part::Envelope(envelope) => Ok(envelope.message),
        }));
    }
    assert_eq!(
        parts,
        [
            Err(b"queued".to_vec()),
            Ok(Message::Reply {
                id: 1,
                epoch: 1,
                reply: DelegatedReply::ClipboardWrite {
                    status: ClipboardStatus::Busy,
                    remember: false
                }
            }),
            Ok(Message::Reply {
                id: 2,
                epoch: 1,
                reply: DelegatedReply::Denied
            }),
        ]
    );
}

#[test]
fn a_viewers_own_replies_and_pastes_are_not_requests() {
    let fixture = peer();
    fixture.feed.deliver(
        &envelope(
            "connection",
            Message::Reply {
                id: 1,
                epoch: 1,
                reply: DelegatedReply::Denied,
            },
        )
        .encode(),
    );
    fixture.feed.deliver(b"after");
    fixture.screen_until("after");
    assert!(
        fixture
            .pane
            .take_events()
            .iter()
            .all(|event| !matches!(event, PaneEvent::ClipboardRead(_) | PaneEvent::ClipboardWrite(_)))
    );
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
}

#[test]
fn a_pane_dropped_while_a_bridge_read_waits_sends_no_reply() {
    let fixture = peer();
    request(&fixture, 1, read(None, vec!["text/plain".into()]));
    fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_)));
    let Fixture { pane, out, .. } = fixture;
    drop(pane);
    assert!(!matches!(
        out.next(Some(Duration::from_millis(10))),
        Ok(Some(Outgoing::Bytes(_)))
    ));
}
