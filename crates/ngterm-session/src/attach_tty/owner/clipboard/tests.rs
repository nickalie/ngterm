use super::*;
use crate::attach_tty::input::{Answer, InputParser, Token};
use crate::protocol::GridSize;
use ngterm_ghostty_vt as vt;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    bytes: Vec<u8>,
    writes: Vec<Vec<vt::ClipboardContent>>,
    reads: Vec<(bool, bool)>,
}
struct Outer {
    seen: Arc<Mutex<Seen>>,
    contents: Vec<vt::ClipboardContent>,
    result: vt::ClipboardWriteResult,
}
impl vt::Effects for Outer {
    fn write_pty(&mut self, bytes: &[u8]) {
        self.seen.lock().unwrap().bytes.extend_from_slice(bytes);
    }
    fn clipboard_read(&mut self, request: &vt::ClipboardReadRequest<'_>) -> vt::ClipboardReadReply {
        self.seen
            .lock()
            .unwrap()
            .reads
            .push((request.granted, request.can_remember));
        vt::ClipboardReadReply::Success {
            contents: self
                .contents
                .iter()
                .filter(|part| request.mimes.contains(&part.mime.as_bytes()))
                .cloned()
                .collect(),
            available: self.contents.iter().map(|part| part.mime.clone()).collect(),
        }
    }
    fn clipboard_read_reply(&mut self, request: &vt::ClipboardReadRequest<'_>) -> (vt::ClipboardReadReply, bool) {
        (self.clipboard_read(request), true)
    }
    fn clipboard_write(&mut self, request: &vt::ClipboardWriteRequest<'_>) -> vt::ClipboardWriteResult {
        self.seen.lock().unwrap().writes.push(
            request
                .contents
                .iter()
                .map(|part| vt::ClipboardContent {
                    mime: String::from_utf8(part.mime.to_vec()).unwrap(),
                    data: part.data.to_vec(),
                })
                .collect(),
        );
        self.result
    }
}
fn fixture() -> Vec<vt::ClipboardContent> {
    [
        ("text/plain", b"Ghostty multipart clipboard".as_slice()),
        ("text/html", b"<b>Ghostty</b> multipart clipboard".as_slice()),
        (
            "image/png",
            include_bytes!("../../../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").as_slice(),
        ),
        (
            "application/octet-stream",
            include_bytes!("../../../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin").as_slice(),
        ),
    ]
    .into_iter()
    .map(|(mime, data)| vt::ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    })
    .collect()
}
fn outer(result: vt::ClipboardWriteResult) -> (vt::Terminal, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut terminal = vt::Terminal::new(80, 24).unwrap();
    terminal
        .set_effects(Box::new(Outer {
            seen: seen.clone(),
            contents: fixture(),
            result,
        }))
        .unwrap();
    (terminal, seen)
}
fn answers(relay: &mut Relay, bytes: &[u8], now: Instant, fx: &mut Effects) {
    let mut input = InputParser::new(true);
    for byte in bytes {
        for token in input.feed(&[*byte]) {
            let Token::Answer(answer) = token else {
                panic!("reply became input: {token:?}");
            };
            relay.answer(answer, now, fx);
        }
        if input.held().is_none() {
            assert!(input.flush(true).is_empty());
        }
    }
    assert!(!input.holding());
}
fn mime_read(id: u64) -> Delegated {
    Delegated {
        native_alias: None,
        id,
        epoch: 1,
        request: DelegatedRequest::ClipboardReadMime {
            bridge: None,
            selection: "c".into(),
            mimes: fixture().iter().map(|part| part.mime.clone()).collect(),
            list: true,
            name: "Evil Program".into(),
            granted: false,
            can_remember: true,
        },
    }
}
#[test]
fn fake_outer_reads_the_four_part_fixture_and_chunks_without_typing_reply_bytes() {
    let now = Instant::now();
    let (mut terminal, seen) = outer(vt::ClipboardWriteResult::Success);
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    let mut fx = Effects::default();
    relay.start(true, now, &mut fx);
    terminal.vt_write(&std::mem::take(&mut fx.stdout));
    answers(
        &mut relay,
        &std::mem::take(&mut seen.lock().unwrap().bytes),
        now,
        &mut fx,
    );
    assert!(relay.native_clipboard.supported);
    fx.outgoing.clear();
    let request = mime_read(1);
    relay.delegated(request, now, &mut fx);
    assert!(fx.outgoing.is_empty());
    terminal.vt_write(&std::mem::take(&mut fx.stdout));
    answers(
        &mut relay,
        &std::mem::take(&mut seen.lock().unwrap().bytes),
        now,
        &mut fx,
    );
    let [
        Outgoing::Answer(
            _,
            DelegatedReply::ClipboardMime {
                contents,
                available,
                status,
                ..
            },
        ),
    ] = fx.outgoing.as_slice()
    else {
        panic!("{:?}", fx.outgoing);
    };
    assert_eq!(*status, ClipboardStatus::Success);
    assert_eq!(
        contents.iter().map(|part| (&part.mime, &part.data)).collect::<Vec<_>>(),
        fixture()
            .iter()
            .map(|part| (&part.mime, &part.data))
            .collect::<Vec<_>>()
    );
    assert_eq!(available.len(), 4);
}
#[test]
fn fake_outer_write_status_controls_completion_and_preserves_every_byte() {
    for (native, expected) in [
        (vt::ClipboardWriteResult::Success, ClipboardStatus::Success),
        (vt::ClipboardWriteResult::Denied, ClipboardStatus::Denied),
        (vt::ClipboardWriteResult::Unsupported, ClipboardStatus::Unsupported),
        (vt::ClipboardWriteResult::Busy, ClipboardStatus::Busy),
        (vt::ClipboardWriteResult::InvalidData, ClipboardStatus::InvalidData),
        (vt::ClipboardWriteResult::IoError, ClipboardStatus::IoError),
    ] {
        let now = Instant::now();
        let (mut terminal, seen) = outer(native);
        let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
        relay.native_clipboard.supported = true;
        let mut fx = Effects::default();
        relay.delegated(
            Delegated {
                native_alias: None,
                id: 1,
                epoch: 1,
                request: DelegatedRequest::ClipboardWrite {
                    selection: "c".into(),
                    contents: fixture()
                        .into_iter()
                        .map(|part| ClipboardRepresentation {
                            mime: part.mime,
                            data: part.data,
                        })
                        .collect(),
                    name: "Evil Program".into(),
                    granted: false,
                    can_remember: true,
                    kitty: true,
                },
            },
            now,
            &mut fx,
        );
        assert!(fx.outgoing.is_empty());
        terminal.vt_write(&std::mem::take(&mut fx.stdout));
        assert_eq!(seen.lock().unwrap().writes, vec![fixture()]);
        assert!(fx.outgoing.is_empty());
        answers(
            &mut relay,
            &std::mem::take(&mut seen.lock().unwrap().bytes),
            now,
            &mut fx,
        );
        assert!(
            matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::ClipboardWrite { status, .. })] if *status == expected)
        );
    }
}
#[test]
fn timed_out_and_reconnected_replies_cannot_complete_the_next_request() {
    let now = Instant::now();
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.native_clipboard.supported = true;
    let mut fx = Effects::default();
    relay.delegated(mime_read(1), now, &mut fx);
    let old = relay.native_clipboard.pending.as_ref().unwrap().id.clone();
    relay.tick(now + DELEGATION_WAIT, &mut fx);
    assert_eq!(fx.outgoing.len(), 1);
    relay.delegated(mime_read(2), now + DELEGATION_WAIT, &mut fx);
    let stale = format!("5522;type=read:status=DONE:id={old}");
    relay.answer(
        Answer::ClipboardMime(Packet::parse(stale.as_bytes()).unwrap()),
        now,
        &mut fx,
    );
    assert_eq!(fx.outgoing.len(), 1);
    assert_eq!(relay.native_clipboard.pending.as_ref().unwrap().request.id, 2);
    relay.reattached();
    relay.answer(
        Answer::ClipboardMime(Packet::parse(stale.as_bytes()).unwrap()),
        now,
        &mut fx,
    );
    assert_eq!(fx.outgoing.len(), 1);
}

#[test]
fn native_lazy_paste_keeps_the_outer_token_local_and_fetches_a_later_type_without_another_prompt() {
    for owner in [false, true] {
        native_lazy_paste(owner);
    }
}

fn native_lazy_paste(owner: bool) {
    let now = Instant::now();
    let (mut terminal, seen) = outer(vt::ClipboardWriteResult::Success);
    terminal.vt_write(b"\x1b[?5522h");
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.reply_owner(owner);
    relay.native_clipboard.supported = true;
    let mut fx = Effects::default();
    let contents: Vec<_> = (0..16)
        .map(|index| vt::ClipboardContent {
            mime: format!("application/type{index}"),
            data: vec![0, index, 255],
        })
        .collect();
    terminal
        .set_effects(Box::new(Outer {
            seen: seen.clone(),
            contents: contents.clone(),
            result: vt::ClipboardWriteResult::Success,
        }))
        .unwrap();
    terminal
        .paste_contents(
            &contents,
            vt::PasteSource::Clipboard,
            vt::ClipboardLocation::Standard,
            false,
        )
        .unwrap();
    let event = std::mem::take(&mut seen.lock().unwrap().bytes);
    terminal.clipboard_grants().lock().unwrap().pasted(&event);
    answers(&mut relay, &event, now, &mut fx);
    let [Outgoing::Paste(paste)] = fx.outgoing.as_slice() else {
        panic!("{:?}", fx.outgoing);
    };
    let bridge = paste.bridge.clone().unwrap();
    assert!(paste.contents.is_empty());
    assert_eq!(bridge.available.len(), 16);
    assert_eq!(seen.lock().unwrap().reads.len(), 0, "registration never reads any data");
    let mut request = mime_read(1);
    if let DelegatedRequest::ClipboardReadMime { bridge: id, mimes, .. } = &mut request.request {
        *id = Some(bridge.id);
        *mimes = vec!["application/type12".into()];
    }
    fx = Effects::default();
    relay.delegated(request, now, &mut fx);
    terminal.vt_write(&std::mem::take(&mut fx.stdout));
    answers(
        &mut relay,
        &std::mem::take(&mut seen.lock().unwrap().bytes),
        now,
        &mut fx,
    );
    assert_eq!(seen.lock().unwrap().reads, [(true, true)]);
    assert!(
        matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::ClipboardMime { contents: returned, .. })] if returned.len() == 1 && returned[0].data == [0,12,255])
    );
}

#[test]
fn native_password_alias_reuses_the_outer_terminals_remembered_read_without_exporting_remember() {
    let now = Instant::now();
    let (mut terminal, seen) = outer(vt::ClipboardWriteResult::Success);
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.native_clipboard.supported = true;
    for (id, alias) in [(1, "alias"), (2, "alias"), (3, "rotated")] {
        let mut request = mime_read(id);
        request.native_alias = Some(crate::protocol::ClipboardAlias(alias.into()));
        let mut fx = Effects::default();
        relay.delegated(request, now, &mut fx);
        terminal.vt_write(&fx.stdout);
        fx.stdout.clear();
        answers(
            &mut relay,
            &std::mem::take(&mut seen.lock().unwrap().bytes),
            now,
            &mut fx,
        );
        assert!(matches!(
            fx.outgoing.as_slice(),
            [Outgoing::Answer(
                _,
                DelegatedReply::ClipboardMime { remember: false, .. }
            )]
        ));
    }
    assert_eq!(seen.lock().unwrap().reads, [(false, true), (true, true), (false, true)]);
}

#[test]
fn a_native_lazy_paste_cannot_be_fetched_after_timeout_owner_loss_or_reconnect() {
    let now = Instant::now();
    for invalidation in 0..3 {
        let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
        relay.reply_owner(true);
        relay.native_clipboard.supported = true;
        let mut fx = Effects::default();
        let event = b"\x1b]5522;type=read:status=OK:pw=b3V0ZXI=\x1b\\\x1b]5522;type=read:status=DATA:mime=Lg==:pw=b3V0ZXI=;aW1hZ2UvcG5nCg==\x1b\\\x1b]5522;type=read:status=DONE:pw=b3V0ZXI=\x1b\\";
        answers(&mut relay, event, now, &mut fx);
        let [Outgoing::Paste(paste)] = fx.outgoing.as_slice() else {
            panic!("{:?}", fx.outgoing);
        };
        let bridge = paste.bridge.as_ref().unwrap().id.clone();
        let when = match invalidation {
            0 => now + DELEGATION_WAIT,
            1 => {
                relay.reply_owner(false);
                now
            }
            _ => {
                relay.reattached();
                now
            }
        };
        let mut request = mime_read(1);
        if let DelegatedRequest::ClipboardReadMime { bridge: id, .. } = &mut request.request {
            *id = Some(bridge);
        }
        fx = Effects::default();
        relay.delegated(request, when, &mut fx);
        assert!(fx.stdout.is_empty());
        assert!(matches!(
            fx.outgoing.as_slice(),
            [Outgoing::Answer(_, DelegatedReply::Denied)]
        ));
    }
}

#[test]
fn oversized_native_data_completes_with_invalid_data_instead_of_an_unencodable_reply() {
    let now = Instant::now();
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.native_clipboard.supported = true;
    let mut fx = Effects::default();
    relay.delegated(mime_read(1), now, &mut fx);
    let pending = relay.native_clipboard.pending.as_mut().unwrap();
    pending.started = true;
    let packet = Packet {
        operation: "read".into(),
        status: "DATA".into(),
        id: pending.id.clone(),
        mime: "image/png".into(),
        data: vec![0; ngterm_core::attach_bridge::DATA_LIMIT + 1],
        token: Vec::new(),
        primary: false,
    };
    relay.native_answer(packet, now, &mut fx);
    assert!(
        matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::ClipboardMime { status: ClipboardStatus::InvalidData, contents, .. })] if contents.is_empty())
    );
    assert!(relay.native_clipboard.pending.is_none());
}

fn supported() -> Relay {
    let mut relay = Relay::new(None, GridSize { cols: 80, rows: 24 }, None);
    relay.native_clipboard.supported = true;
    relay
}
fn native(relay: &mut Relay, body: &str, now: Instant, fx: &mut Effects) {
    relay.answer(Answer::ClipboardMime(Packet::parse(body.as_bytes()).unwrap()), now, fx);
}
fn pending_id(relay: &Relay) -> String {
    relay.native_clipboard.pending.as_ref().unwrap().id.clone()
}
fn b64(bytes: impl AsRef<[u8]>) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
fn completed_status(fx: &Effects) -> Option<ClipboardStatus> {
    match fx.outgoing.as_slice() {
        [Outgoing::Answer(_, DelegatedReply::ClipboardMime { status, .. })] => Some(*status),
        [] => None,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_native_operation_or_an_osc_52_read_in_flight_denies_the_next_request_without_asking() {
    let now = Instant::now();
    let mut relay = supported();
    let mut fx = Effects::default();
    relay.delegated(mime_read(1), now, &mut fx);
    assert!(!fx.stdout.is_empty());
    fx = Effects::default();
    relay.delegated(mime_read(2), now, &mut fx);
    assert!(fx.stdout.is_empty());
    assert!(matches!(fx.outgoing.as_slice(), [Outgoing::Answer(request, DelegatedReply::Denied)] if request.id == 2));
    assert_eq!(relay.native_clipboard.pending.as_ref().unwrap().request.id, 1);

    let mut relay = supported();
    relay.fences_work = Some(true);
    let mut fx = Effects::default();
    relay.delegated(
        Delegated {
            native_alias: None,
            id: 1,
            epoch: 1,
            request: DelegatedRequest::ClipboardRead {
                selection: "c".into(),
                list_only: false,
            },
        },
        now,
        &mut fx,
    );
    assert_eq!(fx.stdout, b"\x1b]52;c;?\x1b\\");
    fx = Effects::default();
    relay.delegated(mime_read(2), now, &mut fx);
    assert!(fx.stdout.is_empty());
    assert!(matches!(fx.outgoing.as_slice(), [Outgoing::Answer(request, DelegatedReply::Denied)] if request.id == 2));
}

#[test]
fn a_native_read_ignores_other_operations_and_unknown_statuses_and_a_bare_done_is_invalid() {
    let now = Instant::now();
    let mut relay = supported();
    let mut fx = Effects::default();
    relay.delegated(mime_read(1), now, &mut fx);
    let id = pending_id(&relay);
    native(
        &mut relay,
        &format!("5522;type=write:status=DONE:id={id}"),
        now,
        &mut fx,
    );
    native(
        &mut relay,
        &format!("5522;type=read:status=LATER:id={id}"),
        now,
        &mut fx,
    );
    assert_eq!(completed_status(&fx), None);
    native(&mut relay, &format!("5522;type=read:status=DONE:id={id}"), now, &mut fx);
    assert_eq!(completed_status(&fx), Some(ClipboardStatus::InvalidData));
    assert!(relay.native_clipboard.pending.is_none());
}

#[test]
fn native_read_chunks_of_one_type_join_and_unrequested_or_undecodable_parts_are_invalid() {
    let now = Instant::now();
    let mut relay = supported();
    let mut fx = Effects::default();
    relay.delegated(mime_read(1), now, &mut fx);
    let id = pending_id(&relay);
    let mime = b64("text/plain");
    for packet in [
        format!("5522;type=read:status=OK:id={id}"),
        format!("5522;type=read:status=DATA:id={id}:mime={mime};{}", b64("multi")),
        format!("5522;type=read:status=DATA:id={id}:mime={mime};{}", b64("part")),
        format!("5522;type=read:status=DONE:id={id}"),
    ] {
        native(&mut relay, &packet, now, &mut fx);
    }
    assert!(
        matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::ClipboardMime { contents, status: ClipboardStatus::Success, .. })]
        if contents.len() == 1 && contents[0].mime == "text/plain" && contents[0].data == b"multipart")
    );

    for (mime, data) in [(b64("image/gif"), b64("GIF89a")), (b64("."), b64([0xff, 0xfe]))] {
        let mut relay = supported();
        let mut fx = Effects::default();
        relay.delegated(mime_read(1), now, &mut fx);
        let id = pending_id(&relay);
        native(&mut relay, &format!("5522;type=read:status=OK:id={id}"), now, &mut fx);
        native(
            &mut relay,
            &format!("5522;type=read:status=DATA:id={id}:mime={mime};{data}"),
            now,
            &mut fx,
        );
        assert!(
            matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::ClipboardMime { contents, available, status: ClipboardStatus::InvalidData, .. })]
            if contents.is_empty() && available.is_empty())
        );
    }
}

#[test]
fn a_native_paste_registers_only_a_whole_live_announcement_of_one_token() {
    let now = Instant::now();
    let ok = |token: &str| format!("5522;type=read:status=OK:pw={}", b64(token));
    let done = |token: &str| format!("5522;type=read:status=DONE:pw={}", b64(token));
    let types = |token: &str, list: &[u8]| {
        format!(
            "5522;type=read:status=DATA:mime={}:pw={};{}",
            b64("."),
            b64(token),
            b64(list)
        )
    };
    let broken: [(Vec<String>, Instant); 5] = [
        (vec![done("outer")], now),
        (
            vec![format!("5522;type=write:status=OK:pw={}", b64("outer")), done("outer")],
            now,
        ),
        (vec![ok("outer"), done("other"), done("outer")], now),
        (vec![ok("outer"), types("outer", &[0xff, 0xfe]), done("outer")], now),
        (vec![ok("outer"), done("outer")], now + DELEGATION_WAIT),
    ];
    for (case, (packets, done_at)) in broken.into_iter().enumerate() {
        let mut relay = supported();
        let mut fx = Effects::default();
        let last = packets.len() - 1;
        for (index, packet) in packets.iter().enumerate() {
            native(&mut relay, packet, if index == last { done_at } else { now }, &mut fx);
        }
        assert!(fx.outgoing.is_empty(), "{case}: {:?}", fx.outgoing);
    }

    let mut relay = supported();
    let mut fx = Effects::default();
    native(
        &mut relay,
        &format!("5522;type=read:status=OK:loc=primary:pw={}", b64("outer")),
        now,
        &mut fx,
    );
    native(&mut relay, &types("outer", b"text/plain image/png"), now, &mut fx);
    native(
        &mut relay,
        &format!("5522;type=read:status=EPERM:pw={}", b64("outer")),
        now,
        &mut fx,
    );
    native(&mut relay, &done("outer"), now, &mut fx);
    let [Outgoing::Paste(paste)] = fx.outgoing.as_slice() else {
        panic!("{:?}", fx.outgoing);
    };
    assert_eq!(paste.selection, "p");
    assert_eq!(paste.bridge.as_ref().unwrap().available, ["text/plain", "image/png"]);
}

#[test]
fn only_the_newest_32_native_pastes_can_be_fetched() {
    let now = Instant::now();
    let mut relay = supported();
    relay.reply_owner(true);
    let mut fx = Effects::default();
    for index in 0..33 {
        let token = b64(format!("outer-{index}"));
        native(
            &mut relay,
            &format!("5522;type=read:status=OK:pw={token}"),
            now,
            &mut fx,
        );
        native(
            &mut relay,
            &format!("5522;type=read:status=DONE:pw={token}"),
            now,
            &mut fx,
        );
    }
    let bridges: Vec<_> = fx
        .outgoing
        .iter()
        .map(|outgoing| match outgoing {
            Outgoing::Paste(paste) => paste.bridge.as_ref().unwrap().id.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(bridges.len(), 33);
    for (bridge, fetched) in [(&bridges[0], false), (&bridges[1], true)] {
        let mut request = mime_read(1);
        if let DelegatedRequest::ClipboardReadMime { bridge: id, .. } = &mut request.request {
            *id = Some(bridge.clone());
        }
        let mut fx = Effects::default();
        relay.delegated(request, now, &mut fx);
        assert_eq!(!fx.stdout.is_empty(), fetched);
        assert_eq!(
            matches!(fx.outgoing.as_slice(), [Outgoing::Answer(_, DelegatedReply::Denied)]),
            !fetched
        );
    }
}

#[test]
fn a_waiting_native_paste_never_shows_its_outer_token_in_debug_output() {
    let mut relay = supported();
    let mut fx = Effects::default();
    let token = b"outer-secret-token";
    native(
        &mut relay,
        &format!("5522;type=read:status=OK:pw={}", b64(token)),
        Instant::now(),
        &mut fx,
    );
    let shown = format!("{relay:?}");
    assert!(shown.contains("Paste { id: "), "{shown}");
    assert!(!shown.contains(&format!("{:?}", token.to_vec())), "{shown}");
    assert!(!shown.contains("outer-secret-token"), "{shown}");
}
