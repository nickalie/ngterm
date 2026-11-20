//! The shipped text page renderer (upstream's `TextPageRenderingTests`) in a real isolated `WebKit` world, with
//! authored scripts disabled: highlight.js, ngterm's `text-repaint.js` and `text.js` over the document
//! `ngterm_core::html::text_page::html` builds, worn with the stylesheet the app gives it (`page.css` with `text.css`
//! and ngterm's `text-narrow.css` over it).

use std::fmt::Write as _;
use std::time::Duration;

use ngterm_wpe::{
    Event, Geometry, InjectionTime, Load, MessageHandler, PolicyKind, Settings, UserScript, ViewId, ViewSpec,
};

use crate::cases::Context;
use crate::markdown::escaped;
use crate::support::{Harness, Outcome, ensure};

const WORLD: &str = "ngterm-markdown";
/// `text_page::MAX_HIGHLIGHT_BYTES`: a larger text renders plain.
const MAX_HIGHLIGHT_BYTES: usize = 256 << 10;
/// Upstream's hostile text: markup, a refresh, links and a `javascript:` URL, none of which may become anything but
/// text.
const HOSTILE: &str = concat!(
    "<script>document.title = 'ran'</script> <img src=x onerror=\"document.title = 'ran'\"> <link rel=stylesheet href=s.css>\n",
    "<iframe src=\"https://example.com/\"></iframe> <meta http-equiv=\"refresh\" content=\"0;url=https://example.com/\">\n",
    "<a href=\"https://example.com/x\">x</a> javascript:document.title='ran' <object data=x></object> https://example.com/y\n",
);
/// Records the policy's refusals where the page world can read them.
const PROBE: &str = "document.addEventListener('securitypolicyviolation', e => {
  document.documentElement.dataset.blocked = (document.documentElement.dataset.blocked || '') + e.blockedURI + ' ';
});";

/// The renderer the app injects for a text page (`documents::text_script`), with ngterm's `text-repaint.js` before
/// `text.js`.
fn script() -> String {
    [
        include_str!("../../../../resources/markdown/highlight-11.11.1.min.js"),
        include_str!("../../../../resources/html/text-repaint.js"),
        include_str!("../../../../resources/markdown/text.js"),
    ]
    .join(";\n")
}

/// The text page's stylesheet (`documents::text_style`).
fn style() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("../../../../resources/markdown/page.css"),
        include_str!("../../../../resources/markdown/text.css"),
        include_str!("../../../../resources/html/text-narrow.css")
    )
}

/// The document `text_page::html` builds: the text as the escaped content of a hidden `div` (a `<pre>` holder would
/// lose a leading newline to the parser), the language (dropped over [`MAX_HIGHLIGHT_BYTES`]) and the line as
/// attributes, under a policy that loads nothing.
fn document(text: &str, language: Option<&str>, line: Option<i64>) -> Load {
    let mut attributes = String::new();
    if let Some(language) = language.filter(|_| text.len() <= MAX_HIGHLIGHT_BYTES) {
        let _ = write!(attributes, " data-language=\"{}\"", escaped(language));
    }
    if let Some(line) = line {
        let _ = write!(attributes, " data-line=\"{line}\"");
    }
    document_with(text, &attributes)
}

/// The document `text_page::html` builds for `text`, its holder carrying `attributes` as they are.
fn document_with(text: &str, attributes: &str) -> Load {
    Load::Html {
        html: format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
             <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
             <style>{}</style></head><body>\
             <div id=\"ngterm-text\" hidden{attributes}>{}</div>\
             <main id=\"ngterm-page\"></main></body></html>",
            style(),
            escaped(text)
        ),
        base_uri: None,
    }
}

/// A text page's view: the renderer at document end in its world, the page's own JavaScript off, and `extra`.
fn view(h: &Harness, extra: Vec<UserScript>) -> ViewId {
    view_of(h, 320, extra, Vec::new())
}

/// A text page's view `width` pixels wide, with `handlers` registered.
fn view_of(h: &Harness, width: u32, extra: Vec<UserScript>, handlers: Vec<MessageHandler>) -> ViewId {
    let mut scripts = vec![UserScript {
        source: script(),
        time: InjectionTime::End,
        world: Some(WORLD.to_owned()),
    }];
    scripts.extend(extra);
    h.view_with(ViewSpec {
        session: h.ephemeral(),
        settings: Settings::overlay(false),
        scripts,
        message_handlers: handlers,
        zoom: 1.0,
        background: None,
        geometry: Geometry {
            width,
            height: 240,
            scale: 1.0,
        },
        block_network: false,
    })
}

/// Evaluates `joined`, a script that returns a string, in `world` (the page's when `None`).
fn reading(h: &Harness, view: ViewId, joined: &str, world: Option<&str>) -> Result<String, String> {
    let encoded = h.evaluate(view, joined, world)?.ok_or("no DOM report")?;
    serde_json::from_str(&encoded).map_err(|error| error.to_string())
}

/// `count` lines, `line <n>` each, then ` <tail>` unless `tail` is empty.
fn numbered(count: usize, tail: &str) -> String {
    let tail = if tail.is_empty() {
        String::new()
    } else {
        format!(" {tail}")
    };
    (1..=count).fold(String::new(), |mut text, line| {
        let _ = writeln!(text, "line {line}{tail}");
        text
    })
}

fn json(text: &str) -> Result<String, String> {
    serde_json::to_string(text).map_err(|error| error.to_string())
}

/// Hostile text stays text, plain (a `.txt`) or highlighted (a `.js`): the same text, no element it names, no title,
/// nothing the policy had to block, and no navigation asked for.
pub fn hostile(h: &Harness, _: &Context) -> Outcome {
    for (name, language) in [("hostile.txt", None), ("hostile.js", Some("javascript"))] {
        let view = view(
            h,
            vec![UserScript {
                source: PROBE.to_owned(),
                time: InjectionTime::Start,
                world: Some("probe".to_owned()),
            }],
        );
        h.load(view, document(HOSTILE, language, None))?;
        h.idle(Duration::from_millis(300), |event| match event {
            Event::Policy(request) if request.view == view && matches!(request.kind, PolicyKind::Navigation { .. }) => {
                Some(format!("{name}: a navigation was asked for: {:?}", request.kind))
            }
            _ => None,
        })?;
        let dom = reading(
            h,
            view,
            &format!(
                "[document.querySelector('pre code').textContent === {},
                 document.querySelectorAll('script, img, link, iframe, a, object, embed, form, base').length,
                 document.querySelectorAll('meta').length, document.title, location.href,
                 document.documentElement.dataset.blocked ?? ''].join('|')",
                json(HOSTILE)?
            ),
            None,
        )?;
        ensure(dom == "true|0|2||about:blank|", || format!("{name}: {dom:?}"))?;
        h.host.close_view(view);
        h.forget(view);
    }
    Ok(())
}

/// Only highlight.js output reaches markup, and it carries none of the text's tags: spans alone under `pre code`.
pub fn spans(h: &Harness, _: &Context) -> Outcome {
    let view = view(h, Vec::new());
    h.load(view, document(HOSTILE, Some("javascript"), None))?;
    let tags = reading(
        h,
        view,
        "[...new Set([...document.querySelectorAll('pre code *')].map(e => e.tagName))].join(',')",
        None,
    )?;
    ensure(tags == "SPAN", || format!("tags under pre code: {tags:?}"))?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// The renderer runs in its own world: the page world sees no highlight.js and no bridge, and has no base URL.
pub fn isolated(h: &Harness, _: &Context) -> Outcome {
    let view = view(h, Vec::new());
    h.load(view, document("package main\n", Some("go"), None))?;
    let in_page = reading(
        h,
        view,
        "[document.querySelector('pre code')?.textContent === 'package main\\n', typeof hljs, typeof window.ngterm,
          typeof window.webkit?.messageHandlers?.ngterm, document.baseURI].join('|')",
        None,
    )?;
    ensure(in_page == "true|undefined|undefined|undefined|about:blank", || {
        format!("the page world: {in_page:?}")
    })?;
    let in_renderer = reading(
        h,
        view,
        "[typeof hljs, typeof window.webkit?.messageHandlers?.ngterm].join('|')",
        Some(WORLD),
    )?;
    ensure(in_renderer == "object|undefined", || {
        format!("the renderer's world: {in_renderer:?}")
    })?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// Source in a mapped language is highlighted; plain text, and source over [`MAX_HIGHLIGHT_BYTES`], is not.
pub fn highlighting(h: &Harness, _: &Context) -> Outcome {
    let big = "func main() { return }\n".repeat(MAX_HIGHLIGHT_BYTES / 23 + 1);
    let cases = [
        ("main.go", "package main\n\nfunc main() { return }\n", Some("go"), true),
        ("App.swift", "import Foundation\nlet x = 1\n", Some("swift"), true),
        ("notes.txt", "func main() { return }\n", None, false),
        ("big.go", big.as_str(), Some("go"), false),
    ];
    let view = view(h, Vec::new());
    for (name, text, language, highlighted) in cases {
        h.load(view, document(text, language, None))?;
        let rendered = reading(
            h,
            view,
            "[document.querySelector('pre code')?.textContent.length,
              document.querySelectorAll('pre code .hljs-keyword').length > 0].join('|')",
            None,
        )?;
        let expected = format!("{}|{highlighted}", text.encode_utf16().count());
        ensure(rendered == expected, || {
            format!("{name}: {rendered:?}, not {expected:?}")
        })?;
    }
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// The requested line is marked and scrolled to: line 300 of 400 gets the band at 12px of padding and 299 rows of
/// 20px (`text.css`), the gutter numbers every line, and the view shows the band.
pub fn requested_line(h: &Harness, _: &Context) -> Outcome {
    let text = numbered(400, "");
    let view = view(h, Vec::new());
    h.load(view, document(&text, None, Some(300)))?;
    let shown = reading(
        h,
        view,
        "(() => {
          const band = document.querySelector('.ngterm-band');
          const rect = band?.getBoundingClientRect();
          return [band?.style.top, document.querySelector('.ngterm-gutter').textContent.split('\\n').length,
                  window.scrollY > 5000, !!rect && rect.top >= 0 && rect.bottom <= window.innerHeight].join('|');
        })()",
        None,
    )?;
    ensure(shown == "5992px|400|true|true", || {
        format!("line 300 of 400: {shown:?}")
    })?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// A line outside the file, 0 or past its last, marks nothing.
pub fn outside_line(h: &Harness, _: &Context) -> Outcome {
    let view = view(h, Vec::new());
    for line in [0, 4] {
        h.load(view, document("a\nb\nc\n", None, Some(line)))?;
        let shown = reading(
            h,
            view,
            "[document.querySelector('pre code')?.textContent === 'a\\nb\\nc\\n',
              document.querySelectorAll('.ngterm-band').length].join('|')",
            None,
        )?;
        ensure(shown == "true|0", || format!("line {line} of 3: {shown:?}"))?;
    }
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// Regression (upstream): a newline right after `<pre>` is dropped by the parser and shifted every line number, so a
/// leading blank line stays in the text, the gutter and the band's place.
pub fn leading_blank_line(h: &Harness, _: &Context) -> Outcome {
    let view = view(h, Vec::new());
    h.load(view, document("\n\nfunc a() {}\n", Some("swift"), Some(3)))?;
    let rendered = reading(
        h,
        view,
        "[document.querySelector('.ngterm-band')?.style.top, JSON.stringify(document.querySelector('pre code').textContent),
          document.querySelector('.ngterm-gutter').textContent].join('|')",
        None,
    )?;
    ensure(rendered == "52px|\"\\n\\nfunc a() {}\\n\"|1\n2\n3", || {
        format!("a leading blank line: {rendered:?}")
    })?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// Upstream b7884db: the page bar spans the visible width when a wide page scrolls sideways, and the page starts
/// below it. `text.js` builds the bar (the find box) on every text page, so a plain one shows it.
pub fn bar(h: &Harness, _: &Context) -> Outcome {
    let text = numbered(50, &"w".repeat(400));
    let view = view(h, Vec::new());
    h.load(view, document(&text, None, None))?;
    let span = reading(
        h,
        view,
        "(() => {
          document.scrollingElement.scrollLeft = 300;
          const bar = document.querySelector('.ngterm-bar');
          const rect = bar.getBoundingClientRect();
          return [document.scrollingElement.scrollLeft, Math.round(rect.left), Math.round(rect.right),
                  document.documentElement.clientWidth, bar.offsetHeight,
                  Math.round(document.querySelector('#ngterm-page').getBoundingClientRect().top
                    + document.scrollingElement.scrollTop)].join('|');
        })()",
        None,
    )?;
    let parts: Vec<i64> = span.split('|').map(|part| part.parse().unwrap_or(-1)).collect();
    ensure(
        matches!(parts.as_slice(), [300, 0, right, width, height, top] if right == width && top == height && *height > 0),
        || format!("scrollLeft|left|right|clientWidth|bar height|page top: {span:?}"),
    )?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// The handler the app registers in the renderer's world (`text_page::HANDLER`).
const HANDLER: &str = "ngtermTextPage";

/// Upstream 696edcf0 (#978): a page's counts wait for its ready report, so a count the app holds is the page's own. A
/// diff page with changes or none, and a page opened on a search, post `ready` before `changes` or `report`.
pub fn counts_after_ready(h: &Harness, _: &Context) -> Outcome {
    let cases = [
        (
            "a diff",
            "@@ -1 +1 @@\n-a\n+b\n",
            " data-diff data-language=\"diff\"",
            "changes",
        ),
        (
            "a diff with no changes",
            "@@ -1 +1 @@\n a\n",
            " data-diff data-language=\"diff\"",
            "changes",
        ),
        (
            "a seeded search",
            "a b a\n",
            " data-revision=\"1\" data-query=\"a\" data-current=\"0\"",
            "report",
        ),
    ];
    for (name, text, attributes, count) in cases {
        let handler = MessageHandler {
            name: HANDLER.to_owned(),
            world: Some(WORLD.to_owned()),
        };
        let view = view_of(h, 320, Vec::new(), vec![handler]);
        h.host.load(view, document_with(text, attributes));
        let mut posted = Vec::new();
        while posted.last().is_none_or(|kind: &String| kind != count) {
            let message = h.script_message(view)?;
            h.host.reply_script_message(message.token, Ok("null".to_owned()));
            let body: serde_json::Value =
                serde_json::from_str(message.body.as_deref().unwrap_or("null")).map_err(|error| error.to_string())?;
            let kind = body
                .as_object()
                .and_then(|body| body.keys().next().cloned())
                .unwrap_or_default();
            posted.push(kind);
        }
        ensure(posted.first().is_some_and(|kind| kind == "ready"), || {
            format!("{name}: the page posted {posted:?}")
        })?;
        h.host.close_view(view);
        h.forget(view);
    }
    Ok(())
}

/// Upstream 6c082ecd (#934): a log page's bar keeps every control in one line on a narrow (400px, upstream's) page,
/// following, paused and with a note: the Follow button ends before Wrap, Wrap shows its whole label, and the search
/// box and its buttons end inside the page. Under 480px the paused label drops its prefix and keeps `Resume`. A
/// 320px page (a pane of a split window) also keeps them all: there upstream's bar clips the search buttons, and
/// ngterm's own narrow rules (`text-narrow.css`) make the gaps smaller (a fix over upstream).
pub fn narrow_bar(h: &Harness, _: &Context) -> Outcome {
    const FITS: &str = "(() => {
      const width = document.documentElement.clientWidth;
      const follow = document.querySelector('.ngterm-logbar button');
      const box = document.querySelector('.ngterm-search').getBoundingClientRect();
      const controls = [...document.querySelectorAll('.ngterm-search button')].map(e => e.getBoundingClientRect().right);
      const wrap = document.querySelector('.ngterm-wrap');
      return [follow.innerText, document.querySelector('.ngterm-logbar > span').textContent, wrap.textContent,
              follow.getBoundingClientRect().right <= wrap.getBoundingClientRect().left,
              wrap.scrollWidth <= wrap.clientWidth, box.right <= width, Math.max(...controls) <= width].join('|');
    })()";
    let text = numbered(200, "") + "x\n";
    let search = " data-revision=\"1\" data-query=\"line\" data-current=\"0\"";
    let cases = [
        (320, "true", "", "Follow: on||Wrap: off"),
        (320, "false", " data-waiting", "Resume|waiting for file|Wrap: off"),
        (400, "true", "", "Follow: on||Wrap: off"),
        (400, "false", "", "Resume||Wrap: off"),
        (400, "false", " data-waiting", "Resume|waiting for file|Wrap: off"),
        (
            520,
            "false",
            " data-waiting",
            "Follow: paused - Resume|waiting for file|Wrap: off",
        ),
    ];
    for (width, following, note, label) in cases {
        let view = view_of(h, width, Vec::new(), Vec::new());
        let attributes = format!(" data-log data-following=\"{following}\"{note}{search}");
        h.load(view, document_with(&text, &attributes))?;
        let fits = reading(h, view, FITS, None)?;
        let expected = format!("{label}|true|true|true|true");
        ensure(fits == expected, || {
            format!("{width}px, following {following}{note}: {fits:?}, not {expected:?}")
        })?;
        h.host.close_view(view);
        h.forget(view);
    }
    Ok(())
}

/// Upstream's escape text (`agents.log` in `TextPageRenderingTests`): SGR colors, and other sequences dropped
/// between the characters a search finds.
const ESCAPED: &str = concat!(
    "\x1b[36mbugs+impl\x1b[39m started\n\x1b[1;38;5;196mfail\x1b[22m red\x1b[0m \x1b(B\x1b[2Kplain\n",
    "\x1b[1;38:2::0:128:0mgreen\x1b[0m\n",
    "\x1b[32mok\x1b[>4;1m still\x1b[0m \x1b_Gf=1;AA\x1b\\x\x1bPtmux;\x1b\x1b[31mhidden\x1b\\y",
    "\x1bPtmux;\x1b\x1b]8;;http://x\x1b\x1b\\label\x1b\x1b]8;;\x1b\x1b\\\x1b\\\nend\x1bP1+r6b",
);
/// The same shown text with SGR colors only.
const SGR_ONLY: &str = concat!(
    "\x1b[36mbugs+impl\x1b[39m started\n\x1b[1;38;5;196mfail\x1b[22m red\x1b[0m plain\n\x1b[32mgreen\x1b[0m\n",
    "\x1b[33mok still\x1b[0m xy\nend",
);
/// `::highlight(ngterm-current)` with no theme palette (`text.css`).
const CURRENT: (u8, u8, u8) = (0xd9, 0xb4, 0x5a);

/// The pixels the first `word` in the page's text covers, as `[left, top, right, bottom]`, rounded outward.
fn word_rect(h: &Harness, view: ViewId, word: &str) -> Result<[u32; 4], String> {
    let script = format!(
        "(() => {{
          const code = document.querySelector('pre code');
          const begin = code.textContent.indexOf({word}), end = begin + {word}.length;
          const range = document.createRange();
          const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
          let at = 0;
          for (let node = walker.nextNode(); node; node = walker.nextNode()) {{
            if (begin >= at && begin < at + node.data.length) range.setStart(node, begin - at);
            if (end > at && end <= at + node.data.length) range.setEnd(node, end - at);
            at += node.data.length;
          }}
          const rect = range.getBoundingClientRect();
          return JSON.stringify([Math.floor(rect.left), Math.floor(rect.top), Math.ceil(rect.right), Math.ceil(rect.bottom)]);
        }})()",
        word = json(word)?
    );
    serde_json::from_str(&reading(h, view, &script, None)?).map_err(|error| error.to_string())
}

/// The line gap above `rect`, where `WebKit` paints a highlight down from the line before: four rows at its middle.
fn gap(rect: [u32; 4]) -> Vec<(u32, u32)> {
    let x = u32::midpoint(rect[0], rect[2]);
    (rect[1] - 4..rect[1]).map(|y| (x, y)).collect()
}

/// A search that moves on leaves no color of its earlier match: on text that escape sequences split into many text
/// nodes, `WebKit` repaints only the text boxes of a changed highlight, so ngterm's `text-repaint.js` repaints the
/// text block (a fix over upstream). The page searches "impl started", "plain", "xy" and "end" in turn, each step's
/// current match drawn; then the line gaps above "plain" and "xy" show what they showed before the first search.
pub fn search_leaves_no_trace(h: &Harness, _: &Context) -> Outcome {
    for (name, text) in [("escape text", ESCAPED), ("SGR text", SGR_ONLY)] {
        let view = view_of(h, 400, Vec::new(), Vec::new());
        h.load(view, document(text, None, None))?;
        let gaps: Vec<_> = [word_rect(h, view, "plain")?, word_rect(h, view, "xy")?]
            .into_iter()
            .flat_map(gap)
            .collect();
        let (x, y) = gaps[0];
        h.showing(view, x, y, (0xff, 0xff, 0xff))?;
        let before = gaps
            .iter()
            .map(|&(x, y)| h.shown(view, x, y))
            .collect::<Result<Vec<_>, _>>()?;
        for (revision, query) in ["impl started", "plain", "xy", "end"].into_iter().enumerate() {
            h.evaluate(
                view,
                &format!("ngtermTextSearchSet({}, {}, false); null", json(query)?, revision + 1),
                Some(WORLD),
            )?;
            let rect = word_rect(h, view, query)?;
            h.showing(view, rect[0] + 1, rect[1] + 2, CURRENT)
                .map_err(|err| format!("{name}, {query:?} as the current match: {err}"))?;
        }
        h.idle(Duration::from_millis(300), |_| None)?;
        let after = gaps
            .iter()
            .map(|&(x, y)| h.shown(view, x, y))
            .collect::<Result<Vec<_>, _>>()?;
        ensure(after == before, || {
            format!("{name}: the gaps above \"plain\" and \"xy\" at {gaps:?} show {after:02x?}, not {before:02x?}")
        })?;
        h.host.close_view(view);
        h.forget(view);
    }
    Ok(())
}

/// Upstream's `testColorEscapesBecomeStyledTextAndSearchReadsTheShownText` (3479714e, b7f93fe0): a log's SGR colors
/// become styled runs, other sequences (a private CSI, DCS and APC strings, a tmux passthrough) leave no text and no
/// style, and a search finds the shown text across the runs.
pub fn colors(h: &Harness, _: &Context) -> Outcome {
    const SPANS: &str = "[...document.querySelectorAll('pre code span')]
        .map(s => `${s.textContent}:${s.style.color}:${s.style.fontWeight}`).join(',')";
    let view = view(h, Vec::new());
    h.load(view, document(ESCAPED, None, None))?;
    let shown = reading(h, view, "document.querySelector('pre code').textContent", None)?;
    ensure(
        shown == "bugs+impl started\nfail red plain\ngreen\nok still xy\nend",
        || format!("the shown text: {shown:?}"),
    )?;
    let spans = reading(h, view, SPANS, None)?;
    let expected = concat!(
        "bugs+impl:var(--ngterm-color-6, #00cdcd):,fail:rgb(255, 0, 0):bold, red:rgb(255, 0, 0):,",
        "green:rgb(0, 128, 0):bold,ok:var(--ngterm-color-2, #00cd00):, still:var(--ngterm-color-2, #00cd00):",
    );
    ensure(spans == expected, || format!("the styled runs: {spans:?}"))?;
    h.evaluate(view, "ngtermTextSearchSet('impl started', 1, false); null", Some(WORLD))?;
    let count = reading(h, view, "document.querySelector('.ngterm-count').textContent", None)?;
    ensure(count == "1 of 1", || format!("the search count: {count:?}"))?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// Upstream's `testAColoredDiffShowsItsRowsAndFileLanguageWithoutTheEscapes`: git's colors drop out of a patch, which
/// keeps its change steps, its bands and its file's language (`text_page::html` reads the headers without the escapes).
pub fn colored_diff(h: &Harness, _: &Context) -> Outcome {
    let plain = [
        "diff --git a/x.go b/x.go",
        "--- a/x.go",
        "+++ b/x.go",
        "@@ -1 +1 @@",
        "-var a",
        "+func b() {}",
        "",
    ];
    let colored = [
        "\x1b[1mdiff --git a/x.go b/x.go\x1b[m",
        "\x1b[1m--- a/x.go\x1b[m",
        "\x1b[1m+++ b/x.go\x1b[m",
        "\x1b[36m@@ -1 +1 @@\x1b[m",
        "\x1b[31m-var a\x1b[m",
        "\x1b[32m+func b() {}\x1b[m",
        "",
    ]
    .join("\n");
    let view = view(h, Vec::new());
    let html = ngterm_core::html::text_page::html(
        &colored,
        Some("diff"),
        None,
        &style(),
        &ngterm_core::html::text_page::PageState::default(),
    );
    h.load(view, Load::Html { html, base_uri: None })?;
    let dom = reading(
        h,
        view,
        &format!(
            "[document.querySelector('.ngterm-changes span').textContent,
              document.querySelector('pre code').textContent === {},
              document.querySelectorAll('pre code .hljs-keyword').length,
              document.querySelectorAll('.ngterm-added, .ngterm-removed').length].join('|')",
            json(&plain.join("\n"))?
        ),
        None,
    )?;
    ensure(dom == "1 change|true|2|2", || format!("a colored diff: {dom:?}"))?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}

/// Upstream's `testAFollowedLogEndingMidEscapeShowsNoFragmentUntilItCompletes`: a log update that ends inside an
/// escape sequence shows none of it, and the read that completes it paints its color.
pub fn log_mid_escape(h: &Harness, _: &Context) -> Outcome {
    let view = view(h, Vec::new());
    h.load(view, document_with("a\n", " data-log data-following=\"true\""))?;
    let mut shown = Vec::new();
    for update in ["a\nx\x1b[3", "a\nx\x1b[31mb\n"] {
        h.evaluate(
            view,
            &format!(
                "ngtermTextUpdate({}, false, false, false, null, null); null",
                json(update)?
            ),
            Some(WORLD),
        )?;
        shown.push(reading(
            h,
            view,
            "JSON.stringify([document.querySelector('pre code').textContent,
              document.querySelector('pre code span')?.style.color ?? ''])",
            None,
        )?);
    }
    ensure(
        shown == [r#"["a\nx",""]"#, r#"["a\nxb\n","var(--ngterm-color-1, #cd0000)"]"#],
        || format!("the shown text and color after each read: {shown:?}"),
    )?;
    h.host.close_view(view);
    h.forget(view);
    Ok(())
}
