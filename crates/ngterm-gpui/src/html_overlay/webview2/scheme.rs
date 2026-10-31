//! The pages' own addresses on `WebView2`. A file page and a page shown for another machine load through custom
//! protocols wry registers on that page alone, which `WebView2` serves only under an `https://<scheme>.<host>/`
//! address of its own (wry's workaround for `WebView2`'s custom schemes): a granted file as
//! `https://ngterm-file.localhost/C:/dir/page.html`, a replica's file as `https://ngterm-page.<page id>/path`, the
//! text of a file page without a grant as `https://ngterm-text.<page id>/`. Every address the web view reports is read
//! back in the app's spelling (a granted file's `file:` URI, a replica's `ngterm-page:` one, the text's `about:blank`,
//! where the WPE host and upstream load it) before the shared navigation policy or the model sees it, as the WPE host
//! reads its grant scheme back (`policy::app_uri`). A URL page registers none of the protocols, so those names reach
//! nothing of the app's from it, and the policy refuses them as the file addresses they read as.
//!
//! The text loads through a protocol rather than `NavigateToString`: `WebView2` reports no address for a string's
//! document (it stays `about:blank`, so no `SourceChanged` comes and the page never reports its title), each string
//! load adds a history entry where `WebKit`'s `about:blank` reload adds none, and a string is capped at 2 MB. A load of
//! the text's one address over the shown one replaces it, as a same-address load does in a browser.

use ngterm_core::html::REPLICA_SCHEME;
use url::Url;
use uuid::Uuid;

use super::super::policy::GRANT_SCHEME;

/// The scheme `WebView2` serves the custom protocols under: a secure context, as a file page is on macOS.
const ENGINE_SCHEME: &str = "https";
/// The grant protocol's one host.
const GRANT_HOST: &str = "localhost";

/// The text protocol's address of page `page`: its one document.
#[must_use]
pub fn text_address(page: Uuid) -> String {
    format!("{ENGINE_SCHEME}://{TEXT_PROTOCOL}.{}/", page.hyphenated())
}

/// Whether `engine` is an address under a text protocol's host other than its document's: nothing is served there
/// (the WPE host's `about:blank` document resolves no relative address either).
#[must_use]
pub fn is_text_resource(engine: &str) -> bool {
    text_rest(engine).is_some_and(|rest| !is_text_document(rest))
}

/// What follows `https://ngterm-text.` in `engine`.
fn text_rest(engine: &str) -> Option<&str> {
    strip_prefix_ignoring_case(engine, &format!("{ENGINE_SCHEME}://{TEXT_PROTOCOL}."))
}

/// Whether `rest`, a text address after its scheme and protocol name, names the document: `<page id>/`, maybe with
/// a fragment.
fn is_text_document(rest: &str) -> bool {
    let address = rest.split_once('#').map_or(rest, |(address, _)| address);
    address
        .split_once('/')
        .is_some_and(|(host, path)| !host.is_empty() && path.is_empty())
}

/// The address `WebView2` loads granted file `path` from; `None` for a path that is not absolute.
#[must_use]
pub fn granted_address(path: &str) -> Option<String> {
    let file = Url::from_file_path(path).ok()?;
    Some(format!("{ENGINE_SCHEME}://{GRANT_SCHEME}.{GRANT_HOST}{}", file.path()))
}

/// The address `WebView2` loads file `path` of replica page `page` from (upstream's `HtmlPageSchemeHandler.url`).
#[must_use]
pub fn replica_address(page: Uuid, path: &str) -> Option<String> {
    let url = ngterm_core::html::replica_url(page, path)?;
    let rest = url
        .as_str()
        .strip_prefix(REPLICA_SCHEME)?
        .strip_prefix("://")?
        .to_owned();
    Some(format!("{ENGINE_SCHEME}://{REPLICA_SCHEME}.{rest}"))
}

/// An address the web view reports, as the app spells it: a granted file's `file:` URI, a replica's `ngterm-page:`
/// one, a page's text `about:blank` (its fragment kept), anything else as it is.
#[must_use]
pub fn app_uri(engine: &str) -> String {
    if let Some(rest) = text_rest(engine).filter(|rest| is_text_document(rest)) {
        return match rest.split_once('#') {
            Some((_, fragment)) => format!("about:blank#{fragment}"),
            None => "about:blank".to_owned(),
        };
    }
    let grant = format!("{ENGINE_SCHEME}://{GRANT_SCHEME}.{GRANT_HOST}/");
    if let Some(rest) = strip_prefix_ignoring_case(engine, &grant) {
        return format!("file:///{rest}");
    }
    let replica = format!("{ENGINE_SCHEME}://{REPLICA_SCHEME}.");
    if let Some(rest) = strip_prefix_ignoring_case(engine, &replica) {
        return format!("{REPLICA_SCHEME}://{rest}");
    }
    engine.to_owned()
}

/// A protocol request's address as wry hands it to the handler (its workaround undone: `ngterm-file://localhost/...`,
/// `ngterm-page://<page id>/...`), as the app spells it.
#[must_use]
pub fn request_app_uri(request: &str) -> String {
    let grant = format!("{GRANT_SCHEME}://{GRANT_HOST}/");
    match strip_prefix_ignoring_case(request, &grant) {
        Some(rest) => format!("file:///{rest}"),
        None => request.to_owned(),
    }
}

/// The protocol names wry registers, by what a page loads through them.
pub const GRANT_PROTOCOL: &str = GRANT_SCHEME;
pub const REPLICA_PROTOCOL: &str = REPLICA_SCHEME;
pub const TEXT_PROTOCOL: &str = "ngterm-text";

/// Whether a text protocol request (`ngterm-text://<page id>/...`, as wry hands it to the handler) asks for the
/// document.
#[must_use]
pub fn requests_text_document(request: &str) -> bool {
    strip_prefix_ignoring_case(request, &format!("{TEXT_PROTOCOL}://"))
        .map(|rest| rest.split_once('?').map_or(rest, |(address, _)| address))
        .is_some_and(is_text_document)
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_file_loads_under_the_grant_protocol_and_reads_back_as_its_file() {
        let address = granted_address(r"C:\Users\u\my site\index.html").unwrap();
        assert_eq!(address, "https://ngterm-file.localhost/C:/Users/u/my%20site/index.html");
        let app = app_uri(&address);
        assert_eq!(app, "file:///C:/Users/u/my%20site/index.html");
        let path = Url::parse(&app).unwrap().to_file_path().unwrap();
        assert_eq!(path, std::path::Path::new(r"C:\Users\u\my site\index.html"));
        assert!(granted_address("relative.html").is_none());
    }

    #[test]
    fn a_replica_loads_under_its_page_host_and_reads_back_as_the_replica_scheme() {
        let page = Uuid::new_v4();
        let address = replica_address(page, "/Users/u/docs/a b.html").unwrap();
        assert_eq!(
            address,
            format!("https://ngterm-page.{}/Users/u/docs/a%20b.html", page.hyphenated())
        );
        let app = Url::parse(&app_uri(&address)).unwrap();
        assert!(ngterm_core::html::replica_url_of(&app, page));
        assert_eq!(
            ngterm_core::html::replica_path(&app).as_deref(),
            Some("/Users/u/docs/a b.html")
        );
    }

    #[test]
    fn a_text_page_loads_under_its_page_host_and_reads_back_as_about_blank() {
        let page = Uuid::new_v4();
        let address = text_address(page);
        assert_eq!(address, format!("https://ngterm-text.{}/", page.hyphenated()));
        assert_eq!(app_uri(&address), "about:blank");
        assert_eq!(app_uri(&format!("{address}#part")), "about:blank#part");
        assert!(!is_text_resource(&address));
        let other = format!("{address}other.html");
        assert!(is_text_resource(&other));
        assert_eq!(app_uri(&other), other, "only the document reads as about:blank");
        assert!(!is_text_resource("https://example.com/"));
        let request = format!("ngterm-text://{}", page.hyphenated());
        assert!(requests_text_document(&format!("{request}/")));
        assert!(requests_text_document(&format!("{request}/?x=1")));
        assert!(!requests_text_document(&format!("{request}/style.css")));
        assert!(!requests_text_document("ngterm-file://localhost/"));
    }

    #[test]
    fn other_addresses_read_as_they_are() {
        for address in ["https://example.com/a", "about:blank", "http://localhost:5173/"] {
            assert_eq!(app_uri(address), address);
        }
        assert_eq!(
            app_uri("HTTPS://NGTERM-FILE.LOCALHOST/C:/a.html"),
            "file:///C:/a.html",
            "the engine's spelling of the host"
        );
    }

    #[test]
    fn a_protocol_request_reads_as_the_app_spells_it() {
        assert_eq!(
            request_app_uri("ngterm-file://localhost/C:/a/b.css"),
            "file:///C:/a/b.css"
        );
        let page = Uuid::new_v4();
        let replica = format!("ngterm-page://{}/x.png", page.hyphenated());
        assert_eq!(request_app_uri(&replica), replica);
    }
}
