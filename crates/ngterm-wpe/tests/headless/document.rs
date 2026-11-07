//! A clicked HTML document's page (upstream's `HtmlOverlayRegistryTests.testAClickedDocumentLoadsNothingFromTheNetwork`)
//! in real `WebKit`: the file's own markup behind the policy `ngterm_core::html::document_page` places after its
//! doctype, page JavaScript off, loaded behind the engine's network filter ([`ViewSpec::block_network`]). A local
//! listener counts every connection the page makes: the CSP keeps subresources off the network, and the filter keeps
//! the link hints (preconnect, DNS prefetch) the CSP does not cover off it too.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ngterm_wpe::{Geometry, Load, Settings, ViewId, ViewSpec};

use crate::cases::Context;
use crate::support::{Harness, Outcome, ensure};

/// `document_page::POLICY`.
const POLICY: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; \
                      style-src 'unsafe-inline'; font-src data:; media-src data:\">";

/// A local server that counts every connection and records the request line each sends, answering each with an empty
/// page.
struct Listener {
    origin: String,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Listener {
    fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| format!("no local port: {err}"))?;
        let port = listener.local_addr().map_err(|err| err.to_string())?.port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (count, record) = (Arc::clone(&connections), Arc::clone(&requests));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                count.fetch_add(1, Ordering::SeqCst);
                let record = Arc::clone(&record);
                std::thread::spawn(move || answer(stream, &record));
            }
        });
        Ok(Self {
            origin: format!("http://127.0.0.1:{port}"),
            connections,
            requests,
        })
    }

    /// The connections made so far, and the requests sent on them.
    fn seen(&self) -> (usize, Vec<String>) {
        (
            self.connections.load(Ordering::SeqCst),
            self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        )
    }
}

/// Records the request line `stream` sends, if any, and answers it with an empty page.
fn answer(mut stream: TcpStream, requests: &Mutex<Vec<String>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_ok_and(|read| read > 0) {
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.trim().to_owned());
    }
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

/// Upstream's beacon page: a link with a ping, a stylesheet, a prefetch, CSS backgrounds and fonts, an image, a video,
/// a frame, a preconnect, a DNS prefetch and an SVG `<use>`, all on the listener; the link fills the top left corner.
fn beacon(origin: &str) -> String {
    format!(
        "<!doctype html>{POLICY}<title>B</title>\
         <a id=\"p\" href=\"{origin}/link\" ping=\"{origin}/ping\" \
         style=\"position:fixed;left:0;top:0;width:100px;height:100px;display:block\">x</a>\
         <link rel=\"stylesheet\" href=\"{origin}/css\"><link rel=\"prefetch\" href=\"{origin}/prefetch\">\
         <style>body {{ background: url({origin}/bg) }} @font-face {{ font-family: f; src: url({origin}/font) }} \
         p {{ font-family: f }}</style><p>text</p>\
         <img src=\"{origin}/img\"><video src=\"{origin}/video\"></video><iframe src=\"{origin}/frame\"></iframe>\
         <link rel=\"preconnect\" href=\"{origin}\"><link rel=\"dns-prefetch\" href=\"{origin}\">\
         <svg><use href=\"{origin}/sprite.svg#a\"/></svg>"
    )
}

fn view(h: &Harness, block_network: bool) -> ViewId {
    h.view_with(ViewSpec {
        session: h.ephemeral(),
        settings: Settings::overlay(false),
        scripts: Vec::new(),
        message_handlers: Vec::new(),
        zoom: 1.0,
        background: None,
        geometry: Geometry {
            width: 320,
            height: 240,
            scale: 1.0,
        },
        block_network,
    })
}

/// Opening the page contacts nothing; clicking its link sends nothing, even with the navigation allowed (the app's
/// policy would cancel it, and offer the browser): the filter refuses the navigation, and the CSP the ping. As upstream
/// notes, the click may still connect to the link's host before the policy answers: `WebKit` preconnects for a
/// navigation it starts.
pub fn silent(h: &Harness, _: &Context) -> Outcome {
    let listener = Listener::start()?;
    let view = view(h, true);
    // asked for before the filter compiled: the load waits for it
    h.load(
        view,
        Load::Html {
            html: beacon(&listener.origin),
            base_uri: None,
        },
    )?;
    h.title(view, "B")?;
    h.idle(Duration::from_millis(500), |_| None)?;
    let seen = listener.seen();
    ensure(seen.0 == 0, || {
        format!("opening the page contacted the listener: {seen:?}")
    })?;
    h.click(view, 10.0, 10.0);
    h.idle(Duration::from_millis(500), |_| None)?;
    let (_, requests) = listener.seen();
    ensure(requests.is_empty(), || {
        format!("clicking the link sent requests: {requests:?}")
    })?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// The same page without the filter: the CSP alone leaves the preconnect hint, which reaches the listener, so the
/// filter is what keeps the page silent (`WebKit` checks link hints against content filters, never the CSP).
pub fn unfiltered(h: &Harness, _: &Context) -> Outcome {
    let listener = Listener::start()?;
    let view = view(h, false);
    h.load(
        view,
        Load::Html {
            html: beacon(&listener.origin),
            base_uri: None,
        },
    )?;
    h.title(view, "B")?;
    h.idle(Duration::from_millis(500), |_| None)?;
    let (connections, requests) = listener.seen();
    ensure(connections > 0, || {
        "the preconnect hint never reached the listener".to_owned()
    })?;
    ensure(requests.is_empty(), || {
        format!("the CSP let a request through: {requests:?}")
    })?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}
