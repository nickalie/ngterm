//! Embedder-side clipboard grants and deferred writes. Grants mirror Ghostty's
//! `terminal/kitty/clipboard_grants.zig` at `LIBGHOSTTY_VT_REV`, including swap removal.
use std::collections::VecDeque;

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::{ClipboardContent, ClipboardLocation, ClipboardOrigin, ClipboardWriteRequest, ClipboardWriteResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardDirection {
    Read,
    Write,
}

#[derive(Clone)]
struct Grant {
    pw: Vec<u8>,
    read: bool,
    write: bool,
    one_time: bool,
}

/// Per-terminal password grants. Debug never discloses passwords.
#[derive(Default)]
pub struct ClipboardGrants {
    entries: Vec<Grant>,
}
impl std::fmt::Debug for ClipboardGrants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardGrants")
            .field("entries", &self.entries.len())
            .finish()
    }
}
impl ClipboardGrants {
    #[must_use]
    pub fn peek(&self, pw: &[u8], direction: ClipboardDirection) -> bool {
        self.entries
            .iter()
            .find(|entry| entry.pw == pw)
            .is_some_and(|entry| match direction {
                ClipboardDirection::Read => entry.read,
                ClipboardDirection::Write => entry.write,
            })
    }
    pub fn consume(&mut self, pw: &[u8], direction: ClipboardDirection) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.pw == pw && !pw.is_empty()) else {
            return false;
        };
        let allowed = self.peek(pw, direction);
        if self.entries[index].one_time {
            self.entries.swap_remove(index);
        }
        allowed
    }
    pub fn grant(&mut self, pw: &[u8], direction: ClipboardDirection, one_time: bool) {
        if pw.is_empty() || pw.len() > 128 {
            return;
        }
        let index = if let Some(index) = self.entries.iter().position(|entry| entry.pw == pw) {
            self.entries[index].one_time &= one_time;
            index
        } else {
            if self.entries.len() >= 32 {
                self.entries.remove(0);
            }
            self.entries.push(Grant {
                pw: pw.to_vec(),
                read: false,
                write: false,
                one_time,
            });
            self.entries.len() - 1
        };
        match direction {
            ClipboardDirection::Read => self.entries[index].read = true,
            ClipboardDirection::Write => self.entries[index].write = true,
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    /// Register only the output of a successful library paste after its bytes were queued.
    pub fn pasted(&mut self, bytes: &[u8]) {
        let Some(body) = bytes.strip_prefix(b"\x1b]5522;type=read:status=OK:") else {
            return;
        };
        let Some(field) = body
            .split(|byte| matches!(byte, b':' | b';' | 0x1b))
            .find_map(|field| field.strip_prefix(b"pw="))
        else {
            return;
        };
        if let Ok(token) = STANDARD.decode(field) {
            self.grant(&token, ClipboardDirection::Read, true);
        }
    }
}

/// A committed write copied before the C callback returns. Passwords remain private to the core.
#[derive(Clone)]
pub struct PendingClipboardWrite {
    pub location: ClipboardLocation,
    pub contents: Vec<ClipboardContent>,
    pub name: String,
    pub granted: bool,
    pub can_remember: bool,
    pub origin: ClipboardOrigin,
    pub pw: Vec<u8>,
    /// Printed control input must never acknowledge the program.
    pub discard_reply: bool,
    template: Option<Vec<u8>>,
}
impl std::fmt::Debug for PendingClipboardWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingClipboardWrite")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl PendingClipboardWrite {
    #[must_use]
    pub fn has_acknowledgement(&self) -> bool {
        self.template.is_some()
    }

    /// Complete once, using the library's id and terminator unchanged.
    pub fn complete(
        self,
        result: ClipboardWriteResult,
        remember: bool,
        grants: &mut ClipboardGrants,
    ) -> Option<Vec<u8>> {
        if result == ClipboardWriteResult::Success && remember && self.can_remember {
            grants.grant(&self.pw, ClipboardDirection::Write, false);
        }
        if self.discard_reply {
            return None;
        }
        let template = self.template?;
        let status = match result {
            ClipboardWriteResult::Success => "DONE",
            ClipboardWriteResult::Denied => "EPERM",
            ClipboardWriteResult::Unsupported => "ENOSYS",
            ClipboardWriteResult::Busy => "EBUSY",
            ClipboardWriteResult::InvalidData => "EINVAL",
            ClipboardWriteResult::IoError => "EIO",
        };
        let index = template.windows(12).position(|bytes| bytes == b"status=EPERM")?;
        Some(
            [
                &template[..index],
                format!("status={status}").as_bytes(),
                &template[index + 12..],
            ]
            .concat(),
        )
    }
}

/// Captures only the acknowledgement immediately following a committed write callback.
#[derive(Debug, Default)]
pub struct DeferredClipboardWrites {
    capture: Option<PendingClipboardWrite>,
    ready: VecDeque<PendingClipboardWrite>,
}
impl DeferredClipboardWrites {
    pub fn capture(&mut self, request: &ClipboardWriteRequest<'_>) {
        self.finish_feed();
        self.capture = Some(PendingClipboardWrite {
            location: request.location,
            contents: request
                .contents
                .iter()
                .map(|content| ClipboardContent {
                    mime: String::from_utf8_lossy(content.mime).into_owned(),
                    data: content.data.to_vec(),
                })
                .collect(),
            name: String::from_utf8_lossy(request.name).into_owned(),
            granted: request.granted,
            can_remember: request.can_remember,
            origin: request.origin,
            pw: request.pw.to_vec(),
            discard_reply: false,
            template: None,
        });
    }
    /// Whether the next PTY write was the captured Kitty acknowledgement and must be swallowed.
    pub fn intercept(&mut self, bytes: &[u8]) -> bool {
        let matches = self
            .capture
            .as_ref()
            .is_some_and(|write| write.origin == ClipboardOrigin::Kitty)
            && bytes.starts_with(b"\x1b]5522;type=write:status=EPERM");
        if matches && let Some(write) = self.capture.as_mut() {
            write.template = Some(bytes.to_vec());
        }
        self.finish_feed();
        matches
    }
    /// Finalize an unmatched slot before publishing it. It never crosses a feed boundary.
    pub fn finish_feed(&mut self) {
        if let Some(write) = self.capture.take() {
            self.ready.push_back(write);
        }
    }
    pub fn take(&mut self) -> Vec<PendingClipboardWrite> {
        self.finish_feed();
        self.ready.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ClipboardWriteRequest<'static> {
        ClipboardWriteRequest {
            location: ClipboardLocation::Standard,
            contents: Vec::new(),
            name: b"Test",
            granted: false,
            can_remember: true,
            pw: b"token",
            origin: ClipboardOrigin::Kitty,
        }
    }
    #[test]
    fn grants_merge_directions_pop_on_mismatch_and_keep_swap_remove_eviction_order() {
        let mut grants = ClipboardGrants::default();
        grants.grant(b"token", ClipboardDirection::Read, true);
        assert!(grants.peek(b"token", ClipboardDirection::Read));
        assert!(!grants.consume(b"token", ClipboardDirection::Write));
        assert!(!grants.consume(b"token", ClipboardDirection::Read));
        for i in 0..32 {
            grants.grant(format!("pw{i}").as_bytes(), ClipboardDirection::Read, i == 0);
        }
        assert!(grants.consume(b"pw0", ClipboardDirection::Read));
        grants.grant(b"pw32", ClipboardDirection::Write, false);
        grants.grant(b"pw33", ClipboardDirection::Write, false);
        assert!(!grants.peek(b"pw31", ClipboardDirection::Read));
        assert!(grants.peek(b"pw1", ClipboardDirection::Read));
        grants.grant(b"pw1", ClipboardDirection::Write, false);
        assert!(grants.consume(b"pw1", ClipboardDirection::Read));
        assert!(grants.consume(b"pw1", ClipboardDirection::Write));
        grants.grant(b"", ClipboardDirection::Read, false);
        grants.grant(&[b'x'; 129], ClipboardDirection::Read, false);
        assert_eq!(grants.entries.len(), 32);
    }
    #[test]
    fn deferred_writes_keep_their_own_templates_and_missing_ack_keeps_kitty_origin() {
        let mut writes = DeferredClipboardWrites::default();
        writes.capture(&request());
        assert!(writes.intercept(b"\x1b]5522;type=write:status=EPERM:id=first\x07"));
        writes.capture(&request());
        assert!(writes.intercept(b"\x1b]5522;type=write:status=EPERM:id=second\x1b\\"));
        writes.capture(&request());
        writes.finish_feed();
        let mut pending = writes.take().into_iter();
        let mut grants = ClipboardGrants::default();
        assert_eq!(
            pending
                .next()
                .unwrap()
                .complete(ClipboardWriteResult::Success, true, &mut grants),
            Some(b"\x1b]5522;type=write:status=DONE:id=first\x07".to_vec())
        );
        assert_eq!(
            pending
                .next()
                .unwrap()
                .complete(ClipboardWriteResult::Denied, true, &mut grants),
            Some(b"\x1b]5522;type=write:status=EPERM:id=second\x1b\\".to_vec())
        );
        let missing = pending.next().unwrap();
        assert_eq!(missing.origin, ClipboardOrigin::Kitty);
        assert!(!missing.has_acknowledgement());
        assert!(
            missing
                .complete(ClipboardWriteResult::Denied, true, &mut grants)
                .is_none()
        );
        assert!(grants.peek(b"token", ClipboardDirection::Write));
        assert!(!writes.intercept(b"\x1b]5522;type=write:status=EPERM:id=later\x07"));
    }
    #[test]
    fn a_new_callback_finalizes_an_unmatched_slot_and_only_the_next_write_can_be_an_ack() {
        let mut writes = DeferredClipboardWrites::default();
        writes.capture(&request());
        assert!(!writes.intercept(b"\x1b[0n"));
        writes.capture(&request());
        writes.capture(&request());
        assert!(writes.intercept(b"\x1b]5522;type=write:status=EPERM\x07"));
        let pending = writes.take();
        assert!(!pending[0].has_acknowledgement());
        assert!(!pending[1].has_acknowledgement());
        assert!(pending[2].has_acknowledgement());
    }
}
