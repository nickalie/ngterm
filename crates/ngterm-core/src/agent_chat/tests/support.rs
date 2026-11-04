//! What the parsers' tests share: feeding bytes through a parser and comparing with a golden.

use std::path::Path;

use crate::agent_chat::{ChatEvent, EventBody, Tool, TranscriptParser};

/// Every line of `bytes` through `parser`, each with its byte offset.
pub fn parse_bytes(parser: &mut dyn TranscriptParser, bytes: &[u8]) -> Vec<ChatEvent> {
    let mut events = Vec::new();
    let mut cursor = 0;
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        let body = line.strip_suffix(b"\n").unwrap_or(line);
        events.extend(parser.parse_line(body, cursor));
        cursor += line.len() as u64;
    }
    events
}

fn ndjson(events: &[ChatEvent]) -> String {
    events.iter().map(|event| event.to_ndjson() + "\n").collect()
}

/// The events equal the golden NDJSON at `path`. `UPDATE_GOLDEN=1` rewrites it (after a deliberate parser change;
/// review the diff).
fn assert_golden(path: &Path, events: &[ChatEvent]) {
    let actual = ndjson(events);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(path, &actual).unwrap();
    }
    let golden = std::fs::read_to_string(path).unwrap_or_default();
    for (index, (want, got)) in golden.lines().zip(actual.lines()).enumerate() {
        assert_eq!(got, want, "{}: event {index} differs from the golden", path.display());
    }
    assert_eq!(
        actual.lines().count(),
        golden.lines().count(),
        "{}: event count",
        path.display()
    );
}

/// Every golden line reads back as the event it was written from: the NDJSON form round-trips.
fn assert_round_trip(events: &[ChatEvent]) {
    for event in events {
        let line = event.to_ndjson();
        let read: ChatEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(&read, event, "{line}");
    }
}

/// A recorded transcript's events: some, round-tripping, equal to the golden at `golden`, and none `unknown` (the
/// recorded version's lines all parse).
pub fn golden_case(golden: &Path, events: &[ChatEvent]) {
    assert!(!events.is_empty(), "{}: no events", golden.display());
    assert_round_trip(events);
    assert_golden(golden, events);
    let unknown: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.body, EventBody::Unknown { .. }))
        .map(ChatEvent::to_ndjson)
        .collect();
    assert!(
        unknown.is_empty(),
        "{}: the recorded version's lines all parse: {unknown:#?}",
        golden.display()
    );
}

/// The last event with `id`: what the chat shows for that item once every line is read.
pub fn last<'a>(events: &'a [ChatEvent], id: &str) -> &'a ChatEvent {
    events
        .iter()
        .rev()
        .find(|event| event.id == id)
        .unwrap_or_else(|| panic!("no event {id}"))
}

pub fn tool(event: &ChatEvent) -> &Tool {
    match &event.body {
        EventBody::Tool(tool) => tool,
        other => panic!("not a tool: {other:?}"),
    }
}
