//! The clipboard writes a client that fell behind gets back with its resync.
//!
//! A client whose queue overflowed is not sent the output it fell behind on (`super::clients`): its resync's
//! snapshot brings its terminal to the session's state, but a clipboard write in that output is no state, so the
//! client's terminal would never apply it, though `ngtermctl clipboard set` (the server's print) or the program had
//! it written and the api answered `Done`. So the pane keeps, per clipboard location, the latest write the native
//! owner completed successfully (OSC 52, OSC 1337 Copy or OSC 5522), moved from the owned deferred write
//! with the output offset at completion and the order it completed in
//! ([`Recovery`]). A client's resync snapshot, and the session's last snapshot for a client that fell behind, carry
//! each location's latest write that completed past where that client's copy of the stream stopped
//! (`Clients::missed_since`), re-encoded as OSC 52 or multipart OSC 5522 writes, in completion order after the snapshot's
//! reset: before its parser continuation, which leaves the client's parser inside a sequence. A fresh attach gets none.
//!
//! Recovery retains every MIME representation, including image-only values and empty clears. Text-only
//! writes retain their OSC 52 form; multipart writes replay as an OSC 5522 transaction without a grant.
//! A value exceeding the aggregate retention limit removes the old entry rather than replaying stale data.

use std::fmt::Write as _;
#[cfg(test)]
use std::sync::{Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ngterm_ghostty_vt as vt;

/// The maximum aggregate MIME and data bytes retained for one clipboard value.
pub const VALUE_LIMIT: usize = 6 * 1024 * 1024;

/// A clipboard write and its representations, `None` for a value too large to retain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub location: vt::ClipboardLocation,
    pub contents: Option<Vec<vt::ClipboardContent>>,
}

impl Written {
    /// Move completed representations into recovery, dropping oversized values atomically.
    #[must_use]
    pub fn new(location: vt::ClipboardLocation, contents: Vec<vt::ClipboardContent>) -> Self {
        let size = contents.iter().try_fold(0usize, |size, content| {
            size.checked_add(content.mime.len())?.checked_add(content.data.len())
        });
        Self {
            location,
            contents: size.filter(|size| *size <= VALUE_LIMIT).map(|_| contents),
        }
    }
}

/// Callback capture used only to test recovery replay through the real parser.
#[cfg(test)]
#[derive(Debug, Default)]
struct Recorder {
    writes: Mutex<Vec<Written>>,
}

#[cfg(test)]
impl Recorder {
    /// Records all representations atomically, or drops a value past the aggregate limit.
    pub fn record(&self, request: &vt::ClipboardWriteRequest<'_>) {
        let contents = request
            .contents
            .iter()
            .map(|content| vt::ClipboardContent {
                mime: String::from_utf8_lossy(content.mime).into_owned(),
                data: content.data.to_vec(),
            })
            .collect();
        self.lock().push(Written::new(request.location, contents));
    }

    /// The writes recorded since the last take, in order.
    #[must_use]
    pub fn take(&self) -> Vec<Written> {
        std::mem::take(&mut *self.lock())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Written>> {
        self.writes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One location's latest write.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// When it completed, among every write kept.
    ordinal: u64,
    /// The output offset of the end of the read it completed in.
    end: u64,
    /// The write encoded as OSC 52 or an OSC 5522 transaction.
    sequence: Vec<u8>,
}

/// Each clipboard location's latest write.
#[derive(Debug, Default)]
pub struct Recovery {
    /// Standard, selection, primary.
    entries: [Option<Entry>; 3],
    next: u64,
}

impl Recovery {
    /// `writes`, completed in order in the read that ended at the output offset `end`.
    pub fn completed(&mut self, writes: Vec<Written>, end: u64) {
        for written in writes {
            let slot = &mut self.entries[index(written.location)];
            *slot = written.contents.map(|contents| Entry {
                ordinal: self.next,
                end,
                sequence: encode(written.location, &contents),
            });
            self.next += 1;
        }
    }

    /// The writes a client whose copy of the stream stopped at `missed_since` did not get: each location's latest that
    /// completed past it, in the order they completed, one after another.
    #[must_use]
    pub fn since(&self, missed_since: u64) -> Vec<u8> {
        let mut entries: Vec<&Entry> = self
            .entries
            .iter()
            .flatten()
            .filter(|entry| entry.end > missed_since)
            .collect();
        entries.sort_by_key(|entry| entry.ordinal);
        entries
            .into_iter()
            .flat_map(|entry| entry.sequence.iter().copied())
            .collect()
    }
}

fn index(location: vt::ClipboardLocation) -> usize {
    match location {
        vt::ClipboardLocation::Standard => 0,
        vt::ClipboardLocation::Selection => 1,
        vt::ClipboardLocation::Primary => 2,
    }
}

/// Replay text-only OSC 52 unchanged; other values use an atomic Kitty transaction.
fn encode(location: vt::ClipboardLocation, contents: &[vt::ClipboardContent]) -> Vec<u8> {
    let text = match contents {
        [] => Some(&[][..]),
        [text] if text.mime == "text/plain" || text.mime.starts_with("text/plain;") => Some(text.data.as_slice()),
        _ => None,
    };
    if let Some(text) = text {
        let name = match location {
            vt::ClipboardLocation::Standard => "c",
            vt::ClipboardLocation::Selection => "s",
            vt::ClipboardLocation::Primary => "p",
        };
        return format!("\x1b]52;{name};{}\x07", STANDARD.encode(text)).into_bytes();
    }
    let loc = if location == vt::ClipboardLocation::Primary {
        "primary"
    } else {
        "clipboard"
    };
    let mut result = format!("\x1b]5522;type=write:loc={loc}\x1b\\");
    for content in contents {
        let _ = write!(
            result,
            "\x1b]5522;type=wdata:mime={};{}\x1b\\",
            STANDARD.encode(&content.mime),
            STANDARD.encode(&content.data)
        );
    }
    result.push_str("\x1b]5522;type=wdata\x1b\\");
    result.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(location: vt::ClipboardLocation, text: &str) -> Written {
        Written {
            location,
            contents: Some(vec![vt::ClipboardContent {
                mime: "text/plain".into(),
                data: text.as_bytes().to_vec(),
            }]),
        }
    }

    #[test]
    fn a_client_gets_each_locations_latest_write_past_its_stop_in_completion_order() {
        let mut recovery = Recovery::default();
        recovery.completed(vec![written(vt::ClipboardLocation::Standard, "old")], 10);
        recovery.completed(
            vec![
                written(vt::ClipboardLocation::Primary, "b"),
                written(vt::ClipboardLocation::Selection, "a"),
            ],
            20,
        );
        recovery.completed(vec![written(vt::ClipboardLocation::Standard, "new")], 30);
        // "b" is Yg==, "a" YQ==: the primary write completed before the selection one, in the same read
        assert_eq!(
            String::from_utf8(recovery.since(10)).unwrap(),
            "\x1b]52;p;Yg==\x07\x1b]52;s;YQ==\x07\x1b]52;c;bmV3\x07"
        );
        assert_eq!(String::from_utf8(recovery.since(20)).unwrap(), "\x1b]52;c;bmV3\x07");
        assert!(recovery.since(30).is_empty(), "nothing past the end");
        // a stop inside the read a write completed in gets it again
        assert_eq!(recovery.since(29), recovery.since(20));
    }

    #[test]
    fn the_order_of_two_locations_in_one_read_is_the_order_they_completed_in() {
        let mut recovery = Recovery::default();
        recovery.completed(
            vec![
                written(vt::ClipboardLocation::Selection, "a"),
                written(vt::ClipboardLocation::Primary, "b"),
            ],
            5,
        );
        assert_eq!(
            String::from_utf8(recovery.since(0)).unwrap(),
            "\x1b]52;s;YQ==\x07\x1b]52;p;Yg==\x07"
        );
    }

    #[test]
    fn a_write_too_large_to_keep_empties_its_location() {
        let mut recovery = Recovery::default();
        recovery.completed(vec![written(vt::ClipboardLocation::Standard, "kept")], 1);
        recovery.completed(
            vec![Written {
                location: vt::ClipboardLocation::Standard,
                contents: None,
            }],
            2,
        );
        assert!(recovery.since(0).is_empty(), "never the stale value in its place");
    }

    #[test]
    fn the_recorder_keeps_every_representation_and_empty_clears() {
        let recorder = Recorder::default();
        let image = vt::ClipboardWriteRequest {
            location: vt::ClipboardLocation::Standard,
            contents: vec![vt::ClipboardData {
                mime: b"image/png",
                data: b"\x89PNG",
            }],
            name: b"",
            granted: false,
            can_remember: false,
            pw: b"",
            origin: vt::ClipboardOrigin::Osc52,
        };
        recorder.record(&image);
        let clear = vt::ClipboardWriteRequest {
            location: vt::ClipboardLocation::Standard,
            contents: Vec::new(),
            name: b"",
            granted: false,
            can_remember: false,
            pw: b"",
            origin: vt::ClipboardOrigin::Osc52,
        };
        recorder.record(&clear);
        let text = vt::ClipboardWriteRequest {
            location: vt::ClipboardLocation::Primary,
            contents: vec![
                vt::ClipboardData {
                    mime: b"image/png",
                    data: b"\x89PNG",
                },
                vt::ClipboardData {
                    mime: b"text/plain;charset=utf-8",
                    data: b"hi",
                },
            ],
            name: b"",
            granted: false,
            can_remember: false,
            pw: b"",
            origin: vt::ClipboardOrigin::Osc52,
        };
        recorder.record(&text);
        let writes = recorder.take();
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0].contents.as_ref().unwrap()[0].data, b"\x89PNG");
        assert_eq!(writes[1].contents, Some(Vec::new()));
        assert_eq!(writes[2].contents.as_ref().unwrap().len(), 2);
        assert_eq!(writes[2].contents.as_ref().unwrap()[1].data, b"hi");
        assert!(recorder.take().is_empty());
        let large = vec![b'x'; VALUE_LIMIT + 1];
        recorder.record(&vt::ClipboardWriteRequest {
            location: vt::ClipboardLocation::Standard,
            contents: vec![vt::ClipboardData {
                mime: b"text/plain",
                data: &large,
            }],
            name: b"",
            granted: false,
            can_remember: false,
            pw: b"",
            origin: vt::ClipboardOrigin::Osc52,
        });
        assert_eq!(
            recorder.take(),
            [Written {
                location: vt::ClipboardLocation::Standard,
                contents: None
            }]
        );
    }
    #[test]
    fn multipart_recovery_replays_binary_representations_and_image_only_and_clear() {
        struct Capture(std::sync::Arc<Recorder>);
        impl vt::Effects for Capture {
            fn clipboard_write(&mut self, request: &vt::ClipboardWriteRequest<'_>) -> vt::ClipboardWriteResult {
                self.0.record(request);
                vt::ClipboardWriteResult::Success
            }
        }
        let recorder = std::sync::Arc::new(Recorder::default());
        let mut terminal = vt::Terminal::new(80, 24).unwrap();
        terminal.set_effects(Box::new(Capture(recorder.clone()))).unwrap();
        terminal.vt_write(include_bytes!(
            "../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt"
        ));
        let written = recorder.take();
        let parts = written[0].contents.clone().unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[2].data.len(), 73);
        assert_eq!(parts[3].data.len(), 2048);
        let mut recovery = Recovery::default();
        recovery.completed(written.clone(), 1);
        terminal.vt_write(&recovery.since(0));
        assert_eq!(recorder.take(), written);
        for contents in [vec![parts[2].clone()], Vec::new()] {
            let written = vec![Written {
                location: vt::ClipboardLocation::Primary,
                contents: Some(contents),
            }];
            recovery.completed(written.clone(), 2);
            terminal.vt_write(&recovery.since(1));
            assert_eq!(recorder.take(), written);
        }
        for (location, selector) in [
            (vt::ClipboardLocation::Standard, "c"),
            (vt::ClipboardLocation::Selection, "s"),
            (vt::ClipboardLocation::Primary, "p"),
        ] {
            let clear = Written::new(location, Vec::new());
            let mut recovery = Recovery::default();
            recovery.completed(vec![clear.clone()], 1);
            let bytes = recovery.since(0);
            assert_eq!(bytes, format!("\x1b]52;{selector};\x07").as_bytes());
            terminal.vt_write(&bytes);
            assert_eq!(
                recorder.take(),
                [clear],
                "the actual parser preserves the clear's location"
            );
        }
    }
}
