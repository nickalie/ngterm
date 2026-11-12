//! A shown pane's clipboard reads and writes through the whole core (OSC 52 and the Kitty protocol's OSC 5522): the
//! events the app gets, the answers it gives, and what the program reads back down the channel.

use std::collections::BTreeMap;
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::pane_tests::{Fixture, QUIET, contains, inputs_until, position, reference, send};
use crate::api::{MobileEvent, PaneInputError};
use crate::pane::{ClipboardPart, ClipboardReply, ClipboardWriteStatus, PaneClipboard, PaneRef};
use crate::transport::{ChannelInput, ChannelPeer};

const MULTIPART: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt");
const PLAIN: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/plain.txt");
const HTML: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/html.txt");
const RED_PNG: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
const ARBITRARY: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin");

fn part(mime: &str, data: &[u8]) -> ClipboardPart {
    ClipboardPart {
        mime: mime.to_owned(),
        data: data.to_vec(),
    }
}

fn answer(fixture: &Fixture, read_id: u64, reply: ClipboardReply) {
    let PaneRef {
        machine_id,
        session_id,
        pane_id,
    } = reference();
    fixture
        .core
        .answer_clipboard_read(machine_id, session_id, pane_id, read_id, reply)
        .unwrap();
}

fn complete(fixture: &Fixture, write_id: u64, status: ClipboardWriteStatus) {
    let PaneRef {
        machine_id,
        session_id,
        pane_id,
    } = reference();
    fixture
        .core
        .complete_clipboard_write(machine_id, session_id, pane_id, write_id, status)
        .unwrap();
}

/// The next clipboard write the app is asked: its id and contents.
fn written(fixture: &Fixture) -> (u64, Vec<ClipboardPart>) {
    fixture.next(|event| match event {
        MobileEvent::ClipboardWrite {
            pane,
            write_id,
            contents,
        } if pane == reference() => Some((write_id, contents)),
        _ => None,
    })
}

/// A clipboard read as the app is asked it.
#[derive(Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the event's own four flags, compared as the app gets them"
)]
struct Asked {
    read_id: u64,
    clipboard: PaneClipboard,
    list_only: bool,
    mimes: Vec<String>,
    list: bool,
    granted: bool,
    can_remember: bool,
}

fn asked(fixture: &Fixture) -> Asked {
    fixture.next(|event| match event {
        MobileEvent::ClipboardReadRequest {
            pane,
            read_id,
            clipboard,
            list_only,
            mimes,
            list,
            granted,
            can_remember,
        } if pane == reference() => Some(Asked {
            read_id,
            clipboard,
            list_only,
            mimes,
            list,
            granted,
            can_remember,
        }),
        _ => None,
    })
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// A Kitty read of the space-separated `mimes` under the password `pw`.
fn kitty_read(mimes: &str, pw: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name=VGVzdA==:pw={};{}\x1b\\",
        STANDARD.encode(pw),
        STANDARD.encode(mimes)
    )
    .into_bytes()
}

/// A Kitty read's answer, from its first packet to its DONE: each DATA packet's payload by its MIME type (joined
/// across chunks), and the last status.
fn read_answer(peer: &mut ChannelPeer) -> (BTreeMap<String, Vec<u8>>, String) {
    let (bytes, _) = inputs_until(peer, |bytes, _| {
        contains(bytes, b"status=DONE") || contains(bytes, b"status=EPERM")
    });
    let text = String::from_utf8(bytes).unwrap();
    let mut data: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut status = String::new();
    for packet in text.split("\x1b]5522;").skip(1) {
        let packet = packet.split("\x1b\\").next().unwrap();
        let (metadata, payload) = packet.split_once(';').unwrap_or((packet, ""));
        let field = |key: &str| {
            metadata
                .split(':')
                .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_owned)
        };
        status = field("status").unwrap_or_default();
        if status == "DATA" {
            let mime = String::from_utf8(STANDARD.decode(field("mime").unwrap()).unwrap()).unwrap();
            data.entry(mime).or_default().extend(STANDARD.decode(payload).unwrap());
        }
    }
    (data, status)
}

/// A program's write ack: everything it was sent up to the answer of a status report asked after it.
fn write_status(peer: &mut ChannelPeer) -> Vec<u8> {
    send(peer, b"\x1b[5n");
    let (bytes, _) = inputs_until(peer, |bytes, _| contains(bytes, b"\x1b[0n"));
    bytes
}

/// An OSC 52 read's request event: its id, after the read's bytes are delivered.
fn osc52_read(fixture: &Fixture, peer: &ChannelPeer) -> u64 {
    send(peer, b"\x1b]52;c;?\x07");
    let asked = asked(fixture);
    assert_eq!(
        asked,
        Asked {
            read_id: asked.read_id,
            clipboard: PaneClipboard::Standard,
            list_only: false,
            mimes: names(&["text/plain"]),
            list: false,
            granted: false,
            can_remember: false,
        },
        "OSC 52 asks for text, with no grant to keep"
    );
    asked.read_id
}

#[test]
fn a_clipboard_read_holds_the_output_until_allowed_or_denied_and_detaching_denies_it() {
    let mut fixture = Fixture::connected();
    let pane = fixture.show();
    let mut opened = fixture.attach();

    let first = osc52_read(&fixture, &opened.peer);
    send(&opened.peer, b"\x1b[6n");
    thread::sleep(QUIET);
    assert!(opened.peer.try_input().is_none(), "the output waits for the answer");
    answer(
        &fixture,
        first,
        ClipboardReply::Allow {
            contents: vec![part("text/plain", b"hi")],
            available: Vec::new(),
            remember: true,
        },
    );
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"R"));
    assert!(
        contains(&bytes, b"aGk="),
        "the text goes back: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        position(&bytes, b"aGk=") < position(&bytes, b"\x1b[1;1R"),
        "then the held output runs"
    );

    let second = osc52_read(&fixture, &opened.peer);
    assert_ne!(second, first);
    send(&opened.peer, b"\x1b[6n");
    answer(&fixture, second, ClipboardReply::Deny);
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"R"));
    assert!(contains(&bytes, b"]52;"), "an empty clipboard goes back");
    assert!(!contains(&bytes, b"aGk="));

    let third = osc52_read(&fixture, &opened.peer);
    fixture
        .core
        .detach_session(
            super::pane_tests::MACHINE.to_owned(),
            super::pane_tests::SESSION.to_owned(),
        )
        .unwrap();
    let dismissed = fixture.next(|event| match event {
        MobileEvent::ClipboardReadDismiss { read_id, .. } => Some(read_id),
        _ => None,
    });
    assert_eq!(dismissed, third);
    let (bytes, others) = inputs_until(&mut opened.peer, |_, others| others.contains(&ChannelInput::Close));
    assert!(
        contains(&bytes, b"]52;"),
        "the read is denied before the channel closes"
    );
    assert_eq!(others, [ChannelInput::Close]);
    assert_eq!(
        pane.type_text("x".to_owned()),
        Err(PaneInputError::Closed),
        "the handle is dead"
    );
    assert!(fixture.quiet().is_empty(), "nothing more about the pane");
}

#[test]
fn the_multipart_write_reaches_the_app_byte_exact_and_completes_with_its_status() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();

    send(&opened.peer, &[MULTIPART, b"\x1b[5n"].concat());
    let (write_id, contents) = written(&fixture);
    assert_eq!(
        contents,
        [
            part("text/plain", PLAIN),
            part("text/html", HTML),
            part("image/png", RED_PNG),
            part("application/octet-stream", ARBITRARY),
        ]
    );
    assert_eq!(RED_PNG.len(), 73);
    assert_eq!(ARBITRARY.len(), 2048);
    thread::sleep(QUIET);
    assert!(
        opened.peer.try_input().is_none(),
        "no status before the device's clipboard took it"
    );
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    let (bytes, _) = inputs_until(&mut opened.peer, |bytes, _| contains(bytes, b"\x1b[0n"));
    assert_eq!(bytes, b"\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n");
    complete(&fixture, write_id, ClipboardWriteStatus::Failed);
    thread::sleep(QUIET);
    assert!(opened.peer.try_input().is_none(), "a second completion is dropped");
}

#[test]
fn image_only_writes_are_never_clears_and_each_status_reaches_the_program() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();
    let image = |png: &[u8]| {
        format!(
            "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime={};{}\x1b\\\x1b]5522;type=wdata\x1b\\",
            STANDARD.encode("image/png"),
            STANDARD.encode(png)
        )
        .into_bytes()
    };
    for (status, said) in [
        (ClipboardWriteStatus::Done, "DONE"),
        (ClipboardWriteStatus::Denied, "EPERM"),
        (ClipboardWriteStatus::Unsupported, "ENOSYS"),
        (ClipboardWriteStatus::Failed, "EIO"),
    ] {
        send(&opened.peer, &image(RED_PNG));
        let (write_id, contents) = written(&fixture);
        assert_eq!(
            contents,
            [part("image/png", RED_PNG)],
            "an image alone is a value, not a clear"
        );
        complete(&fixture, write_id, status);
        assert_eq!(
            write_status(&mut opened.peer),
            format!("\x1b]5522;type=write:status={said}\x1b\\\x1b[0n").as_bytes()
        );
    }

    // only a request with no representations clears: an empty transaction and OSC 52's empty write
    send(&opened.peer, b"\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata\x1b\\");
    let (write_id, contents) = written(&fixture);
    assert!(contents.is_empty());
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    assert_eq!(
        write_status(&mut opened.peer),
        b"\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n"
    );
    send(&opened.peer, b"\x1b]52;c;\x07");
    let (write_id, contents) = written(&fixture);
    assert!(contents.is_empty());
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    assert_eq!(write_status(&mut opened.peer), b"\x1b[0n");
}

#[test]
fn text_aliases_are_written_as_text_and_other_clipboards_are_unsupported_without_the_app() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();

    // `hello`, base64; OSC 52 writes text
    send(&opened.peer, b"\x1b]52;c;aGVsbG8=\x07");
    let (write_id, contents) = written(&fixture);
    assert_eq!(contents, [part("text/plain", b"hello")]);
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    assert_eq!(write_status(&mut opened.peer), b"\x1b[0n");

    let alias = format!(
        "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime={};{}\x1b\\\x1b]5522;type=wdata:mime={};{}\x1b\\\x1b]5522;type=wdata\x1b\\",
        STANDARD.encode("UTF8_STRING"),
        STANDARD.encode("alias"),
        STANDARD.encode("text/plain;charset=utf-8"),
        STANDARD.encode("dropped"),
    );
    send(&opened.peer, alias.as_bytes());
    let (write_id, contents) = written(&fixture);
    assert_eq!(
        contents,
        [part("text/plain", b"alias")],
        "one text part, under text/plain"
    );
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    assert_eq!(
        write_status(&mut opened.peer),
        b"\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n"
    );

    let primary = format!(
        "\x1b]5522;type=write:loc=primary\x1b\\\x1b]5522;type=wdata:mime={};aGk=\x1b\\\x1b]5522;type=wdata\x1b\\",
        STANDARD.encode("text/plain")
    );
    send(&opened.peer, primary.as_bytes());
    assert_eq!(
        write_status(&mut opened.peer),
        b"\x1b]5522;type=write:status=ENOSYS\x1b\\\x1b[0n"
    );
    send(&opened.peer, b"\x1b]52;p;aGk=\x07\x1b]52;s;eW8=\x07");
    assert_eq!(write_status(&mut opened.peer), b"\x1b[0n");
    assert!(
        fixture
            .quiet()
            .iter()
            .all(|event| !matches!(event, MobileEvent::ClipboardWrite { .. })),
        "the device has no other clipboard to ask about"
    );
}

#[test]
fn a_kitty_read_asks_store_names_and_answers_each_requested_type_with_its_grant() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();

    send(
        &opened.peer,
        &kitty_read(
            "UTF8_STRING image/png text/html application/x-a application/x-b .",
            "token",
        ),
    );
    let read = asked(&fixture);
    assert_eq!(
        read,
        Asked {
            read_id: read.read_id,
            clipboard: PaneClipboard::Standard,
            list_only: false,
            mimes: names(&["text/plain", "image/png", "text/html", "application/x-a"]),
            list: true,
            granted: false,
            can_remember: true,
        },
        "the first four types, text under the store's name"
    );
    let available: Vec<String> = (0..20).map(|n| format!("application/x-{n}")).collect();
    answer(
        &fixture,
        read.read_id,
        ClipboardReply::Allow {
            contents: vec![
                part("application/x-a", b"nul\0and\xff"),
                part("image/png", RED_PNG),
                part("text/plain", "héllo".as_bytes()),
            ],
            available: available.clone(),
            remember: true,
        },
    );
    let (data, status) = read_answer(&mut opened.peer);
    assert_eq!(status, "DONE");
    let listing = format!("{}\n", available[..16].join(" "));
    assert_eq!(
        data,
        BTreeMap::from([
            (".".to_owned(), listing.into_bytes()),
            ("UTF8_STRING".to_owned(), "héllo".as_bytes().to_vec()),
            ("application/x-a".to_owned(), b"nul\0and\xff".to_vec()),
            ("image/png".to_owned(), RED_PNG.to_vec()),
        ]),
        "each requested type under its own name, the absent HTML skipped, sixteen types listed"
    );

    send(&opened.peer, &kitty_read("text/plain", "token"));
    let again = asked(&fixture);
    assert!(again.granted, "the remembered Allow granted the program's next read");
    answer(&fixture, again.read_id, ClipboardReply::Deny);
    assert_eq!(read_answer(&mut opened.peer).1, "EPERM");

    send(&opened.peer, &kitty_read("text/plain", "other"));
    let other = asked(&fixture);
    assert!(!other.granted, "another password holds no grant");
    answer(
        &fixture,
        other.read_id,
        ClipboardReply::Allow {
            contents: Vec::new(),
            available: Vec::new(),
            remember: false,
        },
    );
    let (data, status) = read_answer(&mut opened.peer);
    assert_eq!((data, status.as_str()), (BTreeMap::new(), "DONE"));
}

#[test]
fn a_listing_asks_for_types_alone_and_a_read_without_a_password_keeps_no_grant() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();

    send(&opened.peer, &kitty_read(".", "token"));
    let listing = asked(&fixture);
    assert!(listing.list_only && listing.list && listing.mimes.is_empty());
    answer(
        &fixture,
        listing.read_id,
        ClipboardReply::Allow {
            contents: vec![part("text/plain", b"never served")],
            available: names(&["text/plain", "image/png"]),
            remember: false,
        },
    );
    let (data, status) = read_answer(&mut opened.peer);
    assert_eq!(status, "DONE");
    assert_eq!(
        data,
        BTreeMap::from([(".".to_owned(), b"text/plain image/png\n".to_vec())]),
        "a listing serves no data"
    );

    let unnamed = format!("\x1b]5522;type=read;{}\x1b\\", STANDARD.encode("text/plain"));
    send(&opened.peer, unnamed.as_bytes());
    let read = asked(&fixture);
    assert!(!read.can_remember, "no password, nothing to remember");
    answer(
        &fixture,
        read.read_id,
        ClipboardReply::Allow {
            contents: vec![part("text/plain", b"once")],
            available: Vec::new(),
            remember: true,
        },
    );
    assert_eq!(read_answer(&mut opened.peer).1, "DONE");
    send(&opened.peer, unnamed.as_bytes());
    assert!(!asked(&fixture).granted, "a remember without a password grants nothing");
}

#[test]
fn leaving_a_pane_refuses_its_waiting_writes() {
    let mut fixture = Fixture::connected();
    let _pane = fixture.show();
    let mut opened = fixture.attach();

    send(&opened.peer, MULTIPART);
    let (write_id, _) = written(&fixture);
    fixture
        .core
        .hide_pane(
            super::pane_tests::MACHINE.to_owned(),
            super::pane_tests::SESSION.to_owned(),
            super::pane_tests::PANE.to_owned(),
        )
        .unwrap();
    let (bytes, others) = inputs_until(&mut opened.peer, |_, others| others.contains(&ChannelInput::Close));
    assert!(
        contains(&bytes, b"status=EPERM"),
        "the write is refused before the channel closes: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(others, [ChannelInput::Close]);
    complete(&fixture, write_id, ClipboardWriteStatus::Done);
    thread::sleep(QUIET);
    assert!(opened.peer.try_input().is_none(), "a late completion is dropped");
}
