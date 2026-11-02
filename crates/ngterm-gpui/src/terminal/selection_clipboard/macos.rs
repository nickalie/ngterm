//! macOS: the selection clipboard is a named `NSPasteboard` of the app's own (upstream's `pasteboard(for:)`), read and
//! written as text, as the terminal's other copies are.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// The selection pasteboard's name for `app_id` (upstream: the bundle id plus `.selection`).
#[must_use]
pub fn pasteboard_name(app_id: &str) -> String {
    format!("{app_id}.selection")
}

/// The text on the pasteboard named `name`.
#[must_use]
pub fn read(name: &str) -> Option<String> {
    let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(name));
    // SAFETY: AppKit's pasteboard type constant, a static NSString that lives as long as the process.
    let kind = unsafe { NSPasteboardTypeString };
    pasteboard.stringForType(kind).map(|text| text.to_string())
}

/// Replaces what the pasteboard named `name` holds with `text`.
pub fn write(name: &str, text: &str) {
    let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(name));
    // SAFETY: as in `read`.
    let kind = unsafe { NSPasteboardTypeString };
    pasteboard.clearContents();
    if !pasteboard.setString_forType(&NSString::from_str(text), kind) {
        tracing::warn!(target: "ngterm::surface", pasteboard = name, "the selection pasteboard refused the text");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_private_pasteboard_round_trips_text() {
        let name = format!("com.nickalie.ngterm.test.{}", uuid::Uuid::new_v4());
        super::write(&name, "selection text");
        assert_eq!(super::read(&name).as_deref(), Some("selection text"));
    }
}
