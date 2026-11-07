//! The shipped text page renderer (upstream's `TextPageRenderingTests`) in a real isolated `WebKit` world, with
//! authored scripts disabled: highlight.js and `text.js` over the document `ngterm_core::html::text_page::html`
//! builds, worn with the stylesheet the app gives it (`page.css` with `text.css` over it).

use std::fmt::Write as _;
use std::time::Duration;

use ngterm_wpe::{Event, Geometry, InjectionTime, Load, PolicyKind, Settings, UserScript, ViewId, ViewSpec};

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

/// The renderer the app injects for a text page (`documents::text_script`).
fn script() -> String {
    [
        include_str!("../../../../resources/markdown/highlight-11.11.1.min.js"),
        include_str!("../../../../resources/markdown/text.js"),
    ]
    .join(";\n")
}

/// The text page's stylesheet (`documents::text_style`).
fn style() -> String {
    format!(
        "{}\n{}",
        include_str!("../../../../resources/markdown/page.css"),
        include_str!("../../../../resources/markdown/text.css")
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
        message_handlers: Vec::new(),
        zoom: 1.0,
        background: None,
        geometry: Geometry {
            width: 320,
            height: 240,
            scale: 1.0,
        },
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
