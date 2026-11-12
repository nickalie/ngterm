use std::{
    fs::File,
    io::{ErrorKind, Write},
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
};

use calloop::{LoopHandle, PostAction};
use filedescriptor::Pipe;
use strum::IntoEnumIterator;
use wayland_client::{Connection, protocol::wl_data_offer::WlDataOffer};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1;

use crate::linux::platform::{PIPE_READ_TIMEOUT, read_fd_with_timeout};
use crate::linux::clipboard_mime::{listed_mimes, offered_mimes, served, source_data, source_type};
use crate::linux::{ClipboardHtml, ClipboardMimeError, ClipboardRepresentation, HTML_MIME_TYPE, take_clipboard_html};
use gpui::{ClipboardEntry, ClipboardItem, Image, ImageFormat, hash};

/// Text mime types that we'll offer to other programs.
pub(crate) const TEXT_MIME_TYPES: [&str; 3] =
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
pub(crate) const FILE_LIST_MIME_TYPE: &str = "text/uri-list";

/// Text mime types that we'll accept from other programs.
pub(crate) const ALLOWED_TEXT_MIME_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "UTF8_STRING"];

pub(crate) struct Clipboard {
    connection: Connection,
    loop_handle: LoopHandle<'static, ()>,
    self_mime: String,

    // Internal clipboard
    contents: Option<ClipboardItem>,
    primary_contents: Option<ClipboardItem>,
    // The HTML offered with them (ngterm: `with_clipboard_html`)
    html: Option<ClipboardHtml>,
    primary_html: Option<ClipboardHtml>,
    // Exactly what a per-MIME write offers (ngterm: `write_clipboard_representations`)
    representations: Option<Vec<ClipboardRepresentation>>,
    primary_representations: Option<Vec<ClipboardRepresentation>>,

    // External clipboard
    cached_read: Option<ClipboardItem>,
    current_offer: Option<DataOffer<WlDataOffer>>,
    cached_primary_read: Option<ClipboardItem>,
    current_primary_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>,
}

pub(crate) trait ReceiveData {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>);
}

impl ReceiveData for WlDataOffer {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

impl ReceiveData for ZwpPrimarySelectionOfferV1 {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

#[derive(Clone, Debug)]
/// Wrapper for `WlDataOffer` and `ZwpPrimarySelectionOfferV1`, used to help track mime types.
pub(crate) struct DataOffer<T: ReceiveData> {
    pub inner: T,
    mime_types: Vec<String>,
}

impl<T: ReceiveData> DataOffer<T> {
    pub fn new(offer: T) -> Self {
        Self {
            inner: offer,
            mime_types: Vec::new(),
        }
    }

    pub fn add_mime_type(&mut self, mime_type: String) {
        self.mime_types.push(mime_type)
    }

    fn has_mime_type(&self, mime_type: &str) -> bool {
        self.mime_types.iter().any(|t| t == mime_type)
    }

    fn read_bytes(&self, connection: &Connection, mime_type: &str) -> Option<Vec<u8>> {
        let pipe = Pipe::new().unwrap();
        self.inner.receive_data(mime_type.to_string(), unsafe {
            BorrowedFd::borrow_raw(pipe.write.as_raw_fd())
        });
        let fd = pipe.read;
        drop(pipe.write);

        connection.flush().unwrap();

        match read_fd_with_timeout(fd, PIPE_READ_TIMEOUT) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log::error!("error reading clipboard pipe: {err:?}");
                None
            }
        }
    }

    fn read_text(&self, connection: &Connection) -> Option<ClipboardItem> {
        let mime_type = self.mime_types.iter().find(|&mime_type| {
            ALLOWED_TEXT_MIME_TYPES
                .iter()
                .any(|&allowed| allowed == mime_type)
        })?;
        let bytes = self.read_bytes(connection, mime_type)?;
        let text_content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(e) => {
                log::error!("Failed to convert clipboard content to UTF-8: {}", e);
                return None;
            }
        };

        // Normalize the text to unix line endings, otherwise
        // copying from eg: firefox inserts a lot of blank
        // lines, and that is super annoying.
        let result = text_content.replace("\r\n", "\n");
        Some(ClipboardItem::new_string(result))
    }

    fn read_image(&self, connection: &Connection) -> Option<ClipboardItem> {
        for format in ImageFormat::iter() {
            let mime_type = format.mime_type();
            if !self.has_mime_type(mime_type) {
                continue;
            }

            if let Some(bytes) = self.read_bytes(connection, mime_type) {
                let id = hash(&bytes);
                return Some(ClipboardItem {
                    entries: vec![ClipboardEntry::Image(Image { format, bytes, id })],
                });
            }
        }
        None
    }
}

impl Clipboard {
    pub fn new(connection: Connection, loop_handle: LoopHandle<'static, ()>) -> Self {
        Self {
            connection,
            loop_handle,
            self_mime: format!("pid/{}", std::process::id()),

            contents: None,
            primary_contents: None,
            html: None,
            primary_html: None,
            representations: None,
            primary_representations: None,

            cached_read: None,
            current_offer: None,
            cached_primary_read: None,
            current_primary_offer: None,
        }
    }

    pub fn set(&mut self, item: ClipboardItem) {
        self.contents = Some(item);
        self.html = take_clipboard_html();
        self.representations = None;
    }

    pub fn set_primary(&mut self, item: ClipboardItem) {
        self.primary_contents = Some(item);
        self.primary_html = take_clipboard_html();
        self.primary_representations = None;
    }

    /// Takes `representations` as what the clipboard (`primary`: the primary selection) offers, exactly.
    pub fn set_representations(
        &mut self,
        primary: bool,
        representations: Vec<ClipboardRepresentation>,
    ) -> Result<(), ClipboardMimeError> {
        // Wayland encodes MIME names as C strings. Reject the entire write before replacing either selection or
        // handing any name to wayland-scanner's panicking CString conversion.
        if representations
            .iter()
            .any(|representation| representation.mime.contains('\0'))
        {
            return Err(ClipboardMimeError::Io("clipboard MIME name contains a NUL byte".into()));
        }
        let text = representations
            .iter()
            .find(|representation| representation.mime == "text/plain")
            .map(|representation| {
                ClipboardItem::new_string(String::from_utf8_lossy(&representation.data).into_owned())
            });
        if primary {
            self.primary_contents = text;
            self.primary_html = None;
            self.primary_representations = Some(representations);
        } else {
            self.contents = text;
            self.html = None;
            self.representations = Some(representations);
        }
        Ok(())
    }

    /// What this client's own selection holds, per MIME: a per-MIME write's representations, else the item's text
    /// and HTML.
    fn own_representations(&self, primary: bool) -> Vec<ClipboardRepresentation> {
        let (representations, contents, html) = if primary {
            (&self.primary_representations, &self.primary_contents, &self.primary_html)
        } else {
            (&self.representations, &self.contents, &self.html)
        };
        if let Some(representations) = representations {
            return representations.clone();
        }
        let mut own = Vec::new();
        let with_text = html.as_ref().is_none_or(|html| html.with_text);
        if with_text && let Some(text) = contents.as_ref().and_then(ClipboardItem::text) {
            own.push(ClipboardRepresentation {
                mime: "text/plain".into(),
                data: text.into_bytes(),
            });
        }
        if let Some(html) = html {
            own.push(ClipboardRepresentation {
                mime: HTML_MIME_TYPE.into(),
                data: html.html.clone().into_bytes(),
            });
        }
        own
    }

    /// The MIME types the clipboard (`primary`: the primary selection) advertises.
    pub fn mime_types(&self, primary: bool) -> Option<Vec<String>> {
        let offered = if primary {
            self.current_primary_offer.as_ref().map(|offer| &offer.mime_types)
        } else {
            self.current_offer.as_ref().map(|offer| &offer.mime_types)
        }?;
        if offered.contains(&self.self_mime) {
            return Some(listed_mimes(&offered_mimes(&self.own_representations(primary)), None));
        }
        Some(listed_mimes(offered, Some(&self.self_mime)))
    }

    /// Each of `mimes` the clipboard (`primary`: the primary selection) offers, read in order.
    pub fn read_mimes(&self, primary: bool, mimes: &[String]) -> Vec<ClipboardRepresentation> {
        let mut wanted: Vec<&str> = Vec::new();
        for mime in mimes {
            if !wanted.contains(&mime.as_str()) {
                wanted.push(mime);
            }
        }
        if primary {
            self.current_primary_offer
                .as_ref()
                .map_or_else(Vec::new, |offer| self.read_offer(offer, true, &wanted))
        } else {
            self.current_offer
                .as_ref()
                .map_or_else(Vec::new, |offer| self.read_offer(offer, false, &wanted))
        }
    }

    fn read_offer<T: ReceiveData>(
        &self,
        offer: &DataOffer<T>,
        primary: bool,
        wanted: &[&str],
    ) -> Vec<ClipboardRepresentation> {
        let own = offer
            .has_mime_type(&self.self_mime)
            .then(|| self.own_representations(primary));
        wanted
            .iter()
            .filter_map(|&mime| {
                let source = source_type(&offer.mime_types, mime)?;
                let data = match &own {
                    Some(own) => served(own, source)?.to_vec(),
                    None => offer.read_bytes(&self.connection, source)?,
                };
                Some(ClipboardRepresentation {
                    mime: mime.to_owned(),
                    data: source_data(source, data),
                })
            })
            .collect()
    }

    /// The MIME types the clipboard (`primary`: the primary selection) offers: the text types, `text/html` with an
    /// HTML representation, or `text/html` alone.
    pub fn offered(&self, primary: bool) -> Vec<String> {
        let representations = if primary {
            &self.primary_representations
        } else {
            &self.representations
        };
        if let Some(representations) = representations {
            return offered_mimes(representations);
        }
        let html = if primary { &self.primary_html } else { &self.html };
        let offered = match html {
            Some(html) if html.with_text => TEXT_MIME_TYPES.into_iter().chain([HTML_MIME_TYPE]).collect(),
            Some(_) => vec![HTML_MIME_TYPE],
            None => TEXT_MIME_TYPES.to_vec(),
        };
        offered.into_iter().map(str::to_owned).collect()
    }

    pub fn set_offer(&mut self, data_offer: Option<DataOffer<WlDataOffer>>) {
        self.cached_read = None;
        self.current_offer = data_offer;
    }

    pub fn set_primary_offer(&mut self, data_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>) {
        self.cached_primary_read = None;
        self.current_primary_offer = data_offer;
    }

    pub fn self_mime(&self) -> String {
        self.self_mime.clone()
    }

    pub fn send(&self, mime_type: String, fd: OwnedFd) {
        if let Some(representations) = &self.representations {
            if let Some(data) = served(representations, &mime_type) {
                self.send_bytes(fd, data.to_vec());
            }
            return;
        }
        if mime_type == HTML_MIME_TYPE
            && let Some(html) = &self.html
        {
            self.send_bytes(fd, html.html.as_bytes().to_owned());
            return;
        }
        if let Some(text) = self.contents.as_ref().and_then(|contents| contents.text()) {
            self.send_bytes(fd, text.as_bytes().to_owned());
        }
    }

    pub fn send_primary(&self, mime_type: String, fd: OwnedFd) {
        if let Some(representations) = &self.primary_representations {
            if let Some(data) = served(representations, &mime_type) {
                self.send_bytes(fd, data.to_vec());
            }
            return;
        }
        if mime_type == HTML_MIME_TYPE
            && let Some(html) = &self.primary_html
        {
            self.send_bytes(fd, html.html.as_bytes().to_owned());
            return;
        }
        if let Some(text) = self
            .primary_contents
            .as_ref()
            .and_then(|contents| contents.text())
        {
            self.send_bytes(fd, text.as_bytes().to_owned());
        }
    }

    pub fn read(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_offer.as_ref()?;
        if let Some(cached) = self.cached_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.contents.clone();
        }

        let item = offer
            .read_text(&self.connection)
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_read = Some(item.clone());
        Some(item)
    }

    pub fn read_primary(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_primary_offer.as_ref()?;
        if let Some(cached) = self.cached_primary_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.primary_contents.clone();
        }

        let item = offer
            .read_text(&self.connection)
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_primary_read = Some(item.clone());
        Some(item)
    }

    pub fn send_bytes(&self, fd: OwnedFd, bytes: Vec<u8>) {
        let mut written = 0;
        self.loop_handle
            .insert_source(
                calloop::generic::Generic::new(
                    File::from(fd),
                    calloop::Interest::WRITE,
                    calloop::Mode::Level,
                ),
                move |_, file, _| {
                    let file = unsafe { file.get_mut() };
                    loop {
                        match file.write(&bytes[written..]) {
                            Ok(n) if written + n == bytes.len() => {
                                written += n;
                                break Ok(PostAction::Remove);
                            }
                            Ok(n) => written += n,
                            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                                break Ok(PostAction::Continue);
                            }
                            Err(_) => break Ok(PostAction::Remove),
                        }
                    }
                },
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::os::unix::net::UnixStream;

    use super::*;

    fn rep(mime: &str, data: &[u8]) -> ClipboardRepresentation {
        ClipboardRepresentation {
            mime: mime.to_owned(),
            data: data.to_vec(),
        }
    }

    /// What `clipboard` sends a reader asking for `mime` (`primary`: of the primary selection).
    fn served_to_reader(
        clipboard: &Clipboard,
        event_loop: &mut calloop::EventLoop<'static, ()>,
        primary: bool,
        mime: &str,
    ) -> Vec<u8> {
        let (writer, mut reader) = UnixStream::pair().unwrap();
        writer.set_nonblocking(true).unwrap();
        if primary {
            clipboard.send_primary(mime.to_owned(), OwnedFd::from(writer));
        } else {
            clipboard.send(mime.to_owned(), OwnedFd::from(writer));
        }
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(100)), &mut ())
            .unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn nul_mime_names_reject_the_whole_write_and_preserve_both_selections() {
        let (socket, _server) = UnixStream::pair().unwrap();
        let connection = Connection::from_socket(socket).unwrap();
        let mut event_loop = calloop::EventLoop::<'static, ()>::try_new().unwrap();
        let mut clipboard = Clipboard::new(connection, event_loop.handle());
        for primary in [false, true] {
            clipboard
                .set_representations(primary, vec![rep("text/plain", b"preserved")])
                .unwrap();
        }
        for primary in [false, true] {
            let error = clipboard
                .set_representations(
                    primary,
                    vec![
                        rep("text/plain", b"replacement"),
                        rep("application/x-test\0suffix", b"bad name"),
                    ],
                )
                .unwrap_err();
            assert!(matches!(error, ClipboardMimeError::Io(message) if message.contains("NUL")));
            for selection in [false, true] {
                assert_eq!(
                    clipboard.own_representations(selection),
                    [rep("text/plain", b"preserved")]
                );
                assert_eq!(
                    served_to_reader(&clipboard, &mut event_loop, selection, "text/plain"),
                    b"preserved"
                );
            }
        }
        // NUL bytes in the representation's data are valid.
        clipboard
            .set_representations(false, vec![rep("application/x-test", b"\0")])
            .unwrap();
        assert_eq!(
            served_to_reader(&clipboard, &mut event_loop, false, "application/x-test"),
            b"\0"
        );
    }

    struct TextOffer {
        mime: &'static str,
        bytes: Vec<u8>,
    }

    impl ReceiveData for TextOffer {
        fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
            assert_eq!(mime_type, self.mime);
            File::from(fd.try_clone_to_owned().unwrap())
                .write_all(&self.bytes)
                .unwrap();
        }
    }

    #[test]
    fn owned_and_foreign_text_aliases_read_as_the_same_requested_mime() {
        let (socket, _server) = UnixStream::pair().unwrap();
        let connection = Connection::from_socket(socket).unwrap();
        let event_loop = calloop::EventLoop::<'static, ()>::try_new().unwrap();
        let mut clipboard = Clipboard::new(connection, event_loop.handle());
        let aliases = [
            "text/plain;charset=utf-8",
            "text/plain;charset=UTF-8",
            "UTF8_STRING",
            "TEXT",
            "STRING",
        ];
        for primary in [false, true] {
            for alias in aliases {
                let bytes = if alias == "STRING" {
                    b"caf\xe9".to_vec()
                } else {
                    "café".as_bytes().to_vec()
                };
                clipboard
                    .set_representations(primary, vec![rep(alias, &bytes)])
                    .unwrap();
                let mut offer = DataOffer::new(TextOffer { mime: alias, bytes });
                offer.add_mime_type(alias.to_owned());
                // The frontend normalizes every Ghostty text request to text/plain.
                let wanted = ["text/plain", "image/png"];
                let expected = vec![rep("text/plain", "café".as_bytes())];
                assert_eq!(clipboard.read_offer(&offer, primary, &wanted), expected);
                offer.add_mime_type(clipboard.self_mime());
                assert_eq!(clipboard.read_offer(&offer, primary, &wanted), expected);
            }
            clipboard
                .set_representations(
                    primary,
                    vec![rep("text/plain;charset=utf-8", b"alias"), rep("text/plain", b"exact")],
                )
                .unwrap();
            let mut offer = DataOffer::new(TextOffer {
                mime: "text/plain",
                bytes: Vec::new(),
            });
            for mime in clipboard.offered(primary) {
                offer.add_mime_type(mime);
            }
            offer.add_mime_type(clipboard.self_mime());
            assert_eq!(
                clipboard.read_offer(&offer, primary, &["text/plain"]),
                [rep("text/plain", b"exact")]
            );
        }
    }

    #[test]
    fn per_mime_writes_offer_and_serve_exactly_their_representations() {
        let (socket, _server) = UnixStream::pair().unwrap();
        let connection = Connection::from_socket(socket).unwrap();
        let mut event_loop = calloop::EventLoop::<'static, ()>::try_new().unwrap();
        let mut clipboard = Clipboard::new(connection, event_loop.handle());
        let binary: Vec<u8> = (0..=255).collect();
        clipboard.set_representations(
            false,
            vec![rep("text/plain", "héllo".as_bytes()), rep("image/png", &binary)],
        ).unwrap();
        assert_eq!(
            clipboard.offered(false),
            [
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING",
                "text/plain;charset=UTF-8",
                "image/png"
            ]
        );
        assert_eq!(served_to_reader(&clipboard, &mut event_loop, false, "UTF8_STRING"), "héllo".as_bytes());
        assert_eq!(served_to_reader(&clipboard, &mut event_loop, false, "image/png"), binary);
        assert!(served_to_reader(&clipboard, &mut event_loop, false, "text/html").is_empty());
        // the primary selection keeps its own offer
        assert_eq!(clipboard.offered(true), TEXT_MIME_TYPES);
        clipboard.set_representations(true, vec![rep("text/html", b"<b>")]).unwrap();
        assert_eq!(clipboard.offered(true), ["text/html"]);
        assert_eq!(served_to_reader(&clipboard, &mut event_loop, true, "text/html"), b"<b>");
        assert_eq!(clipboard.own_representations(true), [rep("text/html", b"<b>")]);
        // a GPUI write replaces the representations with its item
        clipboard.set(ClipboardItem::new_string("plain".into()));
        assert_eq!(clipboard.offered(false), TEXT_MIME_TYPES);
        assert_eq!(clipboard.own_representations(false), [rep("text/plain", b"plain")]);
        assert_eq!(served_to_reader(&clipboard, &mut event_loop, false, "text/plain"), b"plain");
    }
}
