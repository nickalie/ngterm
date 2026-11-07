//! The page a clicked HTML file shows (upstream's `DocumentPage.swift`): terminal output named the file, so it is
//! untrusted. Its text is loaded as the page with no file grant, no bridge and no page JavaScript, and it loads nothing
//! from the network: a CSP right after its doctype keeps every subresource to `data:` and inline styles, and the
//! frontend installs a filter for what the CSP does not cover (link hints) before the page loads.

/// The local file extensions a click opens as a document page (upstream's `DocumentPage.extensions`).
pub const EXTENSIONS: [&str; 2] = ["html", "htm"];
/// The most a document page reads of its file.
pub const MAX_BYTES: usize = 4 << 20;
/// The policy every document page carries: subresource loads never pass the navigation policy, so this keeps them to
/// `data:` and inline styles (upstream's `DocumentPage.policy`).
pub const POLICY: &str = "<meta http-equiv=\"Content-Security-Policy\" \
                          content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; \
                          media-src data:\">";
/// Turns off the DNS prefetch a Chromium engine (`WebView2`) does for the page's links, which no CSP covers.
pub const NO_DNS_PREFETCH: &str = "<meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">";

/// The page a file's bytes make (upstream's `DocumentPage.page`): its text with [`POLICY`] placed by `with_policy`,
/// `None` when it holds a NUL or is not UTF-8 (`text_page::validate`). A leading byte-order mark is dropped, as
/// Foundation's UTF-8 decoding drops it, so a doctype after it still counts as leading.
#[must_use]
pub fn page(data: Vec<u8>) -> Option<String> {
    super::text_page::validate(data).map(|html| with_policy(without_bom(&html), POLICY))
}

/// [`page`] for a Chromium engine (`WebView2`), whose link hints neither the CSP nor a request filter reaches: every
/// `<link>` element is made inert (`without_links`) and [`NO_DNS_PREFETCH`] goes in beside the policy.
#[must_use]
pub fn chromium_page(data: Vec<u8>) -> Option<String> {
    super::text_page::validate(data).map(|html| {
        with_policy(
            &without_links(without_bom(&html)),
            &format!("{POLICY}{NO_DNS_PREFETCH}"),
        )
    })
}

fn without_bom(html: &str) -> &str {
    html.strip_prefix('\u{feff}').unwrap_or(html)
}

/// `html` with `head` right after a leading doctype, since any tag ahead of it puts the page in quirks mode, or first
/// when there is none (upstream's `withPolicy`). Only HTML's ASCII whitespace may come before the doctype: anything
/// else starts the body, where the parser ignores a CSP meta. A doctype with no closing `>` is none.
fn with_policy(html: &str, head: &str) -> String {
    let start = html
        .find(|character| !matches!(character, '\t' | '\n' | '\u{c}' | '\r' | ' '))
        .unwrap_or(html.len());
    let rest = &html[start..];
    let doctype = rest
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<!doctype"));
    match rest.find('>').filter(|_| doctype) {
        Some(end) => {
            let split = start + end + 1;
            format!("{}{head}{}", &html[..split], &html[split..])
        }
        None => format!("{head}{html}"),
    }
}

/// `html` with every `<link>` start tag renamed to an element that loads nothing (`<ngterm-inert-link`), its
/// attributes kept: a Chromium engine preconnects and resolves names for `preconnect` and `dns-prefetch` links without
/// asking the CSP or the request filter. Under [`POLICY`] no `<link>` could load anything the page shows (a stylesheet
/// or a preload is refused, an icon is never drawn), so renaming them all loses nothing. The tag name is matched in any
/// case and must end there (whitespace, `/` or `>`); a `<link` inside text the parser keeps verbatim (a `<textarea>`,
/// a `<title>`) is renamed too, the price of not parsing the page.
fn without_links(html: &str) -> String {
    const TAG: &[u8] = b"<link";
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut copied = 0;
    let mut at = 0;
    while let Some(found) = bytes[at..]
        .windows(TAG.len())
        .position(|window| window.eq_ignore_ascii_case(TAG))
    {
        let start = at + found;
        let end = start + TAG.len();
        at = end;
        let ends_name = bytes
            .get(end)
            .is_none_or(|next| matches!(next, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'/' | b'>'));
        if ends_name {
            out.push_str(&html[copied..start]);
            out.push_str("<ngterm-inert-link");
            copied = end;
        }
    }
    out.push_str(&html[copied..]);
    out
}

#[cfg(test)]
#[path = "document_page_tests.rs"]
mod tests;
