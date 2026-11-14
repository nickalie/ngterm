//! A Claude Code conversation that moved: Claude Code (seen in 2.1.325) can carry a conversation on in a background
//! process of its daemon, under a new id and a new transcript beside the old one (the user's `/background`, or a Left
//! arrow at the empty prompt). The old transcript then ends with a `continued-in` row naming the new id, and nothing
//! in the new one names the old. The background process's hooks run with the daemon's environment, so this row is
//! what ties them to the pane the conversation came from ([`moved_to`], behind `agent.locate`).
//!
//! As Claude Code reads it, the row counts only while no message follows it: a conversation the user went back to
//! in its first process is that process's again.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// How much of a transcript's end is searched for its `continued-in` row: Claude Code writes it last, a few short
/// rows of its own state after it at most.
const TAIL: u64 = 256 * 1024;

/// How many moves of one conversation are followed.
const MOVES: usize = 8;

/// Whether the conversation whose transcript is `transcript` moved, in one move or several, to the conversation
/// `id`.
#[must_use]
pub fn moved_to(transcript: &Path, id: &str) -> bool {
    let mut at = transcript.to_path_buf();
    for _ in 0..MOVES {
        let Some(next) = continued_in(&at) else {
            return false;
        };
        if next == id {
            return true;
        }
        at.set_file_name(format!("{next}.jsonl"));
    }
    false
}

/// The id the conversation in `transcript` continued under; `None` when it did not move, a message came after the
/// move, or the file can't be read.
#[must_use]
pub fn continued_in(transcript: &Path) -> Option<String> {
    let tail = tail(transcript)?;
    for line in tail.lines().rev() {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match row.get("type").and_then(Value::as_str) {
            Some("continued-in") => {
                return row
                    .get("continuedInSessionId")
                    .and_then(Value::as_str)
                    .filter(|id| is_id(id))
                    .map(str::to_owned);
            }
            Some("user" | "assistant") => return None,
            _ => {}
        }
    }
    None
}

/// The whole rows in the last [`TAIL`] bytes of `path`.
fn tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let from = size.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let whole = if from == 0 {
        &text[..]
    } else {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    };
    Some(whole.to_owned())
}

/// A conversation id as Claude Code names its transcripts: letters, digits and dashes, so it is a file name.
fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn moved(to: &str) -> String {
        format!(
            r#"{{"type":"continued-in","timestamp":"2026-11-14T05:48:26.190Z","sessionId":"aaaa","continuedInSessionId":"{to}"}}"#
        )
    }

    const STATE: &str = r#"{"type":"cost-state","sessionId":"aaaa","totalCostUSD":2.7}"#;
    const REPLY: &str = r#"{"parentUuid":"p","type":"assistant","message":{"role":"assistant","content":[]}}"#;

    #[test]
    fn a_transcript_names_where_its_conversation_continued_until_a_message_follows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aaaa.jsonl");
        assert_eq!(continued_in(&path), None, "no file");
        fs::write(&path, format!("{REPLY}\n")).unwrap();
        assert_eq!(continued_in(&path), None, "it never moved");
        fs::write(&path, format!("{REPLY}\n{}\n{STATE}\n", moved("bbbb"))).unwrap();
        assert_eq!(continued_in(&path), Some("bbbb".to_owned()));
        fs::write(&path, format!("{REPLY}\n{}\n{STATE}\n{REPLY}\n", moved("bbbb"))).unwrap();
        assert_eq!(continued_in(&path), None, "the user went back to it");
        fs::write(&path, format!("{REPLY}\n{}\n", moved("../x"))).unwrap();
        assert_eq!(continued_in(&path), None, "no id");
    }

    #[test]
    fn a_long_transcript_is_read_from_its_end_past_a_row_cut_in_two() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aaaa.jsonl");
        let blob = format!(
            r#"{{"type":"api-request-blob","message":"{}"}}"#,
            "x".repeat(300 * 1024)
        );
        fs::write(&path, format!("{}\n{blob}\n{}\n{STATE}\n", moved("old"), moved("bbbb"))).unwrap();
        assert_eq!(continued_in(&path), Some("bbbb".to_owned()));
        fs::write(&path, format!("{}\n{blob}\n", moved("old"))).unwrap();
        assert_eq!(continued_in(&path), None, "the row is beyond the tail");
    }

    #[test]
    fn moves_are_followed_from_transcript_to_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = |id: &str| dir.path().join(format!("{id}.jsonl"));
        fs::write(path("aaaa"), format!("{REPLY}\n{}\n", moved("bbbb"))).unwrap();
        fs::write(path("bbbb"), format!("{REPLY}\n{}\n{STATE}\n", moved("cccc"))).unwrap();
        fs::write(path("cccc"), format!("{REPLY}\n")).unwrap();
        assert!(moved_to(&path("aaaa"), "bbbb"));
        assert!(moved_to(&path("aaaa"), "cccc"));
        assert!(moved_to(&path("bbbb"), "cccc"));
        assert!(!moved_to(&path("bbbb"), "aaaa"));
        assert!(!moved_to(&path("cccc"), "dddd"));
        assert!(!moved_to(&path("aaaa"), "dddd"));
        // a loop ends
        fs::write(path("cccc"), format!("{}\n", moved("aaaa"))).unwrap();
        assert!(!moved_to(&path("aaaa"), "dddd"));
    }
}
