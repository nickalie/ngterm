use super::*;
use crate::{ClipboardContent, ClipboardPermissionScope, ClipboardReadRequest, ClipboardWrite, WriteCompletion};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ngterm_ghostty_vt::ClipboardWriteResult;

fn write(fixture: &Fixture) -> ClipboardWrite {
    let PaneEvent::ClipboardWrite(write) = fixture.event(|event| matches!(event, PaneEvent::ClipboardWrite(_))) else {
        unreachable!()
    };
    write
}
fn read(fixture: &Fixture) -> ClipboardReadRequest {
    let PaneEvent::ClipboardRead(read) = fixture.event(|event| matches!(event, PaneEvent::ClipboardRead(_))) else {
        unreachable!()
    };
    read
}
fn finish(fixture: &Fixture, write: &ClipboardWrite, status: ClipboardWriteResult, remember: bool) {
    fixture
        .pane
        .answer_clipboard_write(write.id, WriteCompletion { status, remember });
}
fn query(mimes: &str, pw: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name=VGVzdA==:pw={};{}\x1b\\",
        STANDARD.encode(pw),
        STANDARD.encode(mimes)
    )
    .into_bytes()
}
fn transaction(id: &str, pw: &str, contents: &[ClipboardContent]) -> Vec<u8> {
    let mut packets = vec![format!(
        "\x1b]5522;type=write:id={id}:name=VGVzdA==:pw={}\x1b\\",
        STANDARD.encode(pw)
    )];
    for content in contents {
        packets.push(format!(
            "\x1b]5522;type=wdata:mime={};{}\x1b\\",
            STANDARD.encode(&content.mime),
            STANDARD.encode(&content.data)
        ));
    }
    packets.push("\x1b]5522;type=wdata\x1b\\".into());
    packets.concat().into_bytes()
}
fn allow(fixture: &Fixture, read: &ClipboardReadRequest, contents: Vec<ClipboardContent>, remember: bool) {
    let available = contents.iter().map(|content| content.mime.clone()).collect();
    fixture.pane.answer_clipboard_read(
        read.id,
        ClipboardAnswer::Data {
            contents,
            available,
            remember,
        },
    );
}

#[test]
fn multipart_native_write_completion_controls_done_and_the_vt_lock_is_available() {
    let fixture = stream(PaneConfig::new(SIZE));
    let bytes = [
        include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt").as_slice(),
        b"after",
    ]
    .concat();
    fixture.feed.deliver(&bytes);
    let pending = write(&fixture);
    assert_eq!(pending.permission_scope, ClipboardPermissionScope::Kitty);
    assert!(pending.can_remember);
    assert!(!pending.granted);
    assert_eq!(pending.contents.len(), 4);
    for (part, expected) in pending.contents.iter().zip([
        include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/plain.txt").as_slice(),
        include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/html.txt").as_slice(),
        include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").as_slice(),
        include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin").as_slice(),
    ]) {
        assert_eq!(part.data, expected);
    }
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
    assert!(!fixture.pane.read_text(TextRegion::Viewport).unwrap().contains("after"));
    assert!(fixture.pane.shared().vt.try_lock().is_ok());
    finish(&fixture, &pending, ClipboardWriteResult::Success, true);
    let ack = fixture.sent_until(b"status=DONE");
    assert_eq!(ack, b"\x1b]5522;type=write:status=DONE\x1b\\");
    fixture.screen_until("after");
    finish(&fixture, &pending, ClipboardWriteResult::Denied, false);
    assert_eq!(fixture.out.next(Some(Duration::from_millis(10))), Ok(None));
}

#[test]
fn image_only_and_empty_writes_in_one_feed_keep_their_own_templates_and_statuses() {
    let fixture = stream(PaneConfig::new(SIZE));
    let contents = vec![ClipboardContent {
        mime: "image/png".into(),
        data: include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").to_vec(),
    }];
    fixture
        .feed
        .deliver(&[transaction("first", "", &contents), transaction("second", "", &[])].concat());
    let first = write(&fixture);
    assert_eq!(first.contents, contents);
    finish(&fixture, &first, ClipboardWriteResult::Success, true);
    assert_eq!(
        fixture.sent_until(b"id=first"),
        b"\x1b]5522;type=write:status=DONE:id=first\x1b\\"
    );
    let second = write(&fixture);
    assert!(second.contents.is_empty());
    assert!(!second.can_remember);
    finish(&fixture, &second, ClipboardWriteResult::Denied, false);
    assert_eq!(
        fixture.sent_until(b"id=second"),
        b"\x1b]5522;type=write:status=EPERM:id=second\x1b\\"
    );
}

#[test]
fn read_metadata_binary_contents_and_directional_grants_survive_the_core() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(&transaction("one", "token", &[]));
    let pending = write(&fixture);
    finish(&fixture, &pending, ClipboardWriteResult::Success, true);
    fixture.sent_until(b"status=DONE");
    fixture.feed.deliver(&query(
        "application/missing application/octet-stream image/png .",
        "token",
    ));
    let requested = read(&fixture);
    assert!(!requested.granted, "write grant does not grant reads");
    assert_eq!(requested.permission_scope, ClipboardPermissionScope::Kitty);
    assert_eq!(
        requested.mimes,
        ["application/missing", "application/octet-stream", "image/png"]
    );
    assert!(requested.list && requested.can_remember);
    let data: Vec<u8> = (0..=255).cycle().take(4097).collect();
    allow(
        &fixture,
        &requested,
        vec![ClipboardContent {
            mime: "application/octet-stream".into(),
            data: data.clone(),
        }],
        true,
    );
    let bytes = fixture.sent_until(b"status=DONE");
    let decoded: Vec<u8> = String::from_utf8(bytes)
        .unwrap()
        .split("\x1b]5522;")
        .skip(1)
        .filter(|packet| {
            packet.contains("status=DATA")
                && packet
                    .split_once(';')
                    .unwrap()
                    .0
                    .contains(&format!("mime={}", STANDARD.encode("application/octet-stream")))
        })
        .flat_map(|packet| {
            STANDARD
                .decode(packet.split_once(';').unwrap().1.strip_suffix("\x1b\\").unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(decoded, data);
    fixture.feed.deliver(&query("image/png", "token"));
    let requested = read(&fixture);
    assert!(requested.granted);
    allow(&fixture, &requested, Vec::new(), false);
    fixture.sent_until(b"status=DONE");
}

#[test]
fn a_large_write_consumes_a_read_only_paste_token_even_when_denied() {
    let fixture = stream(PaneConfig::new(SIZE));
    fixture.feed.deliver(b"\x1b[?5522hready");
    fixture.screen_until("ready");
    fixture
        .pane
        .paste_contents(
            &[
                ClipboardContent {
                    mime: "text/plain".into(),
                    data: b"clipboard".to_vec(),
                },
                ClipboardContent {
                    mime: "image/png".into(),
                    data: include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png").to_vec(),
                },
            ],
            crate::PasteSource::Clipboard,
            crate::ClipboardLocation::Standard,
            false,
        )
        .unwrap();
    let event = fixture.sent_until(b"status=DONE");
    let event = String::from_utf8(event).unwrap();
    assert!(event.contains(&STANDARD.encode("text/plain image/png\n")));
    let token = event
        .split("pw=")
        .nth(1)
        .unwrap()
        .split([':', ';', '\x1b'])
        .next()
        .unwrap();
    let token = String::from_utf8(STANDARD.decode(token).unwrap()).unwrap();
    fixture.feed.deliver(&query(".", &token));
    let listing = read(&fixture);
    assert!(!listing.granted && listing.mimes.is_empty());
    allow(&fixture, &listing, Vec::new(), false);
    fixture.sent_until(b"status=DONE");
    fixture.feed.deliver(&transaction(
        "large",
        &token,
        &[ClipboardContent {
            mime: "application/octet-stream".into(),
            data: vec![0; 8192],
        }],
    ));
    let pending = write(&fixture);
    assert!(!pending.granted);
    finish(&fixture, &pending, ClipboardWriteResult::Denied, false);
    fixture.sent_until(b"status=EPERM");
    fixture.feed.deliver(&query("text/plain", &token));
    let requested = read(&fixture);
    assert!(
        !requested.granted,
        "the denied write popped the one-time token in the callback"
    );
    allow(&fixture, &requested, Vec::new(), false);
}

#[test]
fn a_failed_paste_queue_leaves_the_grant_table_unchanged() {
    let fixture = stream(PaneConfig {
        write_limit: 4,
        ..PaneConfig::new(SIZE)
    });
    fixture.feed.deliver(b"\x1b[?5522hready");
    fixture.screen_until("ready");
    fixture.pane.write(b"full").unwrap();
    let grants = lock(&fixture.pane.shared().vt).terminal.clipboard_grants();
    for index in 0..32 {
        lock(&grants).grant(
            format!("grant-{index}").as_bytes(),
            ngterm_ghostty_vt::ClipboardDirection::Read,
            false,
        );
    }
    let before = format!("{:?}", lock(&grants));
    assert_eq!(
        fixture.pane.paste(
            "clipboard",
            crate::PasteSource::Clipboard,
            crate::ClipboardLocation::Standard,
            false
        ),
        Err(PaneError::Backpressure)
    );
    assert_eq!(format!("{:?}", lock(&grants)), before);
    assert!(
        lock(&grants).peek(b"grant-0", ngterm_ghostty_vt::ClipboardDirection::Read),
        "failed paste must not evict the oldest grant"
    );
}

#[test]
fn every_native_write_status_uses_the_matching_protocol_acknowledgement() {
    let fixture = stream(PaneConfig::new(SIZE));
    for (status, wire) in [
        (ClipboardWriteResult::Success, "DONE"),
        (ClipboardWriteResult::Denied, "EPERM"),
        (ClipboardWriteResult::Unsupported, "ENOSYS"),
        (ClipboardWriteResult::Busy, "EBUSY"),
        (ClipboardWriteResult::InvalidData, "EINVAL"),
        (ClipboardWriteResult::IoError, "EIO"),
    ] {
        fixture.feed.deliver(&transaction("status", "", &[]));
        let pending = write(&fixture);
        assert_eq!(fixture.out.next(Some(Duration::from_millis(1))), Ok(None));
        finish(&fixture, &pending, status, true);
        assert_eq!(
            fixture.sent_until(b"id=status"),
            format!("\x1b]5522;type=write:status={wire}:id=status\x1b\\").as_bytes()
        );
    }
}

#[test]
fn every_native_read_failure_reaches_the_program_as_its_status() {
    let fixture = stream(PaneConfig::new(SIZE));
    for (answer, wire) in [
        (ClipboardAnswer::Unsupported, "ENOSYS"),
        (ClipboardAnswer::Busy, "EBUSY"),
        (ClipboardAnswer::IoError, "EIO"),
        (ClipboardAnswer::Denied, "EPERM"),
    ] {
        fixture.feed.deliver(&query("text/plain", ""));
        let requested = read(&fixture);
        fixture.pane.answer_clipboard_read(requested.id, answer);
        let sent = fixture.sent_until(format!("status={wire}").as_bytes());
        assert!(!String::from_utf8(sent).unwrap().contains("DATA"));
    }
}
