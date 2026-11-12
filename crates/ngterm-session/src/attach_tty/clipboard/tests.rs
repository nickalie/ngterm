use super::*;

fn b64(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}

#[test]
fn a_packets_debug_output_never_shows_its_password_or_data() {
    let packet = Packet::parse(
        format!(
            "5522;type=read:status=DATA:id=op:mime={}:pw={};{}",
            b64("text/plain"),
            b64("outer-password"),
            b64("clipboard-secret")
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(packet.token, b"outer-password");
    assert_eq!(packet.data, b"clipboard-secret");
    let shown = format!("{packet:?}");
    assert!(
        shown.starts_with(r#"Packet { operation: "read", status: "DATA", id: "op", .."#),
        "{shown}"
    );
    for secret in ["outer-password", "clipboard-secret"] {
        assert!(!shown.contains(secret), "{shown}");
        assert!(!shown.contains(&format!("{:?}", secret.as_bytes())), "{shown}");
    }
}

#[test]
fn a_packet_takes_the_primary_location_and_skips_keys_it_does_not_know() {
    let primary = Packet::parse(b"5522;type=read:status=OK:loc=primary:name=VGVzdA==:future=1").unwrap();
    assert!(primary.primary);
    assert_eq!((primary.operation.as_str(), primary.status.as_str()), ("read", "OK"));
    assert!(
        !Packet::parse(b"5522;type=read:status=OK:loc=clipboard")
            .unwrap()
            .primary
    );
}

#[test]
fn only_final_statuses_are_outcomes() {
    for (status, outcome) in [
        ("DONE", Some(ClipboardStatus::Success)),
        ("EPERM", Some(ClipboardStatus::Denied)),
        ("ENOSYS", Some(ClipboardStatus::Unsupported)),
        ("EBUSY", Some(ClipboardStatus::Busy)),
        ("EINVAL", Some(ClipboardStatus::InvalidData)),
        ("EIO", Some(ClipboardStatus::IoError)),
        ("OK", None),
        ("DATA", None),
        ("LATER", None),
    ] {
        let packet = Packet::parse(format!("5522;type=read:status={status}").as_bytes()).unwrap();
        assert_eq!(packet.outcome(), outcome, "{status}");
    }
}

#[test]
fn a_primary_read_names_its_location_and_asks_at_most_four_types() {
    let mimes: Vec<String> = ["text/plain", "text/html", "image/png", "image/gif", "application/pdf"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        read_query("op", "p", &mimes, true, &[]),
        format!(
            "\x1b]5522;type=read:id=op:loc=primary;{}\x1b\\",
            b64("text/plain text/html image/png image/gif .")
        )
        .into_bytes()
    );
}

#[test]
fn a_write_carries_its_password_and_an_empty_representation_and_nothing_else_writes() {
    let request = DelegatedRequest::ClipboardWrite {
        selection: "p".into(),
        contents: vec![
            ClipboardRepresentation {
                mime: "text/plain".into(),
                data: Vec::new(),
            },
            ClipboardRepresentation {
                mime: "text/html".into(),
                data: b"<b>x</b>".to_vec(),
            },
        ],
        name: "Program".into(),
        granted: false,
        can_remember: true,
        kitty: true,
    };
    let expected = format!(
        "\x1b]5522;type=write:id=op:loc=primary:name={}:pw={}\x1b\\\
         \x1b]5522;type=wdata:mime={}\x1b\\\
         \x1b]5522;type=wdata:mime={};{}\x1b\\\
         \x1b]5522;type=wdata\x1b\\",
        b64("Program"),
        b64("alias"),
        b64("text/plain"),
        b64("text/html"),
        b64("<b>x</b>")
    );
    assert_eq!(
        String::from_utf8(write_query("op", &request, b"alias")).unwrap(),
        expected
    );
    let read = DelegatedRequest::ClipboardRead {
        selection: "c".into(),
        list_only: false,
    };
    assert!(write_query("op", &read, b"alias").is_empty());
}
