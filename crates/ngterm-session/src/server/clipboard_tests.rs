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
        if let ClientEvent::Delegated { id, epoch, request } = event {
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
