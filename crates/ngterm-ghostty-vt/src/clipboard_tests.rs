//! Cases from Ghostty's `test/clipboard-multipart.py` and `stream_terminal.zig` at `LIBGHOSTTY_VT_REV`.
use std::sync::{Arc, Mutex};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::*;

#[derive(Default)]
struct Seen {
    output: Vec<u8>,
    writes: Vec<Vec<ClipboardContent>>,
    permissions: Vec<(bool, bool)>,
    reads: Vec<Vec<String>>,
    origins: Vec<ClipboardOrigin>,
}

struct Clipboard {
    seen: Arc<Mutex<Seen>>,
    result: ClipboardWriteResult,
    remember: bool,
    contents: Vec<ClipboardContent>,
}

fn parts() -> Vec<ClipboardContent> {
    [
        (
            "text/plain",
            include_bytes!("../tests/data/clipboard/plain.txt").as_slice(),
        ),
        (
            "text/html",
            include_bytes!("../tests/data/clipboard/html.txt").as_slice(),
        ),
        (
            "image/png",
            include_bytes!("../tests/data/clipboard/red.png").as_slice(),
        ),
        (
            "application/octet-stream",
            include_bytes!("../tests/data/clipboard/arbitrary.bin").as_slice(),
        ),
    ]
    .into_iter()
    .map(|(mime, data)| ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    })
    .collect()
}

impl Effects for Clipboard {
    fn write_pty(&mut self, data: &[u8]) {
        self.seen.lock().unwrap().output.extend_from_slice(data);
    }

    fn clipboard_write_reply(&mut self, request: &ClipboardWriteRequest<'_>) -> (ClipboardWriteResult, bool) {
        let mut seen = self.seen.lock().unwrap();
        seen.origins.push(request.origin);
        seen.permissions.push((request.granted, request.can_remember));
        seen.writes.push(
            request
                .contents
                .iter()
                .map(|content| ClipboardContent {
                    mime: String::from_utf8(content.mime.to_vec()).unwrap(),
                    data: content.data.to_vec(),
                })
                .collect(),
        );
        (self.result, self.remember)
    }

    fn clipboard_read_reply(&mut self, request: &ClipboardReadRequest<'_>) -> (ClipboardReadReply, bool) {
        let mut seen = self.seen.lock().unwrap();
        seen.origins.push(request.origin);
        seen.permissions.push((request.granted, request.can_remember));
        seen.reads.push(
            request
                .mimes
                .iter()
                .map(|mime| String::from_utf8(mime.to_vec()).unwrap())
                .collect(),
        );
        (
            ClipboardReadReply::Success {
                contents: self.contents.clone(),
                available: self.contents.iter().map(|content| content.mime.clone()).collect(),
            },
            self.remember,
        )
    }
}

fn terminal(
    result: ClipboardWriteResult,
    remember: bool,
    contents: Vec<ClipboardContent>,
) -> (Terminal, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal
        .set_effects(Box::new(Clipboard {
            seen: seen.clone(),
            result,
            remember,
            contents,
        }))
        .unwrap();
    (terminal, seen)
}

fn read(types: &str, password: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name=VGVzdA==:pw={};{}\x1b\\",
        STANDARD.encode(password),
        STANDARD.encode(types)
    )
    .into_bytes()
}

#[test]
fn multipart_bytes_and_directional_terminal_grants() {
    let (mut terminal, seen) = terminal(ClipboardWriteResult::Success, true, parts());
    let transaction = include_bytes!("../tests/data/clipboard/multipart.vt");
    for byte in transaction {
        terminal.vt_write(&[*byte]);
    }
    terminal.vt_write(transaction);
    terminal.vt_write(&read("text/plain", "1234"));
    terminal.vt_write(&read("text/plain", "1234"));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.writes, [parts(), parts()]);
    assert_eq!(
        seen.permissions,
        [(false, true), (true, true), (false, true), (true, true)]
    );
    assert_eq!(
        seen.output
            .windows(b"status=DONE".len())
            .filter(|w| *w == b"status=DONE")
            .count(),
        4
    );
}

#[test]
fn denied_or_passwordless_writes_never_create_grants() {
    for result in [ClipboardWriteResult::Denied, ClipboardWriteResult::Success] {
        let (mut terminal, seen) = terminal(result, true, Vec::new());
        let transaction: &[u8] = if result == ClipboardWriteResult::Denied {
            include_bytes!("../tests/data/clipboard/multipart.vt")
        } else {
            b"\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata\x1b\\"
        };
        terminal.vt_write(transaction);
        terminal.vt_write(transaction);
        assert!(seen.lock().unwrap().permissions.iter().all(|(granted, _)| !granted));
    }
}

#[test]
fn listing_preserves_one_time_paste_token_and_text_source_inserts_text() {
    let data = parts();
    let (mut terminal, seen) = terminal(ClipboardWriteResult::Denied, false, data.clone());
    terminal.vt_write(b"\x1b[?5522h");
    terminal
        .paste_contents(&data, PasteSource::Clipboard, ClipboardLocation::Standard, false)
        .unwrap();
    let event = std::mem::take(&mut seen.lock().unwrap().output);
    terminal.clipboard_grants().lock().unwrap().pasted(&event);
    let event = String::from_utf8(event).unwrap();
    let token = event
        .split("pw=")
        .nth(1)
        .unwrap()
        .split([':', ';', '\x1b'])
        .next()
        .unwrap();
    let token = String::from_utf8(STANDARD.decode(token).unwrap()).unwrap();
    assert!(event.contains(&STANDARD.encode("text/plain text/html image/png application/octet-stream\n")));
    terminal.vt_write(&read(".", &token));
    terminal.vt_write(&read("image/png", &token));
    terminal.vt_write(&read("image/png", &token));
    assert_eq!(
        seen.lock().unwrap().permissions,
        [(false, true), (true, true), (false, true)]
    );
    seen.lock().unwrap().output.clear();
    terminal
        .paste_contents(&data, PasteSource::Text, ClipboardLocation::Standard, false)
        .unwrap();
    assert_eq!(seen.lock().unwrap().output, data[0].data);
}

fn packets(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    std::str::from_utf8(bytes)
        .unwrap()
        .split("\x1b]5522;")
        .skip(1)
        .map(|packet| {
            let packet = packet.strip_suffix("\x1b\\").unwrap();
            let (metadata, payload) = packet.split_once(';').unwrap_or((packet, ""));
            (metadata.to_owned(), STANDARD.decode(payload).unwrap())
        })
        .collect()
}

#[test]
fn reads_skip_unavailable_types_keep_request_order_and_chunk_binary_without_loss() {
    let mut contents = parts();
    let binary: Vec<u8> = (0..=255).cycle().take(4097).collect();
    contents[3].data = binary.clone();
    let (mut terminal, seen) = terminal(ClipboardWriteResult::Denied, false, contents.clone());
    terminal.vt_write(&read(
        "application/missing application/octet-stream image/png text/html text/plain",
        "",
    ));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.reads[0].len(), 4);
    let packets = packets(&seen.output);
    let data: Vec<_> = packets
        .iter()
        .filter(|(meta, _)| meta.contains("status=DATA"))
        .collect();
    assert_eq!(data.len(), 4);
    assert_eq!(data[0].1.len(), 4096);
    assert_eq!(data[1].1.len(), 1);
    assert_eq!([data[0].1.as_slice(), data[1].1.as_slice()].concat(), binary);
    assert!(data[0].0.contains(&STANDARD.encode("application/octet-stream")));
    assert_eq!(data[2].1, contents[2].data);
    assert_eq!(data[3].1, contents[1].data);
}

#[test]
fn listings_cap_types_at_sixteen_without_data_and_grants_do_not_cross_terminals() {
    let contents = (0..20)
        .map(|i| ClipboardContent {
            mime: format!("application/x-{i}"),
            data: vec![0, 255],
        })
        .collect();
    let (mut first, seen) = terminal(ClipboardWriteResult::Denied, true, contents);
    first.vt_write(&read(".", "token"));
    let packets = packets(&seen.lock().unwrap().output);
    let listing = packets.iter().find(|(meta, _)| meta.contains("status=DATA")).unwrap();
    assert_eq!(
        String::from_utf8(listing.1.clone()).unwrap().split_whitespace().count(),
        16
    );
    assert_eq!(packets.len(), 3);
    first.vt_write(&read("text/plain", "token"));
    first.vt_write(&read("text/plain", "token"));
    assert_eq!(seen.lock().unwrap().permissions[2], (true, true));
    let (mut other, seen) = terminal(ClipboardWriteResult::Denied, true, Vec::new());
    other.vt_write(&read("text/plain", "token"));
    assert_eq!(seen.lock().unwrap().permissions, [(false, true)]);
}

#[test]
fn reset_clears_the_rust_grant_table_and_origins_are_authoritative() {
    let (mut terminal, seen) = terminal(ClipboardWriteResult::Success, true, parts());
    terminal.vt_write(include_bytes!("../tests/data/clipboard/multipart.vt"));
    let grants = terminal.clipboard_grants();
    assert!(grants.lock().unwrap().peek(b"1234", ClipboardDirection::Write));
    terminal.vt_write(b"\x1bc");
    assert!(!grants.lock().unwrap().peek(b"1234", ClipboardDirection::Write));
    terminal.vt_write(b"\x1b]1337;Copy=:aGk=\x07");
    assert_eq!(seen.lock().unwrap().writes.last().unwrap()[0].data, b"hi");
    assert_eq!(
        seen.lock().unwrap().origins,
        [ClipboardOrigin::Kitty, ClipboardOrigin::Osc1337]
    );
}

#[test]
fn a_denied_callback_still_consumes_the_authoritative_one_time_token() {
    struct Refuses;
    impl Effects for Refuses {}
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.set_effects(Box::new(Refuses)).unwrap();
    let grants = terminal.clipboard_grants();
    grants.lock().unwrap().grant(b"token", ClipboardDirection::Read, true);
    terminal.vt_write(&read("text/plain", "token"));
    assert!(!grants.lock().unwrap().peek(b"token", ClipboardDirection::Read));
}
