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
}
