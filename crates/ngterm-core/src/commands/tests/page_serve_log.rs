//! A log page's end on the page channel (upstream's `ServerStreamsTests` log tail cases and `PageServeFramesTests`,
//! #978): `log.tail` answers the raw tail bytes with their stamp and offset, `log.unchanged` while the stamp holds, or a
//! failure for a file that cannot be read and for a claim that is no log page.

use std::sync::Arc;

use uuid::Uuid;

use super::page_serve::FakeChannel;
use crate::bounded_file::FileStamp;
use crate::html::HtmlSource;
use crate::html::text_page::{MAX_TAIL_BYTES, TextLog};
use crate::remote::page_serve::{
    CANCELLED, MAX_CHUNK_BYTES, MAX_PENDING_LINES, PageServeFrame, PageServeStream, ServedPage, decode, encode,
};

const LOG: TextLog = TextLog {
    tail: 100,
    follow: true,
};

/// A stream serving `source` with `log` over a fake connection.
fn serve(source: HtmlSource, log: Option<TextLog>) -> (Arc<PageServeStream>, Arc<FakeChannel>) {
    let fake = FakeChannel::new(MAX_PENDING_LINES);
    let served = ServedPage {
        log,
        ..ServedPage::unlinked(source)
    };
    let stream = PageServeStream::start(Uuid::new_v4(), served, fake.clone(), None).unwrap();
    (stream, fake)
}

fn text(path: &std::path::Path) -> HtmlSource {
    HtmlSource::Text {
        path: path.to_str().unwrap().to_owned(),
        line: None,
    }
}

fn ask(stream: &Arc<PageServeStream>, id: i64, stamp: Option<FileStamp>) {
    let mut line = encode(&PageServeFrame::LogTail { id, stamp }).unwrap();
    line.pop();
    assert!(stream.take(&line).is_none());
}

/// The answer to tail request `id`: its head's stamp and offset and its bytes, or its terminal frame alone.
fn tail(fake: &FakeChannel, id: i64) -> Result<(FileStamp, u64, Vec<u8>), PageServeFrame> {
    let (stamp, offset) = match fake.frame().unwrap() {
        PageServeFrame::LogHead { id: of, stamp, offset } if of == id => (stamp, offset),
        other => return Err(other),
    };
    let mut data = Vec::new();
    loop {
        match fake.frame().unwrap() {
            PageServeFrame::Chunk { id: of, data: bytes } if of == id => {
                assert!(bytes.len() <= MAX_CHUNK_BYTES);
                data.extend(bytes);
            }
            PageServeFrame::End { id: of } if of == id => return Ok((stamp, offset, data)),
            other => return Err(other),
        }
    }
}

/// Upstream's `testALogTailServesTheFileEndAndThenUnchanged` and `testALogTailOfAChangedFileReadsItAgain`.
#[test]
fn a_log_tail_serves_the_end_then_unchanged_until_the_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.log");
    std::fs::write(&path, "one\ntwo\n").unwrap();
    let (stream, fake) = serve(text(&path), Some(LOG));
    ask(&stream, 1, None);
    let (stamp, offset, data) = tail(&fake, 1).unwrap();
    assert_eq!((offset, data.as_slice(), stamp.size), (0, &b"one\ntwo\n"[..], 8));
    ask(&stream, 2, Some(stamp));
    assert_eq!(fake.frame(), Some(PageServeFrame::LogUnchanged { id: 2 }));
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    ask(&stream, 3, Some(stamp));
    let (grown, _, data) = tail(&fake, 3).unwrap();
    assert_ne!(grown, stamp);
    assert_eq!(data, b"one\ntwo\nthree\n");
    // truncation reads the new, shorter file
    std::fs::write(&path, "x\n").unwrap();
    ask(&stream, 4, Some(grown));
    assert_eq!(tail(&fake, 4).unwrap().2, b"x\n");
    std::fs::write(&path, "").unwrap();
    ask(&stream, 6, None);
    assert_eq!(
        tail(&fake, 6).unwrap().2,
        b"",
        "an empty file answers a head and an end"
    );
    stream.shutdown();
}

/// Upstream's `testALogTailOverTheTextCapReadsOnlyItsWindow`: any size opens, its window and the byte before it sent.
#[test]
fn a_log_over_the_tail_cap_sends_only_its_window_in_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let size = MAX_TAIL_BYTES * 3;
    std::fs::write(&path, vec![b'a'; size]).unwrap();
    let (stream, fake) = serve(text(&path), Some(LOG));
    ask(&stream, 1, None);
    let (stamp, offset, data) = tail(&fake, 1).unwrap();
    assert_eq!(data.len(), MAX_TAIL_BYTES + 1);
    assert_eq!(offset, (size - MAX_TAIL_BYTES - 1) as u64);
    assert_eq!(stamp.size, size as u64);
    stream.shutdown();
}

/// Upstream's `testALogTailOfAMissingFileFails`, `testALogTailOfANonLogClaimIsRefused`.
#[test]
fn a_missing_file_or_a_claim_that_is_no_log_page_answers_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.log");
    let (stream, fake) = serve(text(&missing), Some(LOG));
    ask(&stream, 1, None);
    assert_eq!(
        fake.frame(),
        Some(PageServeFrame::Failure {
            id: 1,
            error: "file cannot be read".to_owned()
        })
    );
    stream.shutdown();
    let present = dir.path().join("notes.txt");
    std::fs::write(&present, "a\n").unwrap();
    for (source, log) in [
        (text(&present), None),
        (
            HtmlSource::Markdown {
                path: present.to_str().unwrap().to_owned(),
            },
            Some(LOG),
        ),
    ] {
        let (stream, fake) = serve(source, log);
        ask(&stream, 2, None);
        assert_eq!(
            fake.frame(),
            Some(PageServeFrame::Failure {
                id: 2,
                error: "not a log page".to_owned()
            })
        );
        stream.shutdown();
    }
}

/// Upstream's `testCancelALogTailInsideItsChunkLoop`: the cancel lands after the head and the first chunks, and the
/// tail stops there with one cancelled failure, never its end.
#[test]
fn a_log_tail_cancelled_inside_its_chunk_loop_stops_with_one_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    std::fs::write(&path, vec![b'a'; MAX_TAIL_BYTES]).unwrap();
    let (stream, fake) = serve(text(&path), Some(LOG));
    ask(&stream, 1, None);
    // the writer blocks on the full queue, holding the rest of the window
    fake.wait_queued(MAX_PENDING_LINES);
    let mut line = encode(&PageServeFrame::Cancel { id: 1 }).unwrap();
    line.pop();
    assert!(stream.take(&line).is_none());
    let mut chunks = 0;
    let terminal = loop {
        match fake.frame().unwrap() {
            PageServeFrame::LogHead { .. } => {}
            PageServeFrame::Chunk { .. } => chunks += 1,
            other => break other,
        }
    };
    assert_eq!(
        terminal,
        PageServeFrame::Failure {
            id: 1,
            error: CANCELLED.to_owned()
        }
    );
    assert!(
        chunks < MAX_TAIL_BYTES / MAX_CHUNK_BYTES,
        "stopped early: {chunks} chunks"
    );
    stream.shutdown();
}

/// Upstream's `PageServeFramesTests` log cases: the wire spelling, a tail with no stamp omits it, and the stamp is
/// upstream's object.
#[test]
fn log_frames_round_trip_in_upstreams_spelling() {
    let stamp = FileStamp {
        identity: (66, 1234),
        size: 8,
        modified: (1_790_000_000, 5),
    };
    let wire = |frame: &PageServeFrame| {
        let mut line = encode(frame).unwrap();
        line.pop();
        (
            serde_json::from_slice::<serde_json::Value>(&line).unwrap(),
            decode(&line).unwrap(),
        )
    };
    let stamp_json = serde_json::json!({"device": 66, "inode": 1234, "size": 8, "modifiedSeconds": 1_790_000_000_i64,
        "modifiedNanoseconds": 5});
    for (frame, json) in [
        (
            PageServeFrame::LogTail { id: 1, stamp: None },
            serde_json::json!({"id": 1, "kind": "log.tail"}),
        ),
        (
            PageServeFrame::LogTail {
                id: 2,
                stamp: Some(stamp),
            },
            serde_json::json!({"id": 2, "kind": "log.tail", "stamp": stamp_json}),
        ),
        (
            PageServeFrame::LogHead {
                id: 3,
                stamp,
                offset: 9,
            },
            serde_json::json!({"id": 3, "kind": "log.head", "stamp": stamp_json, "offset": 9}),
        ),
        (
            PageServeFrame::LogUnchanged { id: 4 },
            serde_json::json!({"id": 4, "kind": "log.unchanged"}),
        ),
    ] {
        assert_eq!(wire(&frame), (json, frame.clone()));
    }
    assert!(
        decode(br#"{"id":5,"kind":"log.head","offset":1}"#).is_err(),
        "a head needs its stamp"
    );
    assert!(decode(br#"{"id":5,"kind":"log.head","stamp":{"device":1}}"#).is_err());
}
