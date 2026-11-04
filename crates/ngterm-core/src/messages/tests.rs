//! Address and bus regressions ported from upstream's `MessageAddressTests` and `MessageBusTests`.

use super::*;

fn recipient(pane: Uuid) -> MessageRecipient {
    MessageRecipient::new(Uuid::nil(), pane)
}

fn send(bus: &MessageBus, body: &str, panes: &[Uuid]) -> ControlResult {
    bus.send(
        body,
        Some("S:sender"),
        &panes.iter().copied().map(recipient).collect::<Vec<_>>(),
        1.0,
    )
    .unwrap()
}

#[test]
fn recipient_ids_roles_and_case_and_identity_refusals() {
    let pane = Uuid::new_v4();
    assert_eq!(
        MessageAddress::parse(&format!("S:{}", pane.to_string().to_lowercase()), true)
            .unwrap()
            .pane,
        MessagePane::Id(pane)
    );
    for (name, role) in [
        ("left", StatusPane::Left),
        ("primary", StatusPane::Left),
        ("right", StatusPane::Right),
        ("scratch", StatusPane::Scratch),
    ] {
        assert_eq!(
            MessageAddress::parse(&format!("S:{name}"), false).unwrap().pane,
            MessagePane::Role(role)
        );
        assert_eq!(
            MessageAddress::parse(&format!("S:{name}"), true)
                .unwrap_err()
                .to_string(),
            format!("identity \"S:{name}\" must name a pane id, not a role")
        );
    }
    assert_eq!(
        recipient(pane).address,
        format!("{}:{}", uuid_string(Uuid::nil()), uuid_string(pane))
    );
}

#[test]
fn malformed_addresses_name_the_original_input() {
    for text in [
        ":left",
        "S:",
        "S:nope",
        "S:left:extra",
        "",
        "S:00000000000040008000000000000000",
    ] {
        assert_eq!(
            MessageAddress::parse(text, false).unwrap_err().to_string(),
            format!("invalid address \"{text}\": expected SESSION:PANE")
        );
    }
    for identity in [false, true] {
        assert_eq!(
            MessageAddress::parse("S", identity).unwrap_err().to_string(),
            "address \"S\" names a session, not a pane"
        );
    }
}

#[test]
fn oldest_first_metadata_repeat_reads_and_exact_ack() {
    let bus = MessageBus::default();
    let pane = Uuid::new_v4();
    let first = send(&bus, "one", &[pane]).message_id.unwrap();
    send(&bus, "two", &[pane]);
    let read = bus.read(pane, 1, false);
    assert_eq!(read, bus.read(pane, 1, false));
    assert_eq!(read.remaining, Some(1));
    assert!(read.acked.is_none());
    let message = &read.messages.unwrap()[0];
    assert_eq!(
        (
            message.id.as_str(),
            message.from.as_deref(),
            message.to.as_str(),
            message.ts,
            message.body.as_str()
        ),
        (
            first.as_str(),
            Some("S:sender"),
            recipient(pane).address.as_str(),
            1.0,
            "one"
        )
    );
    let id = Uuid::parse_str(&first).unwrap();
    assert_eq!(bus.ack(pane, &[id, Uuid::new_v4(), id]), [first]);
    assert!(bus.ack(pane, &[id]).is_empty());
    assert_eq!(bus.read(pane, 32, false).messages.unwrap()[0].body, "two");
    let taken = bus.read(pane, 1, true);
    assert_eq!(
        taken.acked.unwrap(),
        taken.messages.unwrap().iter().map(|m| m.id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(bus.pending(pane), 0);
    assert!(bus.read(Uuid::new_v4(), 32, true).messages.unwrap().is_empty());
}

#[test]
fn bounded_atomic_read_ack_keeps_the_unreturned_remainder() {
    let bus = MessageBus::default();
    let pane = Uuid::new_v4();
    for i in 0..MAX_BATCH + 8 {
        send(&bus, &format!("m{i}"), &[pane]);
    }
    let read = bus.read(pane, 100, false);
    assert_eq!(read.messages.as_ref().unwrap().len(), MAX_BATCH);
    assert_eq!(read.remaining, Some(8));
    let taken = bus.read(pane, 100, true);
    assert_eq!(read.messages, taken.messages);
    assert_eq!(taken.remaining, Some(8));
    assert_eq!(
        bus.read(pane, 100, false)
            .messages
            .unwrap()
            .iter()
            .map(|m| m.body.clone())
            .collect::<Vec<_>>(),
        (MAX_BATCH..MAX_BATCH + 8).map(|i| format!("m{i}")).collect::<Vec<_>>()
    );
}

#[test]
fn ack_and_dispose_are_pane_scoped_and_share_one_send_id() {
    let bus = MessageBus::default();
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let id = send(&bus, "both", &[a, b]).message_id.unwrap();
    assert_eq!(bus.read(b, 32, false).messages.unwrap()[0].id, id);
    bus.ack(a, &[Uuid::parse_str(&id).unwrap()]);
    assert_eq!((bus.pending(a), bus.pending(b)), (0, 1));
    send(&bus, "more", &[a, b]);
    bus.dispose(a);
    assert_eq!((bus.pending(a), bus.pending(b)), (0, 2));
    let shared = bus.clone();
    shared.dispose(b);
    assert_eq!(bus.pending(b), 0);
}

#[test]
fn mailbox_count_and_utf8_byte_caps_refuse_only_the_full_recipient() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let bus = MessageBus::default();
    for _ in 0..MAX_MESSAGES {
        send(&bus, "x", &[a]);
    }
    let sent = send(&bus, "late", &[a, b]);
    assert_eq!(sent.delivered.unwrap(), [recipient(b).address]);
    assert_eq!(
        sent.rejected.unwrap(),
        [MessageRejection {
            address: recipient(a).address,
            reason: MessageRejectionReason::MailboxFull
        }]
    );
    assert_eq!((bus.pending(a), bus.pending(b)), (MAX_MESSAGES, 1));
    bus.dispose(a);
    let chunk = "é".repeat(MAX_BODY_BYTES / 2);
    for _ in 0..MAX_MAILBOX_BYTES / MAX_BODY_BYTES {
        send(&bus, &chunk, &[a]);
    }
    assert_eq!(
        send(&bus, "b", &[a]).rejected.unwrap()[0].reason,
        MessageRejectionReason::MailboxFull
    );
    assert!(bus.send(&(chunk + "é"), None, &[recipient(b)], 1.0).is_err());
    assert_eq!(bus.pending(b), 1);
    assert!(send(&bus, "empty", &[]).delivered.unwrap().is_empty());
}
