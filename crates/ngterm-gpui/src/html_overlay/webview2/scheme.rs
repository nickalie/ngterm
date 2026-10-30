//! The pages' own addresses on `WebView2`. A granted file page and a page shown for another machine load through
//! custom protocols wry registers on that page alone, which `WebView2` serves only under an `https://<scheme>.<host>/`
//! address of its own (wry's workaround for `WebView2`'s custom schemes): a granted file as
//! `https://ngterm-file.localhost/C:/dir/page.html`, a replica's file as `https://ngterm-page.<page id>/path`. Every
//! address the web view reports is read back in the app's spelling (a granted file's `file:` URI, a replica's
//! `ngterm-page:` one) before the shared navigation policy or the model sees it, as the WPE host reads its grant
//! scheme back (`policy::app_uri`). A URL page registers neither protocol, so those names reach nothing of the app's
//! from it, and the policy refuses them as the file addresses they read as.

use ngterm_core::html::REPLICA_SCHEME;
use url::Url;
use uuid::Uuid;

use super::super::policy::GRANT_SCHEME;

/// The scheme `WebView2` serves the custom protocols under: a secure context, as a file page is on macOS.
const ENGINE_SCHEME: &str = "https";
/// The grant protocol's one host.
const GRANT_HOST: &str = "localhost";

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
/// one, anything else as it is.
#[must_use]
pub fn app_uri(engine: &str) -> String {
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
