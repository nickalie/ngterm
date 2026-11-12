//! The Windows clipboard's names for MIME types and its `HTML Format` envelope (`CF_HTML`), pure so every host tests
//! them: `text/plain` is `CF_UNICODETEXT`, `text/html` the `HTML Format` fragment, `image/png` the `PNG` format other
//! Windows programs exchange, and any other MIME type a format registered under its own name.

/// The registered format holding PNG data (browsers, Office, the Snipping Tool).
pub const PNG_FORMAT: &str = "PNG";
/// The registered format holding HTML (`CF_HTML`).
pub const HTML_FORMAT: &str = "HTML Format";

/// Ghostty's other names for text (`terminal.clipboard.isTextMime`), as another program may register them as formats
/// of their own: ngterm stores its text as `CF_UNICODETEXT` and reads these only when that is absent.
pub const TEXT_ALIASES: [&str; 4] = ["text/plain;charset=utf-8", "UTF8_STRING", "TEXT", "STRING"];

/// How a MIME type is stored on the Windows clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    /// `CF_UNICODETEXT`: UTF-16, NUL-terminated.
    UnicodeText,
    /// A registered format under this name; `html` wraps the data in the `CF_HTML` envelope.
    Registered { name: String, html: bool },
}

/// The format `mime` is stored as.
#[must_use]
pub fn format_of(mime: &str) -> Format {
    match mime {
        "text/plain" => Format::UnicodeText,
        "text/html" => Format::Registered {
            name: HTML_FORMAT.into(),
            html: true,
        },
        "image/png" => Format::Registered {
            name: PNG_FORMAT.into(),
            html: false,
        },
        other => Format::Registered {
            name: other.into(),
            html: false,
        },
    }
}

/// The MIME type a registered format's `name` lists as; `None` for names that are no MIME type.
#[must_use]
pub fn mime_of_registered(name: &str) -> Option<String> {
    match name {
        HTML_FORMAT => Some("text/html".into()),
        PNG_FORMAT => Some("image/png".into()),
        name if TEXT_ALIASES.iter().any(|alias| alias.eq_ignore_ascii_case(name)) => Some("text/plain".into()),
        name if name.contains('/') && !name.contains(char::is_whitespace) => Some(name.to_owned()),
        _ => None,
    }
}

/// `text` as `CF_UNICODETEXT` bytes: UTF-16LE with a terminating NUL.
#[must_use]
pub fn unicode_text(text: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(text)
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A registered text alias's data as UTF-8, up to the first NUL: `STRING` is Latin-1 (ICCCM), the others UTF-8.
#[must_use]
pub fn alias_text(alias: &str, data: &[u8]) -> Vec<u8> {
    let data = data.split(|&byte| byte == 0).next().unwrap_or_default();
    if alias.eq_ignore_ascii_case("STRING") {
        data.iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
            .into_bytes()
    } else {
        data.to_vec()
    }
}

/// `CF_UNICODETEXT` bytes as UTF-8, up to the first NUL.
#[must_use]
pub fn from_unicode_text(bytes: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&unit| u16::from_le_bytes(unit))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units).into_bytes()
}

const HEADER: &str = "Version:0.9\r\nStartHTML:{SH}\r\nEndHTML:{EH}\r\nStartFragment:{SF}\r\nEndFragment:{EF}\r\n";
const PREFIX: &str = "<html><body>\r\n<!--StartFragment-->";
const SUFFIX: &str = "<!--EndFragment-->\r\n</body></html>";

/// `fragment` in the `CF_HTML` envelope, its offsets in ten digits.
#[must_use]
pub fn wrap(fragment: &[u8]) -> Vec<u8> {
    let header_len = HEADER.len() - 4 * "{XX}".len() + 4 * 10;
    let start_html = header_len;
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + SUFFIX.len();
    let header = HEADER
        .replace("{SH}", &format!("{start_html:010}"))
        .replace("{EH}", &format!("{end_html:010}"))
        .replace("{SF}", &format!("{start_fragment:010}"))
        .replace("{EF}", &format!("{end_fragment:010}"));
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(PREFIX.as_bytes());
    bytes.extend_from_slice(fragment);
    bytes.extend_from_slice(SUFFIX.as_bytes());
    bytes
}

/// The fragment a `CF_HTML` envelope carries (its `StartFragment`..`EndFragment`), else the whole data without a
/// trailing NUL.
#[must_use]
pub fn unwrap(data: &[u8]) -> Vec<u8> {
    let data = data.strip_suffix(&[0]).unwrap_or(data);
    let offset = |key: &str| -> Option<usize> {
        let text = std::str::from_utf8(&data[..data.len().min(512)]).ok().or_else(|| {
            // the header is ASCII; the fragment may not be
            let end = data
                .iter()
                .position(|&byte| byte == b'<')
                .unwrap_or(data.len())
                .min(512);
            std::str::from_utf8(&data[..end]).ok()
        })?;
        let start = text.find(key)? + key.len();
        text[start..].lines().next()?.trim().parse().ok()
    };
    match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) if start <= end && end <= data.len() => data[start..end].to_vec(),
        _ => data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_map_to_windows_formats_and_back() {
        assert_eq!(format_of("text/plain"), Format::UnicodeText);
        assert_eq!(
            format_of("text/html"),
            Format::Registered {
                name: "HTML Format".into(),
                html: true
            }
        );
        assert_eq!(
            format_of("image/png"),
            Format::Registered {
                name: "PNG".into(),
                html: false
            }
        );
        assert_eq!(
            format_of("application/x-ngterm"),
            Format::Registered {
                name: "application/x-ngterm".into(),
                html: false
            }
        );
        assert_eq!(mime_of_registered("PNG").as_deref(), Some("image/png"));
        assert_eq!(mime_of_registered("HTML Format").as_deref(), Some("text/html"));
        assert_eq!(mime_of_registered("text/x-thing").as_deref(), Some("text/x-thing"));
        assert_eq!(mime_of_registered("Rich Text Format"), None);
        // a text alias another program registered lists as the text it is
        for alias in [
            "text/plain;charset=utf-8",
            "text/plain;charset=UTF-8",
            "UTF8_STRING",
            "TEXT",
            "STRING",
        ] {
            assert_eq!(mime_of_registered(alias).as_deref(), Some("text/plain"), "{alias}");
        }
    }

    #[test]
    fn registered_text_aliases_read_as_utf8() {
        assert_eq!(alias_text("UTF8_STRING", "café\0junk".as_bytes()), "café".as_bytes());
        assert_eq!(alias_text("text/plain;charset=utf-8", "✓".as_bytes()), "✓".as_bytes());
        assert_eq!(alias_text("STRING", b"caf\xe9"), "café".as_bytes());
        assert!(alias_text("TEXT", b"").is_empty());
    }

    #[test]
    fn text_round_trips_through_utf16() {
        let stored = unicode_text("héllo ✓".as_bytes());
        assert_eq!(stored.len(), ("héllo ✓".encode_utf16().count() + 1) * 2);
        assert_eq!(from_unicode_text(&stored), "héllo ✓".as_bytes());
        assert_eq!(from_unicode_text(&unicode_text(b"")), b"");
    }

    #[test]
    fn html_round_trips_through_the_envelope_and_its_offsets_are_exact() {
        let fragment = "<b>héllo</b>".as_bytes();
        let wrapped = wrap(fragment);
        let text = String::from_utf8(wrapped.clone()).unwrap();
        let field = |key: &str| -> usize {
            let start = text.find(key).unwrap() + key.len();
            text[start..start + 10].parse().unwrap()
        };
        assert_eq!(&wrapped[field("StartFragment:")..field("EndFragment:")], fragment);
        assert_eq!(field("EndHTML:"), wrapped.len());
        assert!(text[field("StartHTML:")..].starts_with("<html>"));
        assert_eq!(unwrap(&wrapped), fragment);
        let mut terminated = wrapped;
        terminated.push(0);
        assert_eq!(unwrap(&terminated), fragment);
        assert_eq!(unwrap(b"<p>no envelope</p>"), b"<p>no envelope</p>");
        assert!(unwrap(&wrap(b"")).is_empty());
    }

    #[test]
    fn a_fragment_that_breaks_utf8_inside_the_header_window_still_unwraps() {
        // the header window ends inside a two-byte character for one of the paddings
        for pad in 0..2 {
            let fragment = format!("<p>{}{}</p>", "a".repeat(pad), "é".repeat(400));
            assert_eq!(unwrap(&wrap(fragment.as_bytes())), fragment.as_bytes(), "{pad}");
        }
        let latin1 = b"<b>caf\xe9</b>";
        assert_eq!(unwrap(&wrap(latin1)), latin1);
        assert_eq!(unwrap(b"caf\xe9 no markup"), b"caf\xe9 no markup");
    }
}
