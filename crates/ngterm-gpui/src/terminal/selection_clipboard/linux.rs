//! Linux: the clipboard and the primary selection per MIME, through GPUI's Wayland/X11 clipboards (our
//! `gpui-pre-linux` patch 0007, after Ghostty's GTK `Surface.set`, `kittyReadStart` and `availableMimes`).

use gpui_kit::App;
use gpui_linux::{ClipboardMimeError, ClipboardRepresentation, ClipboardTarget};
use ngterm_term::ClipboardContent;

use super::MimeFailure;

fn failure(error: ClipboardMimeError) -> MimeFailure {
    match error {
        ClipboardMimeError::Unsupported => MimeFailure::Unsupported,
        ClipboardMimeError::Io(reason) => MimeFailure::Io(reason),
    }
}

fn contents(representations: Vec<ClipboardRepresentation>) -> Vec<ClipboardContent> {
    representations
        .into_iter()
        .map(|representation| ClipboardContent {
            mime: representation.mime,
            data: representation.data,
        })
        .collect()
}

fn representations(contents: Vec<ClipboardContent>) -> Vec<ClipboardRepresentation> {
    contents
        .into_iter()
        .map(|content| ClipboardRepresentation {
            mime: content.mime,
            data: content.data,
        })
        .collect()
}

fn types(target: ClipboardTarget) -> Result<Vec<String>, MimeFailure> {
    gpui_linux::clipboard_mime_types(target).map_err(failure)
}

fn read(target: ClipboardTarget, wanted: &[String]) -> Result<Vec<ClipboardContent>, MimeFailure> {
    gpui_linux::read_clipboard_mimes(target, wanted)
        .map(contents)
        .map_err(failure)
}

fn write(target: ClipboardTarget, contents: Vec<ClipboardContent>) -> Result<(), MimeFailure> {
    gpui_linux::write_clipboard_representations(target, representations(contents)).map_err(failure)
}

pub(super) fn clipboard_mime_types(_: &App) -> Result<Vec<String>, MimeFailure> {
    types(ClipboardTarget::Clipboard)
}

pub(super) fn read_clipboard_mimes(wanted: &[String], _: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
    read(ClipboardTarget::Clipboard, wanted)
}

pub(super) fn write_clipboard_mimes(contents: Vec<ClipboardContent>, _: &mut App) -> Result<(), MimeFailure> {
    write(ClipboardTarget::Clipboard, contents)
}

pub(super) fn primary_mime_types(_: &App) -> Result<Vec<String>, MimeFailure> {
    types(ClipboardTarget::Primary)
}

pub(super) fn read_primary_mimes(wanted: &[String], _: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
    read(ClipboardTarget::Primary, wanted)
}

pub(super) fn write_primary_mimes(contents: Vec<ClipboardContent>, _: &mut App) -> Result<(), MimeFailure> {
    write(ClipboardTarget::Primary, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_registered_platform_every_access_is_unsupported() {
        // a test thread has no Linux platform: the patch reports that rather than a clipboard
        assert_eq!(types(ClipboardTarget::Clipboard), Err(MimeFailure::Unsupported));
        assert_eq!(
            read(ClipboardTarget::Primary, &["text/plain".into()]),
            Err(MimeFailure::Unsupported)
        );
        let content = ClipboardContent {
            mime: "image/png".into(),
            data: vec![1, 2],
        };
        assert_eq!(
            write(ClipboardTarget::Clipboard, vec![content.clone()]),
            Err(MimeFailure::Unsupported)
        );
        assert_eq!(contents(representations(vec![content.clone()])), [content]);
    }

    #[gpui_kit::test]
    fn the_clipboard_and_the_primary_selection_report_a_missing_platform_alike(cx: &mut gpui_kit::TestAppContext) {
        let wanted = ["text/plain".to_owned()];
        let part = || ClipboardContent {
            mime: "text/plain".into(),
            data: b"x".to_vec(),
        };
        cx.update(|cx| {
            assert_eq!(clipboard_mime_types(cx), Err(MimeFailure::Unsupported));
            assert_eq!(primary_mime_types(cx), Err(MimeFailure::Unsupported));
            assert_eq!(read_clipboard_mimes(&wanted, cx), Err(MimeFailure::Unsupported));
            assert_eq!(read_primary_mimes(&wanted, cx), Err(MimeFailure::Unsupported));
            assert_eq!(write_clipboard_mimes(vec![part()], cx), Err(MimeFailure::Unsupported));
            assert_eq!(write_primary_mimes(vec![part()], cx), Err(MimeFailure::Unsupported));
        });
    }

    #[test]
    fn a_refused_transfer_keeps_its_reason() {
        assert_eq!(
            failure(ClipboardMimeError::Io("the owner went away".into())),
            MimeFailure::Io("the owner went away".into())
        );
        assert_eq!(failure(ClipboardMimeError::Unsupported), MimeFailure::Unsupported);
    }
}
