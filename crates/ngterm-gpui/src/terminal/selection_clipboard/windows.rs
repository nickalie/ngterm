//! Windows: the system clipboard per MIME type, in the Win32 clipboard's own formats (`cf_html.rs` names them). A
//! listing enumerates the formats without reading any data; reads skip what the clipboard does not hold; a write
//! empties the clipboard and sets every representation in one open, so readers never see half of it.

use std::ptr;

use ngterm_term::ClipboardContent;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT};

use super::MimeFailure;
use super::cf_html::{self, Format};

/// The clipboard, open until dropped. Another program may hold it for a moment: a few short retries.
struct Open;

impl Open {
    fn new() -> Result<Self, MimeFailure> {
        for attempt in 0..10 {
            // SAFETY: no owner window; closed on drop.
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                return Ok(Self);
            }
            if attempt < 9 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        Err(MimeFailure::Io(format!(
            "the clipboard is in use: {}",
            std::io::Error::last_os_error()
        )))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: opened by `new`.
        unsafe { CloseClipboard() };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The format id for a write; new names consume session-wide atoms, so reads must never call this.
fn write_format_id(format: &Format) -> Result<u32, MimeFailure> {
    match format {
        Format::UnicodeText => Ok(u32::from(CF_UNICODETEXT)),
        Format::Registered { name, .. } => {
            let name = wide(name);
            // SAFETY: a NUL-terminated UTF-16 string that outlives the call.
            match unsafe { RegisterClipboardFormatW(name.as_ptr()) } {
                0 => Err(MimeFailure::Io(std::io::Error::last_os_error().to_string())),
                id => Ok(id),
            }
        }
    }
}

/// A registered format's name; `None` for the predefined ones.
fn format_name(id: u32) -> Option<String> {
    let mut name = [0u16; 256];
    // SAFETY: the buffer's length is passed with it.
    let len = unsafe { GetClipboardFormatNameW(id, name.as_mut_ptr(), 256) };
    usize::try_from(len)
        .ok()
        .filter(|&len| len > 0)
        .map(|len| String::from_utf16_lossy(&name[..len]))
}

/// Finds an existing registered format by `name` without registering an untrusted read request's name.
fn read_format_id(_: &Open, name: &str) -> Option<u32> {
    let mut id = 0;
    loop {
        // SAFETY: the clipboard is open.
        id = unsafe { EnumClipboardFormats(id) };
        if id == 0 {
            return None;
        }
        if format_name(id).is_some_and(|available| available.eq_ignore_ascii_case(name)) {
            return Some(id);
        }
    }
}

/// The open clipboard's text as UTF-8: `CF_UNICODETEXT` (which Windows synthesizes from its other predefined text
/// formats), else a text alias another program registered as a format of its own.
fn text_of(open: &Open) -> Option<Vec<u8>> {
    if let Some(bytes) = data_of(open, u32::from(CF_UNICODETEXT)) {
        return Some(cf_html::from_unicode_text(&bytes));
    }
    cf_html::TEXT_ALIASES.iter().find_map(|alias| {
        let id = read_format_id(open, alias)?;
        data_of(open, id).map(|bytes| cf_html::alias_text(alias, &bytes))
    })
}

/// The bytes the open clipboard holds in format `id`.
fn data_of(_: &Open, id: u32) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is open; the handle stays the clipboard's.
    let handle = unsafe { GetClipboardData(id) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: a clipboard data handle is global memory.
    let size = unsafe { GlobalSize(handle) };
    // SAFETY: as above; unlocked below.
    let data = unsafe { GlobalLock(handle) };
    if data.is_null() {
        // a zero-byte part is stored as discarded memory
        return Some(Vec::new());
    }
    // SAFETY: the locked memory holds `size` bytes until it is unlocked.
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec();
    // SAFETY: locked above.
    unsafe { GlobalUnlock(handle) };
    Some(bytes)
}

/// Puts `bytes` on the open clipboard as format `id`. The clipboard owns the memory once `SetClipboardData`
/// succeeds; otherwise it is freed here.
fn set_data(_: &Open, id: u32, bytes: &[u8]) -> Result<(), MimeFailure> {
    let failure = || MimeFailure::Io(std::io::Error::last_os_error().to_string());
    // SAFETY: a fresh allocation, ours until handed over.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if memory.is_null() {
        return Err(failure());
    }
    let free = |memory| {
        // SAFETY: allocated above and never handed over.
        unsafe { GlobalFree(memory) };
    };
    if !bytes.is_empty() {
        // SAFETY: our allocation of `bytes.len()` bytes; unlocked below.
        let target = unsafe { GlobalLock(memory) };
        if target.is_null() {
            free(memory);
            return Err(failure());
        }
        // SAFETY: `target` holds `bytes.len()` writable bytes that do not overlap `bytes`.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len()) };
        // SAFETY: locked above.
        unsafe { GlobalUnlock(memory) };
    }
    // SAFETY: the clipboard is open and `memory` is movable global memory it takes over on success.
    if unsafe { SetClipboardData(id, memory) }.is_null() {
        free(memory);
        return Err(failure());
    }
    Ok(())
}

/// The MIME types the clipboard holds, text first, without reading any data.
///
/// # Errors
/// [`MimeFailure::Io`] when the clipboard stays in use.
pub(crate) fn mime_types(_: &gpui_kit::App) -> Result<Vec<String>, MimeFailure> {
    let open = Open::new()?;
    let mut types: Vec<String> = Vec::new();
    let mut id = 0;
    loop {
        // SAFETY: the clipboard is open.
        id = unsafe { EnumClipboardFormats(id) };
        if id == 0 {
            break;
        }
        let mime = if [CF_UNICODETEXT, CF_TEXT, CF_OEMTEXT].map(u32::from).contains(&id) {
            Some("text/plain".to_owned())
        } else {
            format_name(id).and_then(|name| cf_html::mime_of_registered(&name))
        };
        if let Some(mime) = mime
            && !types.contains(&mime)
        {
            types.push(mime);
        }
    }
    drop(open);
    if let Some(text) = types.iter().position(|mime| mime == "text/plain") {
        let text = types.remove(text);
        types.insert(0, text);
    }
    Ok(types)
}

/// Each of `wanted` the clipboard holds, in order.
///
/// # Errors
/// [`MimeFailure::Io`] when the clipboard stays in use.
pub(crate) fn read_mimes(wanted: &[String], _: &gpui_kit::App) -> Result<Vec<ClipboardContent>, MimeFailure> {
    let open = Open::new()?;
    let mut read = Vec::new();
    for mime in wanted {
        let data = match cf_html::format_of(mime) {
            Format::UnicodeText => text_of(&open),
            Format::Registered { name, html } => read_format_id(&open, &name)
                .and_then(|id| data_of(&open, id))
                .map(|bytes| if html { cf_html::unwrap(&bytes) } else { bytes }),
        };
        let Some(data) = data else {
            continue;
        };
        read.push(ClipboardContent {
            mime: mime.clone(),
            data,
        });
    }
    Ok(read)
}

/// Replaces the clipboard with exactly `contents`; none empties it.
///
/// # Errors
/// [`MimeFailure::Io`] when the clipboard stays in use or refuses a representation.
pub(crate) fn write_mimes(contents: Vec<ClipboardContent>, _: &mut gpui_kit::App) -> Result<(), MimeFailure> {
    let open = Open::new()?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(MimeFailure::Io(std::io::Error::last_os_error().to_string()));
    }
    for content in contents {
        let format = cf_html::format_of(&content.mime);
        let id = write_format_id(&format)?;
        let bytes = match format {
            Format::UnicodeText => cf_html::unicode_text(&content.data),
            Format::Registered { html: true, .. } => cf_html::wrap(&content.data),
            Format::Registered { .. } => content.data,
        };
        set_data(&open, id, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(mime: &str, data: &[u8]) -> ClipboardContent {
        ClipboardContent {
            mime: mime.into(),
            data: data.to_vec(),
        }
    }

    fn shared_write(contents: Vec<ClipboardContent>, cx: &mut gpui_kit::App) {
        super::super::write_through(contents, |contents| write_mimes(contents, cx)).unwrap();
    }

    fn shared_read(mimes: &[&str], cx: &gpui_kit::App) -> Vec<ClipboardContent> {
        let mimes: Vec<String> = mimes.iter().map(|&mime| mime.to_owned()).collect();
        super::super::read_through(&mimes, |sources| read_mimes(sources, cx)).unwrap()
    }

    /// A Kitty write under a text alias goes through the shared contract, so it stores real text (`CF_UNICODETEXT`,
    /// what every other program and a plain paste read) that reads back under the canonical name and the alias alike;
    /// text another program registered under an alias alone reads as the clipboard's text too.
    fn text_aliases_store_and_read_real_text(cx: &mut gpui_kit::App) {
        let alias = "text/plain;charset=utf-8";
        let text = "héllo 🐈".as_bytes();
        shared_write(vec![part(alias, text)], cx);
        let open = Open::new().unwrap();
        assert_eq!(
            data_of(&open, u32::from(CF_UNICODETEXT)).map(|bytes| cf_html::from_unicode_text(&bytes)),
            Some(text.to_vec()),
            "the text is stored as CF_UNICODETEXT, which other programs paste"
        );
        drop(open);
        assert_eq!(mime_types(cx).unwrap(), ["text/plain"]);
        assert_eq!(
            read_mimes(&["text/plain".into()], cx).unwrap(),
            [part("text/plain", text)]
        );
        assert_eq!(
            shared_read(&[alias, "UTF8_STRING", "text/plain"], cx),
            [part(alias, text), part("UTF8_STRING", text), part("text/plain", text)]
        );
        // an exact `text/plain` part wins over an earlier alias, which is not stored beside it
        let png = include_bytes!("../../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
        shared_write(
            vec![
                part("UTF8_STRING", b"alias"),
                part("text/plain", b"exact"),
                part("image/png", png),
            ],
            cx,
        );
        assert_eq!(mime_types(cx).unwrap(), ["text/plain", "image/png"]);
        assert_eq!(shared_read(&[alias], cx), [part(alias, b"exact")]);

        write_mimes(vec![part("UTF8_STRING", "café".as_bytes())], cx).unwrap();
        assert_eq!(mime_types(cx).unwrap(), ["text/plain"]);
        assert_eq!(
            shared_read(&["text/plain", alias], cx),
            [part("text/plain", "café".as_bytes()), part(alias, "café".as_bytes())]
        );
        write_mimes(vec![part("STRING", b"caf\xe9")], cx).unwrap();
        assert_eq!(
            shared_read(&["TEXT"], cx),
            [part("TEXT", "café".as_bytes())],
            "STRING is Latin-1"
        );
    }

    /// The one test that touches the real clipboard (the VM's or a CI runner's), so nothing races it.
    #[gpui_kit::test]
    fn every_representation_round_trips_through_the_system_clipboard(cx: &mut gpui_kit::TestAppContext) {
        let png = include_bytes!("../../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
        let binary = include_bytes!("../../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin");
        let parts = vec![
            part("text/plain", "héllo ✓".as_bytes()),
            part("text/html", b"<b>hi</b>"),
            part("image/png", png),
            part("application/x-ngterm-test", binary),
        ];
        cx.update(|cx| {
            write_mimes(parts.clone(), cx).unwrap();
            assert_eq!(
                mime_types(cx).unwrap(),
                ["text/plain", "text/html", "image/png", "application/x-ngterm-test"]
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
                read_mimes(&wanted, cx).unwrap(),
                [parts[3].clone(), parts[2].clone(), parts[0].clone(), parts[1].clone()]
            );
            assert_eq!(
                read_mimes(&["application/x-NGTERM-test".into()], cx).unwrap(),
                [part("application/x-NGTERM-test", binary)],
                "registered format names are case insensitive"
            );
            write_mimes(vec![part("image/png", png)], cx).unwrap();
            assert_eq!(mime_types(cx).unwrap(), ["image/png"]);
            assert!(
                read_mimes(&["text/plain".into()], cx).unwrap().is_empty(),
                "an image-only write keeps no text"
            );
            write_mimes(Vec::new(), cx).unwrap();
            assert!(mime_types(cx).unwrap().is_empty());

            let missing = format!("application/x-ngterm-missing-{}", uuid::Uuid::new_v4());
            let is_registered = || (0xC000..=0xFFFF).any(|id| format_name(id).as_deref() == Some(missing.as_str()));
            assert!(!is_registered(), "the unique test format must not exist yet");
            assert!(read_mimes(std::slice::from_ref(&missing), cx).unwrap().is_empty());
            assert!(
                !is_registered(),
                "reading an absent MIME must not allocate a session-wide atom"
            );

            text_aliases_store_and_read_real_text(cx);
            write_mimes(Vec::new(), cx).unwrap();
        });
    }
}
