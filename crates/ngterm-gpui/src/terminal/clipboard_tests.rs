use super::*;

#[gpui_kit::test]
fn stale_answers_do_not_release_waiters_or_remember_a_choice(cx: &mut gpui_kit::TestAppContext) {
    let mut prompts = ClipboardPrompts::default();
    let key = PromptKey {
        access: ClipboardAccess::Read,
        terminal: 7,
    };
    let answers = Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorded = Rc::clone(&answers);
    let Admission::Prompt(id) = prompts.admit(key, Box::new(move |allowed, _| recorded.borrow_mut().push(allowed)))
    else {
        panic!("a fresh request prompts")
    };
    assert!(prompts.answer(key, id + 1, true, true).is_empty());
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Prompt);
    for waiter in prompts.answer(key, id, false, true) {
        cx.update(|cx| waiter(false, cx));
    }
    assert_eq!(*answers.borrow(), [false]);
    assert!(prompts.answer(key, id, true, true).is_empty());
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Deny);
    let recorded = Rc::clone(&answers);
    let Admission::Decided(waiter, allowed) =
        prompts.admit(key, Box::new(move |allowed, _| recorded.borrow_mut().push(allowed)))
    else {
        panic!("the denial is remembered")
    };
    cx.update(|cx| waiter(allowed, cx));
    assert_eq!(*answers.borrow(), [false, false]);
    assert_eq!(prompts.decision(ClipboardAccess::Write), ClipboardDecision::Prompt);
}

use super::super::selection_clipboard::fake::Fake;
use super::super::{TerminalConfig, TerminalSpec};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::AppContext as _;

fn part(mime: &str, data: &[u8]) -> ClipboardContent {
    ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    }
}

#[gpui_kit::test]
fn osc52_writes_keep_text_and_kitty_writes_keep_every_type_or_clear(cx: &mut gpui_kit::TestAppContext) {
    let terminal = cx.new(|cx| {
        Terminal::new(
            TerminalSpec::default(),
            TerminalConfig::default(),
            Box::new(|_| panic!("no view")),
            cx,
        )
    });
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    let write = |scope: ClipboardPermissionScope, contents: Vec<ClipboardContent>| ClipboardWrite {
        id: 0,
        name: String::new(),
        granted: false,
        can_remember: false,
        permission_scope: scope,
        location: ClipboardLocation::Standard,
        contents,
    };
    let png = part("image/png", b"\x89PNG");
    for policy in [ConfigAccess::Deny, ConfigAccess::Allow] {
        let mut config = TerminalConfig::default();
        config.clipboard.write = policy;
        terminal.update(cx, |terminal, cx| terminal.set_config(config, cx));
        cx.update(|cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
            Fake::System.write(vec![part("text/plain", b"before")]);
            on_write(
                &shared,
                &write(
                    ClipboardPermissionScope::Osc52,
                    vec![part("text/plain;charset=utf-8", b"new text")],
                ),
                cx,
            );
            let allowed = policy == ConfigAccess::Allow;
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()).as_deref(),
                Some(if allowed { "new text" } else { "before" })
            );
            // an image-only Kitty write lands as the image alone, with no text left behind
            on_write(&shared, &write(ClipboardPermissionScope::Kitty, vec![png.clone()]), cx);
            let expected = if allowed {
                vec![png.clone()]
            } else {
                vec![part("text/plain", b"before")]
            };
            assert_eq!(Fake::System.contents(), expected);
            // an empty transaction clears
            on_write(&shared, &write(ClipboardPermissionScope::Kitty, Vec::new()), cx);
            assert_eq!(Fake::System.contents().is_empty(), allowed);
        });
    }
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn allowed_reads_answer_the_program_without_prompting(cx: &mut gpui_kit::TestAppContext) {
    use super::super::surface::tests::Fixture;
    let script = "stty raw -echo; printf 'ready'; dd bs=1 count=1 >/dev/null 2>&1; printf '\\033]52;c;?\\007'; dd bs=1 count=12 2>/dev/null | od -An -tx1; printf done; exec cat";
    let mut f = Fixture::new(cx, script, "clipboard-read = allow\n");
    f.cx.write_to_clipboard(ClipboardItem::new_string("hi".into()));
    f.pane().write(b"x").unwrap();
    f.wait("done");
    let text = f.text();
    let bytes: Vec<_> = text.split_whitespace().collect();
    assert!(
        bytes.windows(4).any(|bytes| bytes == ["61", "47", "6b", "3d"]),
        "base64 hi in the reply: {text}"
    );
}

#[gpui_kit::test]
fn one_prompt_answers_queued_requests_and_a_session_allow_applies_to_other_terminals(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut prompts = ClipboardPrompts::default();
    let key = PromptKey {
        access: ClipboardAccess::Write,
        terminal: 1,
    };
    let replies = Rc::new(std::cell::RefCell::new(Vec::new()));
    let waiter = || {
        let replies = Rc::clone(&replies);
        Box::new(move |allowed, _: &mut App| replies.borrow_mut().push(allowed)) as Waiter
    };
    let Admission::Prompt(id) = prompts.admit(key, waiter()) else {
        panic!("the first request prompts")
    };
    assert!(matches!(prompts.admit(key, waiter()), Admission::Queued));
    for waiter in prompts.answer(key, id, true, true) {
        cx.update(|cx| waiter(true, cx));
    }
    assert_eq!(*replies.borrow(), [true, true]);
    let key = PromptKey { terminal: 2, ..key };
    let Admission::Decided(waiter, allowed) = prompts.admit(key, waiter()) else {
        panic!("the session choice applies to every terminal")
    };
    cx.update(|cx| waiter(allowed, cx));
    assert_eq!(*replies.borrow(), [true, true, true]);
    assert_eq!(prompts.decision(ClipboardAccess::Read), ClipboardDecision::Prompt);
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn denied_reads_release_the_pane_without_pasting_clipboard_data(cx: &mut gpui_kit::TestAppContext) {
    use super::super::surface::tests::Fixture;
    let script =
        "stty raw -echo; printf 'ready'; dd bs=1 count=1 >/dev/null 2>&1; printf '\\033]52;c;?\\007done'; exec cat -v";
    let mut f = Fixture::new(cx, script, "clipboard-read = deny\n");
    f.cx.write_to_clipboard(ClipboardItem::new_string("secret".into()));
    f.pane().write(b"x").unwrap();
    f.wait("done");
    // a reply goes to the pane's input before anything written after the read was answered, so `cat` echoes it
    // ahead of the sentinel
    f.pane().write(b"END").unwrap();
    f.wait("END");
    assert!(
        !f.text().contains("c2VjcmV0"),
        "no clipboard data in the pane: {}",
        f.text()
    );
    assert!(
        !f.cx
            .update(|window, cx| crate::modal::alert_open(window.window_handle(), cx))
    );
}

/// A stream pane (no child shell, so every host runs it) in a window, driven through the production GPUI adapter.
struct Stream<'a> {
    cx: &'a mut gpui_kit::VisualTestContext,
    feed: ngterm_term::StreamFeed,
    out: ngterm_term::StreamOut,
    bytes: Vec<u8>,
}

impl<'a> Stream<'a> {
    fn new(cx: &'a mut gpui_kit::TestAppContext, read: ConfigAccess, write: ConfigAccess) -> Self {
        use ngterm_term::{Pane, PaneConfig};
        use std::time::{Duration, Instant};
        cx.executor().allow_parking();
        let (sent, received) = std::sync::mpsc::channel();
        let spawn = Box::new(move |config: PaneConfig| {
            let (pane, feed, out) = Pane::stream(&config)?;
            sent.send((feed, out)).unwrap();
            Ok(pane)
        });
        let mut config = TerminalConfig::default();
        config.clipboard.read = read;
        config.clipboard.write = write;
        let terminal = cx.new(|cx| Terminal::new(TerminalSpec::default(), config, spawn, cx));
        let (_view, cx) =
            cx.add_window_view(|_, cx| super::super::TerminalView::new(&terminal, super::super::ViewRole::Owner, cx));
        let deadline = Instant::now() + Duration::from_secs(5);
        let (feed, out) = loop {
            cx.run_until_parked();
            if let Ok(io) = received.try_recv() {
                break io;
            }
            assert!(Instant::now() < deadline, "stream pane did not start");
            std::thread::yield_now();
        };
        Self {
            cx,
            feed,
            out,
            bytes: Vec::new(),
        }
    }

    /// Feeds `bytes` and a status query after them.
    fn deliver(&mut self, bytes: &[u8]) {
        self.feed.deliver(&[bytes, b"\x1b[5n"].concat());
    }

    /// Pumps until the pane answered the status query (everything before it is answered), or until `stop` holds.
    fn pump(&mut self, stop: impl Fn(&mut Self) -> bool) -> bool {
        use ngterm_term::Outgoing;
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.cx.run_until_parked();
            while let Some(event) = self.out.next(Some(Duration::ZERO)).unwrap() {
                if let Outgoing::Bytes(data) = event {
                    self.bytes.extend(data);
                }
            }
            if self.bytes.ends_with(b"\x1b[0n") {
                return true;
            }
            if stop(self) {
                return false;
            }
            assert!(
                Instant::now() < deadline,
                "no answer: {:?}",
                String::from_utf8_lossy(&self.bytes)
            );
            std::thread::yield_now();
        }
    }

    /// The replies so far, as text, taken.
    fn take(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.bytes)).into_owned()
    }

    /// Pumps until a prompt is up and returns its spec, its preview's page titles and bodies (images decoded here).
    fn prompt(&mut self) -> (crate::modal::AlertSpec, Vec<(String, String)>) {
        let (spec, pages, _) = self.prompt_with_note();
        (spec, pages)
    }

    /// [`Self::prompt`], with what its preview says when it has no page.
    fn prompt_with_note(&mut self) -> (crate::modal::AlertSpec, Vec<(String, String)>, &'static str) {
        assert!(
            !self.pump(|stream| stream.front().is_some()),
            "answered without a prompt"
        );
        assert!(self.bytes.is_empty(), "no reply before consent");
        let alert = self.front().unwrap();
        self.cx.update(|_, cx| {
            let alert = alert.read(cx);
            let preview = alert
                .extra()
                .cloned()
                .and_then(|view| view.downcast::<super::super::clipboard_preview::Preview>().ok())
                .expect("a preview");
            let preview = preview.read(cx);
            let pages = preview
                .pages()
                .iter()
                .map(|page| {
                    let page = page.clone().decoded();
                    (page.title, format!("{:?}", page.body))
                })
                .collect();
            (alert.spec().clone(), pages, preview.empty())
        })
    }

    fn front(&mut self) -> Option<gpui_kit::Entity<crate::modal::Alert>> {
        self.cx
            .update(|window, cx| crate::modal::front_alert(window.window_handle(), cx))
    }

    /// Answers the front prompt with Allow or Deny, its check set to `remember`.
    fn answer(&mut self, allow: bool, remember: bool) {
        let alert = self.front().expect("a prompt");
        self.cx.update(|window, cx| {
            alert.update(cx, |alert, cx| {
                alert.set_checked(remember, cx);
                let button = alert
                    .spec()
                    .buttons
                    .iter()
                    .position(|button| button.label.as_ref() == if allow { ALLOW } else { DENY })
                    .unwrap();
                alert.answer(button, window, cx);
            });
        });
    }
}

const MULTIPART: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/multipart.vt");
const PLAIN: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/plain.txt");
const HTML: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/html.txt");
const RED_PNG: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/red.png");
const ARBITRARY: &[u8] = include_bytes!("../../../ngterm-ghostty-vt/tests/data/clipboard/arbitrary.bin");

fn multipart() -> Vec<ClipboardContent> {
    vec![
        part("text/plain", PLAIN),
        part("text/html", HTML),
        part("image/png", RED_PNG),
        part("application/octet-stream", ARBITRARY),
    ]
}

/// A Kitty write transaction of `contents` under password `pw`.
fn transaction(pw: &str, contents: &[ClipboardContent]) -> Vec<u8> {
    let mut packets = vec![format!(
        "\x1b]5522;type=write:name={}:pw={}\x1b\\",
        STANDARD.encode("Evil Program"),
        STANDARD.encode(pw)
    )];
    for content in contents {
        packets.push(format!(
            "\x1b]5522;type=wdata:mime={};{}\x1b\\",
            STANDARD.encode(&content.mime),
            STANDARD.encode(&content.data)
        ));
    }
    packets.push("\x1b]5522;type=wdata\x1b\\".into());
    packets.concat().into_bytes()
}

/// A Kitty read of `mimes` (space-separated) under password `pw`.
fn query(mimes: &str, pw: &str) -> Vec<u8> {
    format!(
        "\x1b]5522;type=read:name={}:pw={};{}\x1b\\",
        STANDARD.encode("Evil Program"),
        STANDARD.encode(pw),
        STANDARD.encode(mimes)
    )
    .into_bytes()
}

#[gpui_kit::test]
fn kitty_writes_complete_after_allow_deny_prompt_unsupported_or_clear(cx: &mut gpui_kit::TestAppContext) {
    let text = transaction("", &[part("text/plain", b"hi")]);
    for (policy, decision, status, expected) in [
        (ConfigAccess::Allow, None, "DONE", vec![part("text/plain", b"hi")]),
        (ConfigAccess::Deny, None, "EPERM", vec![part("text/plain", b"before")]),
        (ConfigAccess::Ask, Some(true), "DONE", vec![part("text/plain", b"hi")]),
        (
            ConfigAccess::Ask,
            Some(false),
            "EPERM",
            vec![part("text/plain", b"before")],
        ),
    ] {
        Fake::System.write(vec![part("text/plain", b"before")]);
        let mut stream = Stream::new(cx, ConfigAccess::Ask, policy);
        stream.deliver(&text);
        if let Some(allow) = decision {
            let (spec, pages) = stream.prompt();
            assert!(spec.checks.is_empty(), "no password, nothing to remember");
            assert_eq!(pages.len(), 1);
            stream.answer(allow, false);
            assert!(stream.pump(|_| false));
        } else {
            assert!(stream.pump(|_| false));
        }
        assert_eq!(
            stream.take(),
            format!("\x1b]5522;type=write:status={status}\x1b\\\x1b[0n")
        );
        assert_eq!(Fake::System.contents(), expected);
    }
    // an image-only write keeps the image; an empty transaction clears
    Fake::System.write(vec![part("text/plain", b"before")]);
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Allow);
    stream.deliver(&transaction("", &[part("image/png", RED_PNG)]));
    assert!(stream.pump(|_| false));
    assert_eq!(stream.take(), "\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n");
    assert_eq!(Fake::System.contents(), [part("image/png", RED_PNG)]);
    stream.deliver(&transaction("", &[]));
    assert!(stream.pump(|_| false));
    assert_eq!(stream.take(), "\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n");
    assert!(Fake::System.contents().is_empty());
}

#[gpui_kit::test]
fn an_empty_write_asked_about_says_it_clears_the_clipboard(cx: &mut gpui_kit::TestAppContext) {
    Fake::System.write(vec![part("text/plain", b"before")]);
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(&transaction("", &[]));
    let (_, pages, note) = stream.prompt_with_note();
    assert!(pages.is_empty(), "{pages:?}");
    assert_eq!(note, "Nothing: the clipboard will be cleared");
    stream.answer(true, false);
    assert!(stream.pump(|_| false));
    assert!(Fake::System.contents().is_empty());
}

/// The prompt's words never carry the program's self-reported name ("Evil Program").
fn assert_untrusted_name_absent(spec: &crate::modal::AlertSpec) {
    let words = [spec.heading.clone(), spec.body.clone()]
        .into_iter()
        .chain(spec.checks.iter().map(|check| check.label.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !words.contains("Evil"),
        "the program's name is never prompt text: {words}"
    );
}

/// The multipart write's preview: a page per part on Linux and Windows, the text on macOS.
fn assert_multipart_pages(pages: &[(String, String)]) {
    if cfg!(target_os = "macos") {
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].1, format!("Text({:?})", String::from_utf8_lossy(PLAIN)));
        return;
    }
    let titles: Vec<&str> = pages.iter().map(|(title, _)| title.as_str()).collect();
    assert_eq!(
        titles,
        ["text/plain", "text/html", "image/png", "application/octet-stream"]
    );
    assert_eq!(pages[2].1, "Image(2x2)");
    assert_eq!(pages[3].1, "NoPreview(2048)");
}

#[gpui_kit::test]
fn the_multipart_write_previews_every_part_never_trusts_its_name_and_a_remembered_allow_grants(
    cx: &mut gpui_kit::TestAppContext,
) {
    Fake::System.write(Vec::new());
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(MULTIPART);
    let (spec, pages) = stream.prompt();
    assert_untrusted_name_absent(&spec);
    assert_eq!(spec.heading, prompt_text(ClipboardAccess::Write).0);
    assert_eq!(spec.checks.len(), 1, "a password can be remembered");
    assert_eq!(spec.checks[0].label, KITTY_REMEMBER);
    assert_multipart_pages(&pages);
    stream.answer(true, true);
    assert!(stream.pump(|_| false));
    assert_eq!(stream.take(), "\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n");
    assert_eq!(Fake::System.contents(), multipart(), "every part byte for byte");
    // the remembered grant covers the next write under the same password, without a prompt
    stream.deliver(&transaction("1234", &[part("text/plain", b"again")]));
    assert!(stream.pump(|_| false), "granted: no prompt");
    assert_eq!(stream.take(), "\x1b]5522;type=write:status=DONE\x1b\\\x1b[0n");
    assert_eq!(Fake::System.contents(), [part("text/plain", b"again")]);
}

#[gpui_kit::test]
fn a_denial_is_never_remembered(cx: &mut gpui_kit::TestAppContext) {
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(&transaction("other", &[part("text/plain", b"no")]));
    stream.prompt();
    stream.answer(false, true);
    assert!(stream.pump(|_| false));
    assert_eq!(stream.take(), "\x1b]5522;type=write:status=EPERM\x1b\\\x1b[0n");
    stream.deliver(&transaction("other", &[part("text/plain", b"no")]));
    stream.prompt();
    stream.answer(false, false);
    assert!(stream.pump(|_| false));
}

/// Served representations by MIME type, and the listing.
type Served = (Vec<(String, Vec<u8>)>, Option<Vec<String>>);

/// The base64 payloads of the read replies in `replies`, keyed by their MIME type, and the listing.
fn served(replies: &str) -> Served {
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut listing = None;
    for packet in replies.split("\x1b]5522;").skip(1) {
        let packet = packet.split("\x1b\\").next().unwrap_or_default();
        let (meta, payload) = packet.split_once(';').unwrap_or((packet, ""));
        let mime = meta
            .split(':')
            .find_map(|field| field.strip_prefix("mime="))
            .map(|mime| String::from_utf8(STANDARD.decode(mime).unwrap()).unwrap());
        let data = STANDARD.decode(payload).unwrap_or_default();
        match mime.as_deref() {
            Some(".") => {
                listing = Some(
                    String::from_utf8(data)
                        .unwrap()
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect(),
                );
            }
            Some(mime) => match parts.iter_mut().find(|(known, _)| known == mime) {
                Some((_, bytes)) => bytes.extend(data),
                None => parts.push((mime.to_owned(), data)),
            },
            None => {}
        }
    }
    (parts, listing)
}

#[gpui_kit::test]
fn a_kitty_listing_reads_no_data_and_never_prompts(cx: &mut gpui_kit::TestAppContext) {
    Fake::System.write(multipart());
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(b"\x1b]5522;type=read;Lg==\x1b\\");
    assert!(stream.pump(|_| false), "a listing needs no consent");
    let (parts, listing) = served(&stream.take());
    assert!(parts.is_empty());
    assert_eq!(
        listing.unwrap(),
        ["text/plain", "text/html", "image/png", "application/octet-stream"]
    );
    // deny refuses a listing too, the bare read and the `.` request alike (Ghostty checks deny first)
    let mut stream = Stream::new(cx, ConfigAccess::Deny, ConfigAccess::Ask);
    for listing in [&b"\x1b]5522;type=read;Lg==\x1b\\"[..], b"\x1b]5522;type=read\x1b\\"] {
        stream.deliver(listing);
        assert!(stream.pump(|_| false));
        let replies = stream.take();
        assert!(replies.contains("status=EPERM"), "{replies:?}");
        let (parts, listing) = served(&replies);
        assert!(parts.is_empty() && listing.is_none(), "{replies:?}");
    }
}

#[gpui_kit::test]
fn a_read_of_missing_types_with_the_listing_says_only_the_listing_is_disclosed(cx: &mut gpui_kit::TestAppContext) {
    Fake::System.write(vec![part("text/plain", b"secret")]);
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(&query("image/jpeg .", ""));
    let (spec, pages, note) = stream.prompt_with_note();
    assert_eq!(spec.heading, prompt_text(ClipboardAccess::Read).0);
    assert!(pages.is_empty(), "{pages:?}");
    assert_eq!(note, super::super::clipboard_preview::empty_note(ClipboardAccess::Read));
    assert!(!note.contains("cleared"), "{note}");
    stream.answer(true, false);
    assert!(stream.pump(|_| false));
    let (parts, listing) = served(&stream.take());
    assert!(parts.is_empty());
    assert_eq!(listing.unwrap(), ["text/plain"]);
}

/// What an allowed read serves after the clipboard changed under its prompt: Linux and Windows read again, macOS
/// serves what it showed.
fn served_after_a_change() -> Vec<(String, Vec<u8>)> {
    if cfg!(target_os = "macos") {
        vec![
            ("image/png".into(), RED_PNG.to_vec()),
            ("text/plain".into(), PLAIN.to_vec()),
        ]
    } else {
        vec![("text/plain".to_owned(), b"changed".to_vec())]
    }
}

#[gpui_kit::test]
fn kitty_reads_preview_what_they_disclose_and_serve_it_once_allowed(cx: &mut gpui_kit::TestAppContext) {
    Fake::System.write(multipart());
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    // a read shows exactly the requested representations, in the program's order, the missing one skipped
    stream.deliver(&query("image/png image/jpeg text/plain", "1234"));
    let (spec, pages) = stream.prompt();
    assert_eq!(spec.heading, prompt_text(ClipboardAccess::Read).0);
    let titles: Vec<&str> = pages.iter().map(|(title, _)| title.as_str()).collect();
    let expected: &[&str] = if cfg!(target_os = "macos") {
        &[""]
    } else {
        &["image/png", "text/plain"]
    };
    assert_eq!(titles, expected);
    Fake::System.write(vec![part("text/plain", b"changed")]);
    stream.answer(true, true);
    assert!(stream.pump(|_| false));
    assert_eq!(served(&stream.take()).0, served_after_a_change());
    // the remembered allow grants the next read under that password; another is asked about and can be denied
    Fake::System.write(multipart());
    stream.deliver(&query("application/octet-stream", "1234"));
    assert!(stream.pump(|_| false), "granted: no prompt");
    let (parts, _) = served(&stream.take());
    assert_eq!(parts, [("application/octet-stream".to_owned(), ARBITRARY.to_vec())]);
    stream.deliver(&query("text/plain", "other"));
    stream.prompt();
    stream.answer(false, false);
    assert!(stream.pump(|_| false));
    let replies = stream.take();
    assert!(replies.contains("status=EPERM"), "{replies:?}");
    assert!(served(&replies).0.is_empty());
}

#[gpui_kit::test]
fn kitty_reads_with_nothing_to_disclose_or_under_a_policy_never_prompt(cx: &mut gpui_kit::TestAppContext) {
    Fake::System.write(vec![part("text/plain", b"secret")]);
    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(&query("image/png", ""));
    assert!(stream.pump(|_| false), "nothing to serve: no prompt");
    assert!(served(&stream.take()).0.is_empty());
    let mut stream = Stream::new(cx, ConfigAccess::Deny, ConfigAccess::Ask);
    stream.deliver(&query("text/plain", ""));
    assert!(stream.pump(|_| false));
    let replies = stream.take();
    assert!(
        replies.contains("status=EPERM") && served(&replies).0.is_empty(),
        "{replies:?}"
    );
    let mut stream = Stream::new(cx, ConfigAccess::Allow, ConfigAccess::Ask);
    stream.deliver(&query("text/plain", ""));
    assert!(stream.pump(|_| false));
    assert_eq!(
        served(&stream.take()).0,
        [("text/plain".to_owned(), b"secret".to_vec())]
    );
}

#[gpui_kit::test]
fn kitty_text_aliases_disclose_text_under_the_requested_name_and_require_consent(cx: &mut gpui_kit::TestAppContext) {
    let aliases = ["UTF8_STRING", "text/plain;charset=utf-8", "TEXT", "STRING"];
    let expected: Vec<_> = aliases
        .iter()
        .map(|mime| ((*mime).to_owned(), "café 🐈".as_bytes().to_vec()))
        .collect();
    let query = query(&aliases.join(" "), "");
    Fake::System.write(vec![part("text/plain", "café 🐈".as_bytes())]);
    let mut stream = Stream::new(cx, ConfigAccess::Allow, ConfigAccess::Ask);
    stream.deliver(&query);
    assert!(stream.pump(|_| false));
    assert_eq!(served(&stream.take()).0, expected);

    let mut stream = Stream::new(cx, ConfigAccess::Ask, ConfigAccess::Ask);
    stream.deliver(&query);
    stream.prompt();
    stream.answer(true, false);
    assert!(stream.pump(|_| false));
    assert_eq!(served(&stream.take()).0, expected);

    stream.deliver(&query);
    stream.prompt();
    stream.answer(false, false);
    assert!(stream.pump(|_| false));
    let reply = stream.take();
    assert!(reply.contains("status=EPERM"), "{reply:?}");
    assert!(served(&reply).0.is_empty());
}
