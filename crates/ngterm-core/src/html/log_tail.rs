//! The text a log page shows from the bytes read off its file's end (upstream's `LogTail.swift`): whole lines only, the
//! last `max_lines` of them, with an unfinished character left for the next read and a copytruncate hole skipped.

/// The shown text, its line count, whether its first line is the clipped end of a line longer than the read window,
/// and `start_offset`, the file offset of the text's first byte (upstream's `LogTail.Snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub lines: usize,
    pub clipped: bool,
    pub start_offset: u64,
}

const LINE_FEED: u8 = b'\n';

/// The last `max_lines` whole lines of `data`, read from file offset `offset` (upstream's `LogTail.snapshot`). A read
/// that did not start at the file's start begins one byte before its window, so a window opening on a line boundary
/// keeps that line; the partial line ahead of the first LF is dropped, unless nothing follows that LF, when the one
/// line longer than the window is kept from its first whole character and marked clipped. An unfinished character at
/// the end is left for the next read, and a run of NUL bytes ahead of the text (a writer still at its old offset after
/// copytruncate) is skipped. `None` for a NUL byte or invalid UTF-8 in the kept lines, which mark binary data.
#[must_use]
pub fn snapshot(data: &[u8], offset: u64, max_lines: usize) -> Option<Snapshot> {
    let mut bytes = data;
    let mut clipped = false;
    if offset > 0 && !bytes.is_empty() {
        match bytes.iter().position(|byte| *byte == LINE_FEED) {
            Some(feed) if feed < bytes.len() - 1 => bytes = &bytes[feed + 1..],
            _ => {
                let continuation = bytes.iter().take_while(|byte| is_continuation(**byte)).count();
                bytes = &bytes[continuation..];
                clipped = !bytes.is_empty();
            }
        }
    }
    let unfinished = unfinished_tail(bytes);
    bytes = &bytes[..bytes.len() - unfinished];
    let hole = bytes.iter().take_while(|byte| **byte == 0).count();
    if hole > 0 && hole < bytes.len() {
        bytes = &bytes[hole..];
    }

    let feeds: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == LINE_FEED).then_some(index))
        .collect();
    let total = feeds.len() + usize::from(bytes.last().is_some_and(|last| *last != LINE_FEED));
    if total > max_lines {
        let drop = total - max_lines;
        bytes = feeds.get(drop - 1).map_or(&[][..], |feed| &bytes[feed + 1..]);
        clipped = false;
    }
    if bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?.to_owned();
    // the unfinished character is the only cut at the end, so every other missing byte was cut at the front
    let start_offset = offset + (data.len() - unfinished - bytes.len()) as u64;
    Some(Snapshot {
        text,
        lines: total.min(max_lines),
        clipped,
        start_offset,
    })
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// The count of trailing bytes that are a valid prefix of a character the read cut short; a complete or malformed
/// ending counts 0, leaving the decode to judge it.
fn unfinished_tail(bytes: &[u8]) -> usize {
    let tail_start = bytes.len().saturating_sub(3);
    let Some(lead) = (tail_start..bytes.len())
        .rev()
        .find(|index| !is_continuation(bytes[*index]))
    else {
        return 0;
    };
    let present = bytes.len() - lead;
    let (needed, second) = match bytes[lead] {
        0xC2..=0xDF => (2, 0x80..=0xBF),
        0xE0 => (3, 0xA0..=0xBF),
        0xED => (3, 0x80..=0x9F),
        0xE1..=0xEF => (3, 0x80..=0xBF),
        0xF0 => (4, 0x90..=0xBF),
        0xF4 => (4, 0x80..=0x8F),
        0xF1..=0xF3 => (4, 0x80..=0xBF),
        _ => return 0,
    };
    if present >= needed {
        return 0;
    }
    if present > 1 && !second.contains(&bytes[lead + 1]) {
        return 0;
    }
    present
}

#[cfg(test)]
#[path = "log_tail_tests.rs"]
mod tests;
