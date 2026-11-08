//! Native custom scheme addresses, kept distinct from network URLs.
use uuid::Uuid;
pub const GRANT_PROTOCOL: &str = super::super::policy::GRANT_SCHEME;
pub const TEXT_PROTOCOL: &str = "ngterm-text";
pub const REPLICA_PROTOCOL: &str = ngterm_core::html::REPLICA_SCHEME;
pub fn text_address(page: Uuid) -> String {
    format!("{TEXT_PROTOCOL}://{page}/")
}
pub fn is_text_resource(uri: &str) -> bool {
    uri.starts_with("ngterm-text:") && !requests_text_document(uri)
}
pub fn requests_text_document(uri: &str) -> bool {
    url::Url::parse(uri).is_ok_and(|url| url.scheme() == TEXT_PROTOCOL && url.path() == "/" && url.query().is_none())
}
pub fn granted_address(path: &str) -> Option<String> {
    let file = url::Url::from_file_path(path).ok()?;
    Some(format!("{GRANT_PROTOCOL}://localhost{}", file.path()))
}
pub fn replica_address(page: Uuid, path: &str) -> Option<String> {
    ngterm_core::html::replica_url(page, path).map(|url| url.to_string())
}
pub fn app_uri(engine: &str) -> String {
    if let Some(rest) = engine.strip_prefix("ngterm-file://localhost") {
        return format!("file://{rest}");
    }
    if requests_text_document(engine) {
        return url::Url::parse(engine)
            .ok()
            .and_then(|url| url.fragment().map(|fragment| format!("about:blank#{fragment}")))
            .unwrap_or_else(|| "about:blank".into());
    }
    engine.into()
}
pub fn request_app_uri(uri: &str) -> String {
    app_uri(uri)
}
#[cfg(test)]
pub fn is_network(uri: &str) -> bool {
    uri.starts_with("https:") || uri.starts_with("http:")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_addresses_round_trip_and_escape_reserved_characters() {
        let address = granted_address("/tmp/site/a #b.html").unwrap();
        assert_eq!(app_uri(&address), "file:///tmp/site/a%20%23b.html");
        assert!(!is_network(&address));
        assert!(!requests_text_document("ngterm-text://page/other"));
    }

    #[test]
    fn a_relative_path_has_no_granted_address() {
        assert_eq!(granted_address("site/a.html"), None);
        assert_eq!(
            granted_address("/tmp/a.html").as_deref(),
            Some("ngterm-file://localhost/tmp/a.html")
        );
    }

    #[test]
    fn a_text_page_is_its_document_and_everything_under_it_a_resource() {
        let document = text_address(Uuid::nil());
        assert_eq!(document, "ngterm-text://00000000-0000-0000-0000-000000000000/");
        assert!(requests_text_document(&document));
        assert!(requests_text_document(&format!("{document}#line-3")));
        assert!(!is_text_resource(&document));
        for resource in [format!("{document}logo.png"), format!("{document}?page=2")] {
            assert!(!requests_text_document(&resource), "{resource}");
            assert!(is_text_resource(&resource), "{resource}");
        }
    }

    #[test]
    fn other_addresses_are_no_text_page() {
        for uri in ["https://example.com/", "ngterm-file://localhost/", "not an address"] {
            assert!(!requests_text_document(uri), "{uri}");
            assert!(!is_text_resource(uri), "{uri}");
        }
    }

    #[test]
    fn a_replica_address_is_rooted_at_the_page_and_escapes_its_path() {
        let page = "ngterm-page://00000000-0000-0000-0000-000000000000";
        assert_eq!(
            replica_address(Uuid::nil(), "/srv/a b%.html").unwrap(),
            format!("{page}/srv/a%20b%25.html")
        );
        assert_eq!(
            replica_address(Uuid::nil(), "a.html").unwrap(),
            format!("{page}/a.html")
        );
    }

    #[test]
    fn the_app_sees_a_text_document_as_a_blank_page_keeping_its_fragment() {
        let document = text_address(Uuid::nil());
        assert_eq!(app_uri(&document), "about:blank");
        assert_eq!(app_uri(&format!("{document}#line-3")), "about:blank#line-3");
        assert_eq!(request_app_uri(&format!("{document}#line-3")), "about:blank#line-3");
    }

    #[test]
    fn the_app_sees_other_addresses_as_they_are() {
        for uri in [
            "https://example.com/a?b#c",
            "ngterm-text://page/logo.png",
            "about:blank",
        ] {
            assert_eq!(app_uri(uri), uri);
            assert_eq!(request_app_uri(uri), uri);
        }
    }
}
