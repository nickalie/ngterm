//! macOS: the selection clipboard is a named `NSPasteboard` of the app's own (upstream's `pasteboard(for:)`), read and
//! written as text, as the terminal's other copies are. The Kitty clipboard protocol reaches it and the general
//! pasteboard per MIME type (Ghostty 83fb956's `NSPasteboard+Extension.swift`): `text/plain` is the string type, any
//! other MIME type its `UTType`'s identifier (the MIME type itself when it has none), and a listing maps the declared
//! types back through their `UTType`s without reading data. `text/plain` reads the pasteboard's string as is: unlike
//! Ghostty, a copied file is not served as its escaped path or as `text/uri-list`, as ngterm's own paste does not.

use ngterm_term::ClipboardContent;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSData, NSString};

use super::MimeFailure;

// `UTType` lives in UniformTypeIdentifiers, which AppKit loads; link it so the class is there in any binary.
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

/// The selection pasteboard's name for `app_id` (upstream: the bundle id plus `.selection`).
#[must_use]
pub fn pasteboard_name(app_id: &str) -> String {
    format!("{app_id}.selection")
}

/// The text on the pasteboard named `name`.
#[must_use]
pub fn read(name: &str) -> Option<String> {
    let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(name));
    pasteboard.stringForType(string_type()).map(|text| text.to_string())
}

/// Replaces what the pasteboard named `name` holds with `text`.
pub fn write(name: &str, text: &str) {
    let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(name));
    pasteboard.clearContents();
    if !pasteboard.setString_forType(&NSString::from_str(text), string_type()) {
        tracing::warn!(target: "ngterm::surface", pasteboard = name, "the selection pasteboard refused the text");
    }
}

fn string_type() -> &'static NSString {
    // SAFETY: AppKit's pasteboard type constant, a static NSString that lives as long as the process.
    unsafe { NSPasteboardTypeString }
}

/// The pasteboard named `name`, or the general one.
fn pasteboard(name: Option<&str>) -> Retained<NSPasteboard> {
    match name {
        Some(name) => NSPasteboard::pasteboardWithName(&NSString::from_str(name)),
        None => NSPasteboard::generalPasteboard(),
    }
}

fn ut_type_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"UTType")
}

/// The pasteboard type a MIME type is stored under (Ghostty's `PasteboardType(mimeType:)`).
#[must_use]
pub fn pasteboard_type(mime: &str) -> Retained<NSString> {
    if mime == "text/plain" {
        return NSString::from_str(&string_type().to_string());
    }
    let identifier = ut_type_class().and_then(|class| {
        let mime = NSString::from_str(mime);
        // SAFETY: `+[UTType typeWithMIMEType:]` takes an NSString and returns an autoreleased UTType or nil.
        let ty: Option<Retained<AnyObject>> = unsafe { msg_send![class, typeWithMIMEType: &*mime] };
        // SAFETY: `-[UTType identifier]` returns a non-nil NSString.
        ty.map(|ty| -> Retained<NSString> { unsafe { msg_send![&*ty, identifier] } })
    });
    identifier.unwrap_or_else(|| NSString::from_str(mime))
}

/// The MIME type a pasteboard type maps to through its `UTType`, UTF-8 text named `text/plain`.
#[must_use]
pub fn mime_of(ty: &NSString) -> Option<String> {
    let class = ut_type_class()?;
    // SAFETY: `+[UTType typeWithIdentifier:]` takes an NSString and returns an autoreleased UTType or nil.
    let ut: Option<Retained<AnyObject>> = unsafe { msg_send![class, typeWithIdentifier: ty] };
    // SAFETY: `-[UTType preferredMIMEType]` returns an NSString or nil.
    let mime: Option<Retained<NSString>> = unsafe { msg_send![&*ut?, preferredMIMEType] };
    let mime = mime?.to_string();
    Some(if mime == "text/plain;charset=utf-8" {
        "text/plain".into()
    } else {
        mime
    })
}

/// The MIME types the pasteboard named `name` (or the general one) declares, text first, without reading data.
#[must_use]
pub fn mime_types(name: Option<&str>) -> Vec<String> {
    let pasteboard = pasteboard(name);
    let declared: Vec<Retained<NSString>> = pasteboard.types().map(|types| types.to_vec()).unwrap_or_default();
    let mut types: Vec<String> = Vec::new();
    for mime in declared.iter().filter_map(|ty| mime_of(ty)) {
        if !types.contains(&mime) {
            types.push(mime);
        }
    }
    if let Some(text) = types.iter().position(|mime| mime == "text/plain") {
        let text = types.remove(text);
        types.insert(0, text);
    }
    types
}

/// Each of `wanted` the pasteboard named `name` (or the general one) holds, in order.
#[must_use]
pub fn read_mimes(name: Option<&str>, wanted: &[String]) -> Vec<ClipboardContent> {
    let pasteboard = pasteboard(name);
    wanted
        .iter()
        .filter_map(|mime| {
            let data = if mime == "text/plain" {
                pasteboard.stringForType(string_type())?.to_string().into_bytes()
            } else {
                pasteboard.dataForType(&pasteboard_type(mime))?.to_vec()
            };
            Some(ClipboardContent {
                mime: mime.clone(),
                data,
            })
        })
        .collect()
}

/// Replaces what the pasteboard named `name` (or the general one) holds with exactly `contents`; none clears it.
///
/// # Errors
/// [`MimeFailure::Io`] when the pasteboard refuses a representation.
pub fn write_mimes(name: Option<&str>, contents: Vec<ClipboardContent>) -> Result<(), MimeFailure> {
    let pasteboard = pasteboard(name);
    let types: Vec<Retained<NSString>> = contents.iter().map(|content| pasteboard_type(&content.mime)).collect();
    pasteboard.clearContents();
    if contents.is_empty() {
        return Ok(());
    }
    let declared = NSArray::from_retained_slice(&types);
    // SAFETY: no owner; the pasteboard copies the type array.
    unsafe { pasteboard.declareTypes_owner(&declared, None) };
    for (content, ty) in contents.into_iter().zip(&types) {
        let data = NSData::with_bytes(&content.data);
        if !pasteboard.setData_forType(Some(&data), ty) {
            return Err(MimeFailure::Io(format!("the pasteboard refused {}", content.mime)));
        }
    }
    Ok(())
}

#[cfg(not(test))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the pasteboard's reads share the other stores' signatures"
)]
pub(super) fn general_mime_types(_: &gpui_kit::App) -> Result<Vec<String>, MimeFailure> {
    Ok(mime_types(None))
}

#[cfg(not(test))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the pasteboard's reads share the other stores' signatures"
)]
pub(super) fn read_general_mimes(wanted: &[String], _: &gpui_kit::App) -> Result<Vec<ClipboardContent>, MimeFailure> {
    Ok(read_mimes(None, wanted))
}

#[cfg(not(test))]
pub(super) fn write_general_mimes(contents: Vec<ClipboardContent>, _: &mut gpui_kit::App) -> Result<(), MimeFailure> {
    write_mimes(None, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private pasteboard, released when dropped, so nothing reaches the desktop's.
    struct Private(String);

    impl Private {
        fn new() -> Self {
            Self(format!("com.nickalie.ngterm.test.{}", uuid::Uuid::new_v4()))
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            let pasteboard = NSPasteboard::pasteboardWithName(&NSString::from_str(&self.0));
            // SAFETY: a live `NSPasteboard`, and this message's signature; no one else knows this name.
            let () = unsafe { msg_send![&*pasteboard, releaseGlobally] };
        }
    }

    fn part(mime: &str, data: &[u8]) -> ClipboardContent {
        ClipboardContent {
            mime: mime.into(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn a_private_pasteboard_round_trips_text() {
        let name = Private::new();
        super::write(&name.0, "selection text");
        assert_eq!(super::read(&name.0).as_deref(), Some("selection text"));
    }

    #[test]
    fn mime_types_map_to_their_uniform_types() {
        assert_eq!(pasteboard_type("text/plain").to_string(), "public.utf8-plain-text");
        assert_eq!(pasteboard_type("text/html").to_string(), "public.html");
        assert_eq!(pasteboard_type("image/png").to_string(), "public.png");
        assert_eq!(mime_of(&NSString::from_str("public.png")).as_deref(), Some("image/png"));
        assert_eq!(
            mime_of(&NSString::from_str("public.utf8-plain-text")).as_deref(),
            Some("text/plain")
        );
    }

    #[test]
    fn every_representation_round_trips_by_mime_and_an_empty_write_clears() {
        let name = Private::new();
        let png = include_bytes!("../../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
        let binary = include_bytes!("../../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin");
        let parts = [
            part("text/plain", "héllo".as_bytes()),
            part("text/html", b"<b>hi</b>"),
            part("image/png", png),
            part("application/x-ngterm-test", binary),
        ];
        write_mimes(Some(&name.0), parts.to_vec()).unwrap();
        assert_eq!(
            mime_types(Some(&name.0))[..3],
            ["text/plain".to_owned(), "text/html".to_owned(), "image/png".to_owned()]
        );
        let wanted: Vec<String> = [
            "application/x-ngterm-test",
            "image/png",
            "image/jpeg",
            "text/plain",
            "text/html",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            read_mimes(Some(&name.0), &wanted),
            [parts[3].clone(), parts[2].clone(), parts[0].clone(), parts[1].clone()]
        );
        write_mimes(Some(&name.0), vec![part("image/png", png)]).unwrap();
        // the pasteboard may advertise its own conversions of the image (TIFF) beside it, never text
        let types = mime_types(Some(&name.0));
        assert_eq!(types.first().map(String::as_str), Some("image/png"), "{types:?}");
        assert!(types.iter().all(|mime| mime.starts_with("image/")), "{types:?}");
        assert!(
            read_mimes(Some(&name.0), &["text/plain".into()]).is_empty(),
            "an image-only write keeps no text"
        );
        write_mimes(Some(&name.0), Vec::new()).unwrap();
        assert!(mime_types(Some(&name.0)).is_empty());
    }
}
