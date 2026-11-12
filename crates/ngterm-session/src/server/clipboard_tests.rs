use super::tests::core;
use super::*;
use crate::protocol::{ClipboardRepresentation, ClipboardStatus, DelegatedRequest};
use crate::server::clients::ClientEvent;
use crate::server::replies::PARK_HIGH;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::time::{Duration, Instant};

fn delegated(attached: &Attached) -> (u64, u64, DelegatedRequest) {
    loop {
        let event = attached
            .subscription
            .recv_timeout(Duration::ZERO)
            .expect("the request must be queued");
        if let ClientEvent::Delegated { id, epoch, request, .. } = event {
            return (id, epoch, request);
        }
    }
}
fn read(types: &str, pw: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name=VGVzdA==:pw={};{}\x1b\\",
        STANDARD.encode(pw),
        STANDARD.encode(types)
    )
    .into_bytes()
}
fn write(id: &str, pw: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=write:id={id}:name=VGVzdA==:pw={}\x1b\\\x1b]5522;type=wdata\x1b\\",
        STANDARD.encode(pw)
    )
    .into_bytes()
}

fn assert_multipart(contents: &[ClipboardRepresentation]) {
    assert_eq!(contents.len(), 4);
    for (part, (mime, bytes)) in contents.iter().zip([
        (
            "text/plain",
            include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/plain.txt").as_slice(),
        ),
        (
            "text/html",
            include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/html.txt").as_slice(),
        ),
        (
            "image/png",
            include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").as_slice(),
        ),
        (
            "application/octet-stream",
            include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin").as_slice(),
        ),
    ]) {
        assert_eq!(part.mime, mime);
        assert_eq!(part.data, bytes);
    }
}

#[test]
fn clipboard_multipart_write_waits_for_one_owner_completion_and_keeps_binary_bytes() {
    let (core, owner) = core(ReplyMetadata::default());
    let other = core.lock().clients.attach(999);
    core.read(
        &[
            include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt").as_slice(),
            b"\x1b[5n",
        ]
        .concat(),
    );
    let (id, epoch, request) = delegated(&owner);
    let DelegatedRequest::ClipboardWrite {
        contents,
        can_remember,
        granted,
        kitty,
        ..
    } = request
    else {
        panic!("a write was expected");
    };
    assert!(can_remember && kitty && !granted);
    assert_multipart(&contents);
    assert!(core.outlet.take().is_empty(), "neither premature EPERM nor DONE");
    assert!(
        core.lock().recovery.since(0).is_empty(),
        "no recovery before native success"
    );
    while let Ok(event) = other.recv_timeout(Duration::ZERO) {
        assert!(
            !matches!(event, ClientEvent::Delegated { .. }),
            "only the owner is asked"
        );
    }
    let answer = Answer::from(DelegatedReply::ClipboardWrite {
        status: ClipboardStatus::Success,
        remember: true,
    });
    let mut state = core.lock();
    assert!(!state.answer(other.id(), id, epoch, answer.clone()));
    assert!(!state.answer(owner.subscription.id(), id, epoch + 1, answer.clone()));
    assert!(state.answer(owner.subscription.id(), id, epoch, answer.clone()));
    assert!(!state.answer(owner.subscription.id(), id, epoch, answer));
    drop(state);
    assert_eq!(
        core.outlet.take().concat(),
        b"\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n"
    );
    assert!(!core.lock().recovery.since(0).is_empty());
}

#[test]
fn clipboard_templates_are_separate_and_timeout_or_owner_change_denies_once() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(&[write("first", "token"), write("second", "token")].concat());
    let (id, epoch, _) = delegated(&owner);
    core.lock()
        .expire(Instant::now() + DELEGATION_WAIT + Duration::from_secs(1));
    assert_eq!(
        core.outlet.take().concat(),
        b"\x1b]5522;type=write:status=EPERM:id=first\x1b\\"
    );
    let (next, next_epoch, _) = delegated(&owner);
    assert_ne!(id, next);
    let mut state = core.lock();
    assert!(!state.answer(owner.subscription.id(), id, epoch, Answer::Denied));
    state.clients.detach(owner.subscription.id());
    let change = state.lead.detach(owner.subscription.id());
    state.tell(&change.notices);
    state.expire(Instant::now());
    assert!(!state.answer(owner.subscription.id(), next, next_epoch, Answer::Denied));
    drop(state);
    assert_eq!(
        core.outlet.take().concat(),
        b"\x1b]5522;type=write:status=EPERM:id=second\x1b\\"
    );
    assert!(core.outlet.take().is_empty());
    assert!(
        core.lock().recovery.since(0).is_empty(),
        "denied writes never enter recovery"
    );
}

#[test]
fn clipboard_recovery_keeps_the_last_success_when_a_later_write_fails() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(include_bytes!(
        "../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt"
    ));
    let (id, epoch, _) = delegated(&owner);
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::Write {
            status: ClipboardStatus::Success,
            remember: false
        },
    ));
    let accepted = core.lock().recovery.since(0);
    assert!(!accepted.is_empty());
    for status in [
        ClipboardStatus::Denied,
        ClipboardStatus::Unsupported,
        ClipboardStatus::Busy,
        ClipboardStatus::InvalidData,
        ClipboardStatus::IoError,
    ] {
        core.read(&write("failed-clear", ""));
        let (id, epoch, _) = delegated(&owner);
        assert_eq!(core.lock().recovery.since(0), accepted);
        assert!(core.lock().answer(
            owner.subscription.id(),
            id,
            epoch,
            Answer::Write {
                status,
                remember: false
            },
        ));
        assert_eq!(core.lock().recovery.since(0), accepted);
    }
    core.read(&write("accepted-clear", ""));
    let (id, epoch, _) = delegated(&owner);
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::Write {
            status: ClipboardStatus::Success,
            remember: false
        },
    ));
    assert_eq!(core.lock().recovery.since(0), b"\x1b]52;c;\x07");
}

#[test]
fn clipboard_binary_reads_preserve_requested_types_and_remember_only_the_read_direction() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(&read("application/missing application/octet-stream .", "token"));
    let (id, epoch, request) = delegated(&owner);
    assert!(
        matches!(request, DelegatedRequest::ClipboardReadMime { mimes, list: true, can_remember: true, granted: false, .. } if mimes == ["application/missing", "application/octet-stream"])
    );
    let data: Vec<u8> = (0..=255).cycle().take(4097).collect();
    let reply = DelegatedReply::ClipboardMime {
        contents: vec![ClipboardRepresentation {
            mime: "application/octet-stream".into(),
            data: data.clone(),
        }],
        available: vec!["application/octet-stream".into()],
        remember: true,
        status: ClipboardStatus::Success,
    };
    assert!(core.lock().answer(owner.subscription.id(), id, epoch, reply.into()));
    let output = String::from_utf8(core.outlet.take().concat()).unwrap();
    let data_packets: Vec<_> = output
        .split("\x1b]5522;")
        .skip(1)
        .filter(|packet| {
            packet.split_once(';').is_some_and(|(meta, _)| {
                meta.contains(&format!("mime={}", STANDARD.encode("application/octet-stream")))
            })
        })
        .collect();
    let bytes: Vec<_> = data_packets
        .into_iter()
        .flat_map(|packet| {
            STANDARD
                .decode(packet.split_once(';').unwrap().1.strip_suffix("\x1b\\").unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(bytes, data);
    core.read(&read("text/plain", "token"));
    let (id, epoch, request) = delegated(&owner);
    assert!(matches!(
        request,
        DelegatedRequest::ClipboardReadMime { granted: true, .. }
    ));
    assert!(core.lock().answer(owner.subscription.id(), id, epoch, Answer::Denied));
    core.outlet.take();
    core.read(&write("write", "token"));
    assert!(matches!(
        delegated(&owner).2,
        DelegatedRequest::ClipboardWrite { granted: false, .. }
    ));
}

#[test]
fn clipboard_printed_writes_are_delegated_without_acknowledging_the_program() {
    let (core, owner) = core(ReplyMetadata::default());
    let mut state = core.lock();
    let bytes = write("control", "");
    core.outlet.discard(true);
    state.feed(&bytes, true);
    core.outlet.discard(false);
    state.replies.observe_print(&bytes);
    state.delegate_printed_writes();
    drop(state);
    let (id, epoch, request) = delegated(&owner);
    assert!(matches!(request, DelegatedRequest::ClipboardWrite { .. }));
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::from(DelegatedReply::ClipboardWrite {
            status: ClipboardStatus::Success,
            remember: false
        })
    ));
    assert!(core.outlet.take().is_empty());
}

#[test]
fn clipboard_reads_wait_for_preceding_legacy_writes_and_retain_their_metadata() {
    for write in [b"\x1b]52;c;aGk=\x07".as_slice(), b"\x1b]1337;Copy=:aGk=\x07"] {
        for read in [b"\x1b]52;c;?\x07".to_vec(), read("text/plain .", "read-token")] {
            assert_read_after_write(write, &read);
        }
    }
}

fn assert_read_after_write(write: &[u8], read: &[u8]) {
    let (core, owner) = core(ReplyMetadata::default());
    let owner_id = owner.subscription.id();
    core.read(&[write, read, b"\x1b[5n"].concat());
    let (id, epoch, request) = delegated(&owner);
    assert!(matches!(request, DelegatedRequest::ClipboardWrite { contents, .. } if contents[0].data == b"hi"));
    assert!(
        owner.subscription.recv_timeout(Duration::ZERO).is_err(),
        "the read has not overtaken the write"
    );
    assert!(core.outlet.take().is_empty());
    assert!(core.lock().answer(
        owner_id,
        id,
        epoch,
        Answer::Write {
            status: ClipboardStatus::Success,
            remember: false
        }
    ));
    let (id, epoch, request) = delegated(&owner);
    let kitty = read.starts_with(b"\x1b]5522;");
    let answer = checked_read_answer(kitty, request);
    assert!(core.lock().answer(owner_id, id, epoch, answer));
    let replies = core.outlet.take().concat();
    assert!(replies.windows(4).any(|bytes| bytes == b"aGk="));
    assert!(replies.ends_with(b"\x1b[0n"), "later queries still follow the read");
    if kitty {
        let grants = core.lock().terminal.clipboard_grants();
        assert!(grants.lock().unwrap().peek(b"read-token", vt::ClipboardDirection::Read));
    }
}

fn checked_read_answer(kitty: bool, request: DelegatedRequest) -> Answer {
    if kitty {
        assert!(
            matches!(request, DelegatedRequest::ClipboardReadMime { mimes, list: true, name, can_remember: true, .. }
            if mimes == ["text/plain"] && name == "Test")
        );
        DelegatedReply::ClipboardMime {
            contents: vec![ClipboardRepresentation {
                mime: "text/plain".into(),
                data: b"hi".to_vec(),
            }],
            available: vec!["text/plain".into()],
            remember: true,
            status: ClipboardStatus::Success,
        }
        .into()
    } else {
        assert!(matches!(request, DelegatedRequest::ClipboardRead { selection, .. } if selection == "c"));
        Answer::Text(Some("hi".into()))
    }
}

#[test]
fn clipboard_writes_wait_for_an_overflowed_owner_to_resync() {
    let (core, owner) = core(ReplyMetadata::default());
    let owner_id = owner.subscription.id();
    // Two queued reads overflow even though an empty queue accepts a read larger than its limit.
    core.read(&vec![b'x'; super::super::clients::DEFAULT_QUEUE_LIMIT]);
    core.read(b"overflow");
    assert_eq!(core.lock().clients.overflowed(owner_id), Some(true));
    core.read(&[write("retained", "token"), b"after\x1b[5n".to_vec()].concat());
    assert!(core.outlet.take().is_empty(), "no EPERM while awaiting resync");
    core.read(b"later");
    assert!(core.outlet.take().is_empty(), "later output remains parked");
    {
        let mut state = core.lock();
        // The same snapshot/epoch transition and resume used by Pane::resync.
        let (offset, size) = (state.offset, state.size);
        state.clients.snapshot(owner_id, offset, size, Vec::new());
        state.resume_clipboards();
    }
    let (id, epoch, request) = delegated(&owner);
    assert!(matches!(request, DelegatedRequest::ClipboardWrite { .. }));
    assert!(epoch > 1);
    assert!(core.lock().answer(
        owner_id,
        id,
        epoch,
        Answer::Write {
            status: ClipboardStatus::Success,
            remember: false
        }
    ));
    assert_eq!(
        core.outlet.take().concat(),
        b"\x1b]5522;type=write:status=DONE:id=retained\x1b\\\x1b[0n"
    );
    assert_eq!(core.lock().recovery.since(0), b"\x1b]52;c;\x07");
}

fn drain_output(subscription: &super::super::clients::Subscription) -> Vec<u8> {
    let mut output = Vec::new();
    while let Ok(event) = subscription.try_recv() {
        match event {
            ClientEvent::Output { data, .. } => output.extend_from_slice(&data),
            ClientEvent::Delegated { .. } => panic!("a later request overtook the pending write"),
            _ => {}
        }
    }
    output
}

#[test]
fn clipboard_overflow_wait_expires_and_unblocks_every_viewer_and_the_pty() {
    for write in [write("stalled", "token"), b"\x1b]52;c;aGk=\x07".to_vec()] {
        assert_overflow_expires(&write);
    }
}

fn assert_overflow_expires(write: &[u8]) {
    let (core, owner) = core(ReplyMetadata::default());
    let owner_id = owner.subscription.id();
    let viewer = core.lock().clients.attach(999);
    core.read(&vec![b'x'; super::super::clients::DEFAULT_QUEUE_LIMIT]);
    drain_output(&viewer);
    core.read(b"overflow");
    drain_output(&viewer);
    assert_eq!(core.lock().clients.overflowed(owner_id), Some(true));
    core.read(write);
    let deadline = core
        .lock()
        .replies
        .deadline()
        .expect("overflow wait must have a deadline");
    drain_output(&viewer);
    core.read(&vec![b'y'; PARK_HIGH]);
    core.read(b"after\x1b[5n");
    assert!(core.pause.is_paused());
    assert!(drain_output(&viewer).is_empty());
    assert_eq!(
        core.lock().replies.deadline(),
        Some(deadline),
        "new output cannot extend the wait"
    );
    let mut state = core.lock();
    state.expire(deadline.checked_sub(Duration::from_nanos(1)).unwrap());
    assert!(core.outlet.take().is_empty());
    state.expire(deadline);
    assert_eq!(state.replies.parked(), 0);
    assert!(core.unpause(&state), "the helper can resume PTY reads");
    assert!(state.replies.deadline().is_none());
    assert!(
        state.recovery.since(0).is_empty(),
        "a timeout never records a successful write"
    );
    drop(state);
    assert_resumed_overflow(
        write.starts_with(b"\x1b]5522;"),
        &core.outlet.take().concat(),
        &drain_output(&viewer),
    );
    core.lock().expire(deadline + DELEGATION_WAIT);
    assert!(core.outlet.take().is_empty(), "denial is emitted only once");
}

fn assert_resumed_overflow(kitty: bool, replies: &[u8], output: &[u8]) {
    let expected: &[u8] = if kitty {
        b"\x1b]5522;type=write:status=EPERM:id=stalled\x1b\\\x1b[0n"
    } else {
        b"\x1b[0n"
    };
    assert_eq!(replies, expected);
    // Ghostty dispatches an OSC on ST's ESC; its backslash is parked too.
    let output = if kitty {
        output.strip_prefix(b"\\").expect("the rest of ST is resumed")
    } else {
        output
    };
    assert_eq!(output.len(), PARK_HIGH + b"after\x1b[5n".len());
    assert!(output[..PARK_HIGH].iter().all(|byte| *byte == b'y'));
    assert_eq!(&output[PARK_HIGH..], b"after\x1b[5n");
}

#[test]
fn clipboard_owner_pruned_at_write_dispatch_preserves_the_park_and_pending_request() {
    for first in [write("first", ""), b"\x1b]52;c;aGk=\x07".to_vec()] {
        for next in [write("second", ""), read("text/plain", "read-token")] {
            assert_owner_pruned_at_dispatch(&first, &next);
        }
    }
}

fn assert_owner_pruned_at_dispatch(write: &[u8], next: &[u8]) {
    let (core, mut owner) = core(ReplyMetadata::default());
    let other = {
        let mut state = core.lock();
        let (size, offset) = (state.size, state.offset);
        let change = state.lead.attach(
            999,
            size,
            Some(&Managed {
                nonce: "other".into(),
                claim: false,
            }),
        );
        let subscription = state.clients.attach(999);
        state.remember(999, ReplyMetadata::default());
        state.tell(&change.notices);
        state.clients.snapshot(999, offset, size, Vec::new());
        Attached {
            subscription,
            offset,
            size,
        }
    };
    // OSC dispatch is on BEL, or on the ESC of ST (before the backslash).
    let dispatch = write.len() - if write.ends_with(b"\x1b\\") { 2 } else { 1 };
    core.read(&write[..dispatch]);
    drain_output(&other.subscription);
    // Model the interval after Drop closes the queue but before its pruning hook acquires the pane lock.
    // With the prefix already fed, the next advance prunes inside the write's dispatch-byte feed.
    owner.subscription.on_drop(|_| {});
    drop(owner);
    core.read(&[&write[dispatch..], b"after", next, b"\x1b[5n"].concat());
    let (id, epoch, request) = delegated(&other);
    assert!(matches!(request, DelegatedRequest::ClipboardWrite { .. }));
    assert!(
        drain_output(&other.subscription).is_empty(),
        "all later output stays parked"
    );
    assert!(core.outlet.take().is_empty());
    assert!(core.lock().answer(
        999,
        id,
        epoch,
        Answer::Write {
            status: ClipboardStatus::Success,
            remember: false
        }
    ));
    let (next_id, next_epoch, request) = delegated(&other);
    assert_ne!(id, next_id);
    let answer = next_request_answer(request);
    assert!(core.lock().answer(999, next_id, next_epoch, answer));
    let replies = core.outlet.take().concat();
    if write.starts_with(b"\x1b]5522;") {
        assert!(replies.starts_with(b"\x1b]5522;type=write:status=DONE:id=first\x1b\\"));
    }
    assert!(replies.ends_with(b"\x1b[0n"));
    assert_eq!(
        core.lock().recovery.since(0),
        if write.starts_with(b"\x1b]5522;") || next.starts_with(b"\x1b]5522;type=write") {
            b"\x1b]52;c;\x07".as_slice()
        } else {
            b"\x1b]52;c;aGk=\x07"
        }
    );
}

fn next_request_answer(request: DelegatedRequest) -> Answer {
    match request {
        DelegatedRequest::ClipboardWrite { .. } => Answer::Write {
            status: ClipboardStatus::Success,
            remember: false,
        },
        DelegatedRequest::ClipboardReadMime { mimes, .. } => {
            assert_eq!(mimes, ["text/plain"]);
            Answer::Text(Some("hi".into()))
        }
        DelegatedRequest::ClipboardRead { .. } => panic!("the saved next request must be delegated"),
    }
}

fn lazy_paste(core: &Arc<Core>, owner: &Attached) -> Vec<u8> {
    use crate::protocol::ClipboardPasteBridge;
    let bridge = ClipboardPasteBridge {
        id: "lazy-source".into(),
        available: (0..16).map(|index| format!("application/type{index}")).collect(),
    };
    let contents: Vec<_> = bridge
        .available
        .iter()
        .map(|mime| vt::ClipboardContent {
            mime: mime.clone(),
            data: Vec::new(),
        })
        .collect();
    let mut state = core.lock();
    let epoch = state.clients.epoch(owner.subscription.id()).unwrap();
    let bytes = state.paste_bytes(&contents, false, "c", false).unwrap();
    state.register_paste_bridge(owner.subscription.id(), epoch, Some(&bridge), &bytes);
    state.terminal.clipboard_grants().lock().unwrap().pasted(&bytes);
    let end = bytes.windows(2).position(|part| part == b"\x1b\\").unwrap();
    crate::attach_tty::clipboard::Packet::parse(&bytes[2..end])
        .unwrap()
        .token
}
fn token_read(types: &str, token: &[u8]) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name=VGVzdA==:pw={};{}\x1b\\",
        STANDARD.encode(token),
        STANDARD.encode(types)
    )
    .into_bytes()
}
#[test]
fn lazy_clipboard_listings_preserve_the_server_token_and_a_later_type_is_fetched_once() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(b"\x1b[?5522h");
    let token = lazy_paste(&core, &owner);
    core.read(&token_read(".", &token));
    let bytes = core.outlet.take().concat();
    assert!(
        String::from_utf8_lossy(&bytes).contains(
            &STANDARD.encode(
                (0..16)
                    .map(|index| format!("application/type{index}"))
                    .collect::<Vec<_>>()
                    .join(" ")
                    + "\n"
            )
        )
    );
    assert!(
        core.lock()
            .terminal
            .clipboard_grants()
            .lock()
            .unwrap()
            .peek(&token, vt::ClipboardDirection::Read)
    );
    core.read(&token_read("application/type12", &token));
    let (id, epoch, request) = delegated(&owner);
    assert!(
        matches!(request, DelegatedRequest::ClipboardReadMime { bridge: Some(bridge), mimes, granted: true, .. } if bridge == "lazy-source" && mimes == ["application/type12"])
    );
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::Mime {
            contents: vec![ClipboardRepresentation {
                mime: "application/type12".into(),
                data: vec![0, 12, 255]
            }],
            available: Vec::new(),
            status: ClipboardStatus::Success,
            remember: false,
        }
    ));
    assert!(String::from_utf8_lossy(&core.outlet.take().concat()).contains(&STANDARD.encode([0, 12, 255])));
    core.read(&token_read("application/type12", &token));
    assert!(String::from_utf8_lossy(&core.outlet.take().concat()).contains("EPERM"));
    assert!(!drain_output(&owner.subscription).is_empty());
}
#[test]
fn lazy_clipboard_tokens_are_terminal_scoped_and_invalidated_by_reset_or_owner_loss() {
    for reset in [false, true] {
        let (first, owner) = core(ReplyMetadata::default());
        first.read(b"\x1b[?5522h");
        let token = lazy_paste(&first, &owner);
        let (other, other_owner) = core(ReplyMetadata::default());
        other.read(&token_read("application/type12", &token));
        assert!(matches!(
            delegated(&other_owner).2,
            DelegatedRequest::ClipboardReadMime {
                bridge: None,
                granted: false,
                ..
            }
        ));
        if reset {
            first.read(b"\x1bc");
        } else {
            let mut state = first.lock();
            let change = state.lead.detach(owner.subscription.id());
            state.tell(&change.notices);
            state.refresh_owner();
        }
        first.read(&token_read("application/type12", &token));
        assert!(String::from_utf8_lossy(&first.outlet.take().concat()).contains("EPERM"));
    }
}

#[test]
fn evicting_a_lazy_paste_revokes_its_grant_after_an_earlier_token_was_consumed() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(b"\x1b[?5522h");
    let tokens: Vec<_> = (0..32).map(|_| lazy_paste(&core, &owner)).collect();
    core.read(&token_read("application/type12", &tokens[0]));
    let (id, epoch, _) = delegated(&owner);
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::Mime {
            contents: Vec::new(),
            available: Vec::new(),
            remember: false,
            status: ClipboardStatus::Success,
        }
    ));
    lazy_paste(&core, &owner);
    lazy_paste(&core, &owner);
    core.read(&token_read("application/type12", &tokens[1]));
    assert!(matches!(
        delegated(&owner).2,
        DelegatedRequest::ClipboardReadMime {
            bridge: None,
            granted: false,
            ..
        }
    ));
}
fn alias_delegation(owner: &Attached) -> (u64, u64, DelegatedRequest, Option<crate::protocol::ClipboardAlias>) {
    loop {
        let event = owner.subscription.recv_timeout(Duration::ZERO).unwrap();
        if let ClientEvent::Delegated {
            id,
            epoch,
            request,
            native_alias,
        } = event
        {
            return (id, epoch, request, native_alias);
        }
    }
}
#[test]
fn clipboard_grants_and_native_aliases_survive_owner_switch_and_aliases_rotate_on_ris() {
    let (core, owner) = core(ReplyMetadata::default());
    core.read(&read("image/png", "program-token"));
    let (id, epoch, _, alias) = alias_delegation(&owner);
    assert!(alias.is_some());
    assert!(core.lock().answer(
        owner.subscription.id(),
        id,
        epoch,
        Answer::Mime {
            contents: Vec::new(),
            available: Vec::new(),
            remember: true,
            status: ClipboardStatus::Success,
        }
    ));
    core.outlet.take();
    let other = {
        let mut state = core.lock();
        let size = state.size;
        let offset = state.offset;
        let change = state.lead.attach(
            999,
            size,
            Some(&Managed {
                nonce: "other".into(),
                claim: true,
            }),
        );
        let subscription = state.clients.attach(999);
        state.remember(999, ReplyMetadata::default());
        state.tell(&change.notices);
        state.refresh_owner();
        state.clients.snapshot(999, offset, size, Vec::new());
        Attached {
            subscription,
            offset,
            size,
        }
    };
    core.read(&read("image/png", "program-token"));
    let (id, epoch, request, other_alias) = alias_delegation(&other);
    assert_eq!(alias, other_alias);
    assert!(matches!(
        request,
        DelegatedRequest::ClipboardReadMime { granted: true, .. }
    ));
    core.lock().answer(other.subscription.id(), id, epoch, Answer::Denied);
    core.read(b"\x1bc");
    core.read(&read("image/png", "program-token"));
    let (_, _, request, reset_alias) = alias_delegation(&other);
    assert_ne!(alias, reset_alias);
    assert!(matches!(
        request,
        DelegatedRequest::ClipboardReadMime { granted: false, .. }
    ));
    assert_eq!(format!("{:?}", alias.unwrap()), "ClipboardAlias { .. }");
}

#[test]
fn pastes_accept_followers_only_in_their_current_epoch() {
    let (core, owner) = core(ReplyMetadata::default());
    let mut state = core.lock();
    let other = state.clients.attach(999);
    let size = state.size;
    state.clients.snapshot(999, 0, size, Vec::new());
    let epoch = state.clients.epoch(999).unwrap();
    assert!(state.paste_client(999, epoch));
    assert!(!state.paste_client(999, epoch + 1));
    assert!(!state.paste_client(998, epoch));
    let owner = owner.subscription.id();
    assert!(state.paste_client(owner, state.clients.epoch(owner).unwrap()));
    drop(other);
}

fn follower(core: &Arc<Core>) -> Attached {
    let mut state = core.lock();
    let size = state.size;
    let offset = state.offset;
    let subscription = state.clients.attach(999);
    state.clients.snapshot(999, offset, size, Vec::new());
    Attached {
        subscription,
        offset,
        size,
    }
}

fn claim(core: &Arc<Core>, attached: &Attached) {
    let mut state = core.lock();
    let size = state.size;
    let change = state.lead.attach(
        attached.subscription.id(),
        size,
        Some(&Managed {
            nonce: "new-owner".into(),
            claim: true,
        }),
    );
    state.tell(&change.notices);
}

fn no_delegation(attached: &Attached) {
    while let Ok(event) = attached.subscription.recv_timeout(Duration::ZERO) {
        assert!(
            !matches!(event, ClientEvent::Delegated { .. }),
            "unexpected clipboard read: {event:?}"
        );
    }
}

fn binary_answer(data: &[u8]) -> Answer {
    Answer::Mime {
        contents: vec![ClipboardRepresentation {
            mime: "application/type12".into(),
            data: data.to_vec(),
        }],
        available: Vec::new(),
        remember: false,
        status: ClipboardStatus::Success,
    }
}

#[test]
fn follower_lazy_paste_lists_sixteen_types_and_reads_only_its_own_clipboard() {
    let (core, owner) = core(ReplyMetadata::default());
    let source = follower(&core);
    core.read(b"\x1b[?5522h");
    let token = lazy_paste(&core, &source);
    core.read(&token_read(".", &token));
    let listed = core.outlet.take().concat();
    assert!(
        String::from_utf8_lossy(&listed).contains(
            &STANDARD.encode(
                (0..16)
                    .map(|index| format!("application/type{index}"))
                    .collect::<Vec<_>>()
                    .join(" ")
                    + "\n"
            )
        )
    );
    no_delegation(&owner);
    no_delegation(&source);
    core.read(&token_read("application/type12", &token));
    let (id, epoch, request) = delegated(&source);
    assert!(
        matches!(request, DelegatedRequest::ClipboardReadMime { bridge: Some(bridge), mimes, granted: true, .. }
        if bridge == "lazy-source" && mimes == ["application/type12"])
    );
    no_delegation(&owner);
    assert!(
        !core
            .lock()
            .answer(owner.subscription.id(), id, epoch, binary_answer(b"owner clipboard"))
    );
    assert!(core.lock().answer(
        source.subscription.id(),
        id,
        epoch,
        binary_answer(b"follower\0\xffclipboard")
    ));
    let response = String::from_utf8(core.outlet.take().concat()).unwrap();
    assert!(response.contains(&STANDARD.encode(b"follower\0\xffclipboard")));
    assert!(!response.contains(&STANDARD.encode(b"owner clipboard")));
    core.read(&token_read("application/type12", &token));
    assert!(String::from_utf8_lossy(&core.outlet.take().concat()).contains("EPERM"));
    core.read(&read("text/plain", "ordinary"));
    let (id, epoch, request) = delegated(&owner);
    assert!(matches!(
        request,
        DelegatedRequest::ClipboardReadMime {
            bridge: None,
            granted: false,
            ..
        }
    ));
    no_delegation(&source);
    assert!(core.lock().answer(owner.subscription.id(), id, epoch, Answer::Denied));
}

#[test]
fn follower_lazy_sources_fail_closed_on_epoch_owner_change_disconnect_and_overflow() {
    for invalidation in 0..4 {
        let (core, owner) = core(ReplyMetadata::default());
        let source = follower(&core);
        core.read(b"\x1b[?5522h");
        let token = lazy_paste(&core, &source);
        match invalidation {
            0 => {
                let mut state = core.lock();
                let size = state.size;
                state.clients.snapshot(source.subscription.id(), 0, size, Vec::new());
            }
            1 => claim(&core, &source),
            2 => {
                core.lock().clients.detach(source.subscription.id());
            }
            _ => {
                let mut state = core.lock();
                state
                    .clients
                    .output(0, &vec![b'x'; super::super::clients::DEFAULT_QUEUE_LIMIT]);
                state.clients.output(0, b"overflow");
                assert_eq!(state.clients.overflowed(source.subscription.id()), Some(true));
            }
        }
        core.read(&token_read("application/type12", &token));
        assert!(
            String::from_utf8_lossy(&core.outlet.take().concat()).contains("EPERM"),
            "{invalidation}"
        );
        no_delegation(&owner);
        no_delegation(&source);
    }
}

#[test]
fn a_follower_fetch_denial_timeout_or_source_loss_never_retries_as_input_or_owner_read() {
    for failure in 0..3 {
        let (core, owner) = core(ReplyMetadata::default());
        let source = follower(&core);
        core.read(b"\x1b[?5522h");
        let token = lazy_paste(&core, &source);
        core.read(&token_read("application/type12", &token));
        let (id, epoch, _) = delegated(&source);
        if failure == 0 {
            assert!(core.lock().answer(source.subscription.id(), id, epoch, Answer::Denied));
        } else {
            let mut state = core.lock();
            if failure == 2 {
                state.clients.detach(source.subscription.id());
            }
            state.expire(Instant::now() + DELEGATION_WAIT + Duration::from_secs(1));
        }
        assert!(
            !core
                .lock()
                .answer(source.subscription.id(), id, epoch, binary_answer(b"late"))
        );
        assert_eq!(core.outlet.take().concat(), b"\x1b]5522;type=read:status=EPERM\x1b\\");
        core.read(&token_read("application/type12", &token));
        assert_eq!(core.outlet.take().concat(), b"\x1b]5522;type=read:status=EPERM\x1b\\");
        no_delegation(&owner);
    }
}

#[test]
fn a_dropped_follower_cancels_its_pending_fetch_without_waiting_for_the_deadline() {
    let (core, owner) = core(ReplyMetadata::default());
    let source = follower(&core);
    core.read(b"\x1b[?5522h");
    let token = lazy_paste(&core, &source);
    core.read(&token_read("application/type12", &token));
    let (id, epoch, _) = delegated(&source);
    let source_id = source.subscription.id();
    no_delegation(&owner);
    core.read(&vec![b'x'; PARK_HIGH + 1]);
    core.read(b"after-fetch\x1b[5n");
    assert!(core.pause.is_paused());
    drop(source);
    let mut state = core.lock();
    state.prune();
    assert_eq!(state.replies.owner(), Some(owner.subscription.id()));
    state.expire(Instant::now()); // The helper handles cancellation now, not at the 30-second deadline.
    assert!(!state.answer(source_id, id, epoch, binary_answer(b"late")));
    assert!(core.unpause(&state), "the stalled PTY must resume immediately");
    drop(state);
    assert_eq!(
        core.outlet.take().concat(),
        b"\x1b]5522;type=read:status=EPERM\x1b\\\x1b[0n"
    );
    let mut output = Vec::new();
    while let Ok(event) = owner.subscription.recv_timeout(Duration::ZERO) {
        if let ClientEvent::Output { data, .. } = event {
            output.extend_from_slice(&data);
        }
    }
    assert!(output.windows(b"after-fetch".len()).any(|part| part == b"after-fetch"));
}

fn owner_content_token(core: &Arc<Core>, owner: &Attached) -> Vec<u8> {
    let mut state = core.lock();
    let bytes = state
        .paste_bytes(
            &[vt::ClipboardContent {
                mime: "text/plain".into(),
                data: b"owner text".to_vec(),
            }],
            false,
            "c",
            false,
        )
        .unwrap();
    let epoch = state.clients.epoch(owner.subscription.id()).unwrap();
    state.register_paste_bridge(owner.subscription.id(), epoch, None, &bytes);
    state.terminal.clipboard_grants().lock().unwrap().pasted(&bytes);
    let end = bytes.windows(2).position(|pair| pair == b"\x1b\\").unwrap();
    crate::attach_tty::clipboard::Packet::parse(&bytes[2..end])
        .unwrap()
        .token
}

#[test]
fn owner_content_paste_tokens_list_without_consumption_and_never_read_a_replacement_owner() {
    for change_owner in [false, true] {
        let (core, owner) = core(ReplyMetadata::default());
        let other = follower(&core);
        core.read(b"\x1b[?5522h");
        let token = owner_content_token(&core, &owner);
        core.read(&token_read(".", &token));
        let (id, epoch, request) = delegated(&owner);
        assert!(matches!(request, DelegatedRequest::ClipboardReadMime { bridge: None, mimes, .. } if mimes.is_empty()));
        assert!(core.lock().answer(
            owner.subscription.id(),
            id,
            epoch,
            Answer::Mime {
                contents: Vec::new(),
                available: vec!["text/plain".into()],
                remember: false,
                status: ClipboardStatus::Success,
            }
        ));
        core.outlet.take();
        if change_owner {
            claim(&core, &other);
        }
        core.read(&token_read("text/plain", &token));
        if change_owner {
            assert!(String::from_utf8_lossy(&core.outlet.take().concat()).contains("EPERM"));
            no_delegation(&owner);
            no_delegation(&other);
        } else {
            let (id, epoch, request) = delegated(&owner);
            assert!(matches!(
                request,
                DelegatedRequest::ClipboardReadMime {
                    bridge: None,
                    granted: true,
                    ..
                }
            ));
            assert!(core.lock().answer(owner.subscription.id(), id, epoch, Answer::Denied));
        }
    }
}
