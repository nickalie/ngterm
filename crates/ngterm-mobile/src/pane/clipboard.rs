//! A shown pane's clipboard traffic as the apps see it: a program's reads (OSC 52, the Kitty clipboard protocol's
//! OSC 5522) wait for [`crate::MobileCore::answer_clipboard_read`], and its writes for
//! [`crate::MobileCore::complete_clipboard_write`], each under an id of the core's own. Both go through the MIME
//! contract the desktop's stores keep ([`ngterm_term::clipboard_mime`]): the app's clipboard keeps its text as
//! `text/plain`, a read under any of Ghostty's text names reads that, and the answer carries the name the program
//! asked for.
//!
//! The device has one clipboard: a write to the selection or the primary one is answered unsupported here (ENOSYS)
//! and never reaches the app. A write completes only with the app's own status, once the device's clipboard took it
//! or refused it, and an image-only write is a write, never a clear: only a request with no representations clears.

use ngterm_term::clipboard_mime::{self, MAX_LISTED_TYPES};
use ngterm_term::{
    ClipboardAnswer, ClipboardLocation, ClipboardPermissionScope, ClipboardReadRequest, ClipboardWrite,
    ClipboardWriteResult, ReadScope, WriteCompletion,
};

use super::records::{ClipboardPart, ClipboardReply, ClipboardWriteStatus, PaneRef};
use crate::api::MobileEvent;

/// A clipboard read waiting for the user: the core's id, the terminal's, the names the program asked for (the app is
/// asked their store names) and whether an Allow may be remembered.
#[derive(Debug, Clone)]
pub(super) struct Read {
    pub(super) id: u64,
    pub(super) terminal: u64,
    wanted: Vec<String>,
    can_remember: bool,
}

/// A clipboard write waiting for the app's clipboard: the core's id and the terminal's (zero: a recovery replay, which
/// no program waits on).
#[derive(Debug, Clone, Copy)]
pub(super) struct Write {
    pub(super) id: u64,
    pub(super) terminal: u64,
}

/// The read `request` as the app is asked it under the core's `id`, and the record that answers it.
pub(super) fn read(pane: PaneRef, id: u64, request: &ClipboardReadRequest) -> (Read, MobileEvent) {
    let list_only = request.scope == ReadScope::ListOnly;
    let (wanted, mimes) = if list_only {
        (Vec::new(), Vec::new())
    } else {
        clipboard_mime::read_sources(&request.mimes)
    };
    let kitty = request.permission_scope == ClipboardPermissionScope::Kitty;
    let read = Read {
        id,
        terminal: request.id,
        wanted,
        can_remember: kitty && request.can_remember,
    };
    let event = MobileEvent::ClipboardReadRequest {
        pane,
        read_id: id,
        clipboard: request.location.into(),
        list_only,
        mimes,
        list: list_only || request.list,
        granted: kitty && request.granted,
        can_remember: read.can_remember,
    };
    (read, event)
}

/// The terminal's answer to `read` from the app's `reply`: the parts relabelled with the names the program asked for,
/// at most sixteen listed types, and a grant only where the request could keep one.
pub(super) fn answer(read: Read, reply: ClipboardReply) -> ClipboardAnswer {
    match reply {
        ClipboardReply::Allow {
            contents,
            mut available,
            remember,
        } => {
            let contents: Vec<_> = contents.into_iter().map(Into::into).collect();
            available.truncate(MAX_LISTED_TYPES);
            ClipboardAnswer::Data {
                contents: clipboard_mime::relabel(read.wanted, &contents),
                available,
                remember: remember && read.can_remember,
            }
        }
        ClipboardReply::Deny => ClipboardAnswer::Denied,
    }
}

/// The write `write` as the app is asked it under the core's `id`, and the record its completion finds; `None` for a
/// clipboard the device lacks, which the terminal is answered unsupported for at once.
pub(super) fn write(pane: PaneRef, id: u64, write: &ClipboardWrite) -> Option<(Write, MobileEvent)> {
    if write.location != ClipboardLocation::Standard {
        return None;
    }
    let contents = clipboard_mime::canonical_text(write.contents.clone())
        .into_iter()
        .map(ClipboardPart::from)
        .collect();
    let pending = Write { id, terminal: write.id };
    Some((
        pending,
        MobileEvent::ClipboardWrite {
            pane,
            write_id: id,
            contents,
        },
    ))
}

/// The terminal's completion of a write from the app's `status`; nothing is remembered (the apps ask no write
/// consent: Ghostty's `clipboard-write = allow`).
pub(super) fn completion(status: ClipboardWriteStatus) -> WriteCompletion {
    WriteCompletion {
        status: match status {
            ClipboardWriteStatus::Done => ClipboardWriteResult::Success,
            ClipboardWriteStatus::Denied => ClipboardWriteResult::Denied,
            ClipboardWriteStatus::Unsupported => ClipboardWriteResult::Unsupported,
            ClipboardWriteStatus::Failed => ClipboardWriteResult::IoError,
        },
        remember: false,
    }
}
