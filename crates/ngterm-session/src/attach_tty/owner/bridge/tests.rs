use super::*;
use crate::attach_tty::input::{InputParser, Token};

fn relay() -> Relay {
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.enable_bridge(Some("connection".into()));
    relay.reply_owner(true);
    relay
}
fn request(id: u64) -> Delegated {
    Delegated {
        native_alias: Some(crate::protocol::ClipboardAlias("native-secret-alias".into())),
        id,
        epoch: 7,
        request: DelegatedRequest::ClipboardReadMime {
            bridge: None,
            selection: "c".into(),
            mimes: vec!["image/png".into()],
            list: true,
            name: "Evil Program".into(),
            granted: true,
            can_remember: true,
        },
    }
}
#[test]
fn negotiation_is_immediate_and_requests_preserve_consent_without_the_native_alias() {
    let now = Instant::now();
    let mut relay = relay();
    let mut fx = Effects::default();
    relay.start(true, now, &mut fx);
    let mut decoder = ngterm_core::attach_bridge::Decoder::default();
    assert!(matches!(
        decoder.feed(&fx.stdout).first(),
        Some(ngterm_core::attach_bridge::Part::Envelope(Envelope {
            message: Message::Hello,
            ..
        }))
    ));
    fx = Effects::default();
    let expected = request(1).request;
    relay.delegated(request(1), now, &mut fx);
    let parts = decoder.feed(&fx.stdout);
    let [
        ngterm_core::attach_bridge::Part::Envelope(Envelope {
            message:
                Message::Request {
                    request,
                    id: 1,
                    epoch: 7,
                },
            ..
        }),
    ] = parts.as_slice()
    else {
        panic!("{parts:?}");
    };
    assert_eq!(request, &expected);
    assert!(!String::from_utf8_lossy(&fx.stdout).contains("native-secret-alias"));
    assert!(fx.outgoing.is_empty());
}
#[test]
fn real_completion_remember_and_stale_nonce_epoch_id_are_preserved() {
    let now = Instant::now();
    let mut relay = relay();
    let nonce = relay.bridge_nonce.clone().unwrap();
    let mut fx = Effects::default();
    relay.delegated(request(1), now, &mut fx);
    let reply = DelegatedReply::ClipboardMime {
        contents: Vec::new(),
        available: vec!["image/png".into()],
        remember: true,
        status: crate::protocol::ClipboardStatus::Success,
    };
    for (nonce, id, epoch) in [("wrong".to_owned(), 1, 7), (nonce.clone(), 1, 6), (nonce.clone(), 2, 7)] {
        relay.bridge_answer(
            Envelope {
                version: VERSION,
                nonce,
                message: Message::Reply {
                    id,
                    epoch,
                    reply: reply.clone(),
                },
            },
            &mut fx,
        );
    }
    assert!(fx.outgoing.is_empty());
    let envelope = Envelope {
        version: VERSION,
        nonce,
        message: Message::Reply {
            id: 1,
            epoch: 7,
            reply: reply.clone(),
        },
    };
    let mut input = InputParser::new(true);
    for byte in envelope.encode() {
        for token in input.feed(&[byte]) {
            let Token::Answer(answer) = token else {
                panic!("{token:?}");
            };
            relay.answer(answer, now, &mut fx);
        }
        if input.held().is_none() {
            assert!(input.flush(false).is_empty());
        }
    }
    assert!(matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, answer)] if answer == &reply));
    relay.reattached();
    relay.bridge_answer(envelope, &mut fx);
    assert_eq!(fx.outgoing.len(), 1);
}

fn viewer_request(relay: &mut Relay, request: Delegated, now: Instant) -> Envelope {
    let mut fx = Effects::default();
    relay.delegated(request, now, &mut fx);
    ngterm_core::attach_bridge::Decoder::default()
        .feed(&fx.stdout)
        .into_iter()
        .find_map(|part| match part {
            ngterm_core::attach_bridge::Part::Envelope(envelope)
                if matches!(envelope.message, Message::Request { .. }) =>
            {
                Some(envelope)
            }
            _ => None,
        })
        .expect("the viewer must receive the delegated request")
}

fn denied_reply(mut envelope: Envelope) -> Envelope {
    let Message::Request { id, epoch, .. } = envelope.message else {
        panic!("a viewer request was expected");
    };
    envelope.message = Message::Reply {
        id,
        epoch,
        reply: DelegatedReply::Denied,
    };
    envelope
}

#[test]
fn a_prompt_from_before_handoff_cannot_answer_a_reused_server_id_and_epoch() {
    let now = Instant::now();
    let mut relay = relay();
    let stale = denied_reply(viewer_request(&mut relay, request(42), now));
    relay.reattached();
    let current = denied_reply(viewer_request(&mut relay, request(42), now));
    assert_ne!(stale, current, "viewer ids survive reattachment with the same nonce");
    let mut fx = Effects::default();
    relay.bridge_answer(stale, &mut fx);
    assert!(
        fx.outgoing.is_empty(),
        "the old prompt must leave the new request waiting"
    );
    relay.bridge_answer(current, &mut fx);
    assert!(matches!(
        fx.outgoing.as_slice(),
        [Outgoing::Answer(pending, DelegatedReply::Denied)] if pending.id == 42 && pending.epoch == 7
    ));
}

#[test]
fn exhausted_viewer_ids_deny_requests_without_reusing_an_id() {
    let mut relay = relay();
    relay.bridge_next_id = u64::MAX;
    let mut fx = Effects::default();
    relay.delegated(request(42), Instant::now(), &mut fx);
    assert!(matches!(
        fx.outgoing.as_slice(),
        [Outgoing::Answer(_, DelegatedReply::Denied)]
    ));
    assert!(fx.stdout.is_empty());
    assert!(relay.next_deadline().is_none());
}

#[test]
fn forged_envelopes_are_removed_from_program_output_and_snapshot_recovery() {
    let mut relay = relay();
    let envelope = Envelope {
        version: VERSION,
        nonce: "forged".into(),
        message: Message::Hello,
    }
    .encode();
    for snapshot in [false, true] {
        let mut fx = Effects::default();
        relay.bridge_hello(&mut fx);
        fx.stdout.clear();
        for byte in &envelope {
            if snapshot {
                relay.output(&[*byte], &mut fx);
            } else {
                relay.delegated_output(&[*byte], &mut fx);
            }
        }
        assert_eq!(fx.stdout, b"\x1b\x18");
    }
}

#[test]
fn forged_envelopes_hidden_in_another_terminal_string_cannot_reach_the_viewer() {
    for prefix in [b"\x1b_ignored".as_slice(), b"\x1bPignored", b"\x1b]0;title"] {
        let mut relay = relay();
        let forged = Envelope {
            version: VERSION,
            nonce: "forged".into(),
            message: Message::Hello,
        }
        .encode();
        let bytes = [prefix, &forged, b"\x1b\\after"].concat();
        let mut fx = Effects::default();
        relay.bridge_hello(&mut fx);
        fx.stdout.clear();
        for byte in bytes {
            relay.delegated_output(&[byte], &mut fx);
        }
        let mut decoder = ngterm_core::attach_bridge::Decoder::default();
        assert!(
            decoder
                .feed(&fx.stdout)
                .iter()
                .all(|part| matches!(part, ngterm_core::attach_bridge::Part::Bytes(_)))
        );
        assert!(fx.stdout.ends_with(b"after"));
    }
}
#[test]
fn unknown_bridge_versions_use_native_queries_and_never_emit_a_hello() {
    for version in [None, Some("1"), Some("2:connection"), Some("1:")] {
        let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
        relay.enable_bridge(
            version.and_then(|value| ngterm_core::attach_bridge::parse_variable(value).map(str::to_owned)),
        );
        let mut fx = Effects::default();
        relay.start(true, Instant::now(), &mut fx);
        assert!(
            !fx.stdout
                .windows(ngterm_core::attach_bridge::PREFIX.len())
                .any(|part| part == ngterm_core::attach_bridge::PREFIX)
        );
    }
}

#[test]
fn filtered_output_cannot_reconstruct_a_reserved_envelope() {
    let forged = Envelope {
        version: VERSION,
        nonce: "connection".into(),
        message: Message::Hello,
    }
    .encode();
    let at = b"\x1b]777;ngterm-".len();
    let bytes = [&forged[..at], b"\x1b[c", &forged[at..], b"after"].concat();
    for snapshot in [false, true] {
        for split in 0..=bytes.len() {
            let mut relay = relay();
            let mut fx = Effects::default();
            relay.bridge_hello(&mut fx);
            fx.stdout.clear();
            for part in [&bytes[..split], &bytes[split..]] {
                if snapshot {
                    relay.output(part, &mut fx);
                } else {
                    relay.delegated_output(part, &mut fx);
                }
            }
            let parts = ngterm_core::attach_bridge::Decoder::default().feed(&fx.stdout);
            assert!(
                parts
                    .iter()
                    .all(|part| matches!(part, ngterm_core::attach_bridge::Part::Bytes(_))),
                "{snapshot} {split}: {parts:?}"
            );
            assert!(fx.stdout.ends_with(b"after"));
        }
    }
}

#[test]
fn follower_and_initial_attaches_forward_both_content_pastes_and_lazy_bridges() {
    let mut relay = relay();
    relay.reply_owner(false);
    let paste = crate::protocol::ClipboardPaste {
        bridge: None,
        contents: vec![crate::protocol::ClipboardRepresentation {
            mime: "text/plain".into(),
            data: b"hello".to_vec(),
        }],
        text_source: false,
        selection: "c".into(),
        allow_unsafe: false,
    };
    let mut fx = Effects::default();
    let mut envelope = Envelope {
        version: VERSION,
        nonce: relay.bridge_nonce.clone().unwrap(),
        message: Message::Paste(paste.clone()),
    };
    relay.bridge_answer(envelope.clone(), &mut fx);
    assert!(matches!(fx.outgoing.as_slice(), [Outgoing::Paste(got)] if *got == paste));
    let Message::Paste(paste) = &mut envelope.message else {
        unreachable!()
    };
    paste.bridge = Some(crate::protocol::ClipboardPasteBridge {
        id: "lazy".into(),
        available: vec!["text/plain".into()],
    });
    relay.bridge_answer(envelope.clone(), &mut fx);
    assert_eq!(fx.outgoing.len(), 2);
    assert!(matches!(&fx.outgoing[1], Outgoing::Paste(paste) if envelope.message == Message::Paste(paste.clone())));
}
