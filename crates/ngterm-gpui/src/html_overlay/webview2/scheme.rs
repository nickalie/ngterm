//! The pages' own addresses on `WebView2`. A file page and a page shown for another machine load through custom
//! protocols wry registers on that page alone, which `WebView2` serves only under an `https://<scheme>.<host>/` address
//! of its own (wry's workaround for `WebView2`'s custom schemes): a granted file as
//! `https://ngterm-file.localhost/C:/dir/page.html`, one on a network share (`\\server\share\dir\page.html`, its
//! `file:` URI's host the server) as `https://ngterm-file.localhost/UNC/server/share/dir/page.html` (the server as the
//! segment after `UNC`, Windows' own spelling of a share's path, which no drive path's first segment `X:` is), a
//! replica's file as `https://ngterm-page.<page id>/path`, the text of a file page without a grant as
//! `https://ngterm-text.<page id>/`. Every address the web view reports is read back in the app's spelling (a granted
//! file's `file:` URI, a replica's `ngterm-page:` one, the text's `about:blank`, where the WPE host and upstream load
//! it) before the shared navigation policy or the model sees it, as the WPE host reads its grant scheme back
//! (`policy::app_uri`). A URL page registers none of the protocols, so those names reach nothing of the app's from it,
//! and the policy refuses them as the file addresses they read as.
//!
//! The text loads through a protocol rather than `NavigateToString`: a string's document never reported its title
//! (granted and URL pages did, through the same events: the page report waits on an address, and a string's stays
//! the `about:blank` the web view started at), each string load added a history entry where `WebKit`'s reload of its
//! `about:blank` text adds none, and a string is capped at 2 MB. A load of the text's one address over the shown one
//! replaces it, as a same-address load does in a browser.

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

/// The address `WebView2` loads granted file `path` from; `None` for a path that is not absolute. A share's file
/// carries its server after [`UNC_SEGMENT`] (module docs), spelled as a `file:` URI's host (lowercased, punycoded),
/// which the grant's check reads alike (`policy::granted_file`).
#[must_use]
pub fn granted_address(path: &str) -> Option<String> {
    let file = Url::from_file_path(path).ok()?;
    let server = file
        .host_str()
        .filter(|server| !server.is_empty())
        .map(|server| format!("/{UNC_SEGMENT}/{server}"))
        .unwrap_or_default();
    Some(format!(
        "{ENGINE_SCHEME}://{GRANT_SCHEME}.{GRANT_HOST}{server}{}",
        file.path()
    ))
}

/// The path segment a share's server follows in a granted address.
const UNC_SEGMENT: &str = "UNC";

/// The `file:` URI of a granted address's path `rest` (what follows its host's `/`): a share's file by its server, any
/// other as the drive path it spells. Only a granted share's page loads under `UNC/`: a root-relative address another
/// page asks for there reads as a share's file too, which the grant then refuses (`policy::granted_file`).
fn granted_file_uri(rest: &str) -> String {
    let share = strip_prefix_ignoring_case(rest, &format!("{UNC_SEGMENT}/"))
        .filter(|share| share.split('/').next().is_some_and(|server| !server.is_empty()));
    match share {
        Some(share) => format!("file://{share}"),
        None => format!("file:///{rest}"),
    }
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
        return granted_file_uri(rest);
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
        Some(rest) => granted_file_uri(rest),
        None => request.to_owned(),
    }
}

/// Whether `engine` reaches the network: an HTTP(S) address none of the app's protocols serves.
#[must_use]
pub fn is_network(engine: &str) -> bool {
    let web = ["http://", "https://"]
        .iter()
        .any(|scheme| strip_prefix_ignoring_case(engine, scheme).is_some());
    let own = [GRANT_PROTOCOL, REPLICA_PROTOCOL, TEXT_PROTOCOL]
        .iter()
        .any(|protocol| strip_prefix_ignoring_case(engine, &format!("{ENGINE_SCHEME}://{protocol}.")).is_some());
    web && !own
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
    fn a_shares_file_loads_with_its_server_and_reads_back_as_its_file() {
        let address = granted_address(r"\\server\share\my site\index.html").unwrap();
        assert_eq!(
            address,
            "https://ngterm-file.localhost/UNC/server/share/my%20site/index.html"
        );
        let app = app_uri(&address);
        assert_eq!(app, "file://server/share/my%20site/index.html");
        let path = Url::parse(&app).unwrap().to_file_path().unwrap();
        assert_eq!(path, std::path::Path::new(r"\\server\share\my site\index.html"));
        assert_eq!(
            request_app_uri("ngterm-file://localhost/UNC/server/share/a/b.css"),
            "file://server/share/a/b.css"
        );
        let verbatim = granted_address(r"\\?\UNC\server\share\a.html").unwrap();
        assert_eq!(app_uri(&verbatim), "file://server/share/a.html");
        // a drive's file never reads as a share's, nor does a `UNC` segment naming no server
        assert_eq!(
            app_uri("https://ngterm-file.localhost/C:/UNC/x/a.html"),
            "file:///C:/UNC/x/a.html"
        );
        assert_eq!(app_uri("https://ngterm-file.localhost/UNC//x"), "file:///UNC//x");
        assert_eq!(app_uri("https://ngterm-file.localhost/unc/srv/s/a"), "file://srv/s/a");
    }

    /// A drive letter mapped for a test's span by `map` (`subst`, `net use`) and removed on drop by `remove`.
    struct Drive {
        letter: String,
        remove: Vec<String>,
    }

    impl Drive {
        fn map(map: impl FnOnce(&str) -> Vec<String>, remove: impl FnOnce(&str) -> Vec<String>) -> Self {
            let letter = ('D'..='Z')
                .rev()
                .map(|letter| format!("{letter}:"))
                .find(|drive| !std::path::Path::new(&format!(r"{drive}\")).exists())
                .unwrap();
            let run = |args: Vec<String>| {
                let output = std::process::Command::new(&args[0]).args(&args[1..]).output().unwrap();
                assert!(output.status.success(), "{args:?}: {output:?}");
            };
            run(map(&letter));
            Self {
                remove: remove(&letter),
                letter,
            }
        }
    }

    impl Drop for Drive {
        fn drop(&mut self) {
            let _ = std::process::Command::new(&self.remove[0])
                .args(&self.remove[1..])
                .output();
        }
    }

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    /// A granted page and what it reads pass the grant inside it only, however the page names its grant: a drive's
    /// directory; this machine's administrative share of it by address (a `file:` URL drops a `localhost` host) and
    /// by the machine's name in capitals, which a page's address lowercases; a `subst` drive and a network drive
    /// mapped onto them, which the grant resolves to another volume.
    #[test]
    fn the_grant_serves_a_drives_and_a_shares_files_only_inside_it() {
        use crate::html_overlay::policy::{GrantRoot, OUTSIDE_GRANT, granted_file};

        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        std::fs::create_dir(canonical.join("site")).unwrap();
        std::fs::write(canonical.join("site").join("index.html"), "x").unwrap();
        std::fs::write(canonical.join("outside.html"), "x").unwrap();
        let plain = canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap().to_owned();
        let (drive, rest) = plain.split_once(r":\").unwrap();
        let share = format!(r"\\127.0.0.1\{drive}$\{rest}");
        let computer = std::env::var("COMPUTERNAME").unwrap().to_uppercase();
        assert_ne!(computer, computer.to_lowercase());
        let named = format!(r"\\{computer}\{drive}$\{rest}");
        let named_root = std::fs::canonicalize(format!(r"{named}\site")).unwrap();
        assert!(
            named_root.to_string_lossy().contains(&computer),
            "the grant keeps the server's capitals: {}",
            named_root.display()
        );
        let substed = Drive::map(
            |letter| words(&format!("subst {letter} {plain}")),
            |letter| words(&format!("subst {letter} /D")),
        );
        let mapped = Drive::map(
            |letter| words(&format!(r"net use {letter} \\127.0.0.1\{drive}$ /persistent:no")),
            |letter| words(&format!("net use {letter} /delete /y")),
        );
        let substed_base = substed.letter.clone();
        let mapped_base = format!(r"{}\{rest}", mapped.letter);
        for base in [&plain, &share, &named, &substed_base, &mapped_base] {
            let root = GrantRoot::new(std::path::Path::new(&format!(r"{base}\site"))).unwrap();
            let read = |file: &str| {
                let app = app_uri(&granted_address(&format!(r"{base}\{file}")).unwrap());
                let requested = Url::parse(&app).unwrap().to_file_path().unwrap();
                granted_file(Some(&root), Some(&requested))
            };
            let index = std::fs::canonicalize(format!(r"{base}\site\index.html")).unwrap();
            assert_eq!(read(r"site\index.html"), Ok(index), "{base}");
            assert_eq!(read("outside.html"), Err(OUTSIDE_GRANT), "{base}");
            let elsewhere = std::path::Path::new(r"\\ngterm-nowhere.invalid\share\site\index.html");
            assert_eq!(granted_file(Some(&root), Some(elsewhere)), Err(OUTSIDE_GRANT), "{base}");
        }
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
    fn only_addresses_off_the_apps_protocols_reach_the_network() {
        assert!(is_network("http://127.0.0.1:8000/a.html"));
        assert!(is_network("HTTPS://example.com/"));
        assert!(!is_network("https://ngterm-file.localhost/C:/a.html"));
        assert!(!is_network(&replica_address(Uuid::new_v4(), "/a.html").unwrap()));
        assert!(!is_network(&text_address(Uuid::new_v4())));
        assert!(!is_network("about:blank"));
        assert!(!is_network("data:text/html,x"));
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
