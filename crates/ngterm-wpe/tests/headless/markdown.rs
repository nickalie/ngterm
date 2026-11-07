//! The shipped Markdown renderer in a real isolated `WebKit` world, with authored scripts disabled.

use ngterm_wpe::{Geometry, InjectionTime, Load, Settings, UserScript, ViewSpec};

use crate::cases::Context;
use crate::support::{Harness, Outcome, ensure};

const WORLD: &str = "ngterm-markdown";
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5VQAAAAASUVORK5CYII=";

fn script() -> String {
    [
        include_str!("../../../../resources/markdown/markdown-it-14.1.0.min.js"),
        include_str!("../../../../resources/markdown/highlight-11.11.1.min.js"),
        include_str!("../../../../resources/markdown/render.js"),
    ]
    .join(";\n")
}

const STYLE: &str = include_str!("../../../../resources/markdown/page.css");

pub fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn document(markdown: &str) -> Load {
    let images = serde_json::json!({"uris": [PNG], "references": {"pixel.png": 0}});
    Load::Html {
        html: format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><style>{STYLE}</style><div id=\"ngterm-markdown\" hidden data-images=\"{}\">{}</div><main id=\"ngterm-page\"></main>",
            escaped(&images.to_string()),
            escaped(markdown),
        ),
        base_uri: None,
    }
}

fn view(h: &Harness) -> ngterm_wpe::ViewId {
    h.view_with(ViewSpec {
        session: h.ephemeral(),
        settings: Settings::overlay(false),
        scripts: vec![UserScript {
            source: script(),
            time: InjectionTime::End,
            world: Some(WORLD.to_owned()),
        }],
        message_handlers: Vec::new(),
        zoom: 1.0,
        background: None,
        geometry: Geometry {
            width: 320,
            height: 240,
            scale: 1.0,
        },
        block_network: false,
    })
}

pub fn inert(h: &Harness, _: &Context) -> Outcome {
    let view = view(h);
    h.load(
        view,
        document(concat!(
            "<script>window.authored = true</script>\n\n",
            "<img src=\"https://invalid.example/never-fetch.png\">\n\n",
            "![pixel](pixel.png) ![remote](https://invalid.example/remote.png)\n\n",
            "[web](https://example.com) [anchor](#heading) [bad](file:///etc/passwd)\n\n",
            "- [x] tight\n\n- [ ] loose\n\n  second paragraph\n\n",
            "```javascript\nconst value = 1;\n```\n",
        )),
    )?;
    let checks = h
        .evaluate(
            view,
            r"JSON.stringify({
        authored: typeof window.authored,
        renderer: typeof window.markdownit,
        bridge: typeof window.ngterm,
        scriptCount: document.querySelectorAll('script').length,
        images: [...document.images].map(image => image.getAttribute('src')),
        links: [...document.querySelectorAll('a')].map(link => link.getAttribute('href')),
        checked: [...document.querySelectorAll('input')].map(box => [box.checked, box.disabled]),
        highlighted: !!document.querySelector('code .hljs-keyword'),
        dataGone: !document.getElementById('ngterm-markdown')
    })",
            None,
        )?
        .ok_or("no DOM report")?;
    let encoded: String = serde_json::from_str(&checks).map_err(|error| error.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&encoded).map_err(|error| error.to_string())?;
    ensure(
        value
            == serde_json::json!({
                "authored":"undefined", "renderer":"undefined", "bridge":"undefined", "scriptCount":0,
                "images":[PNG], "links":["https://example.com", "#heading"],
                "checked":[[true,true],[false,true]], "highlighted":true, "dataGone":true,
            }),
        || format!("inert Markdown DOM: {value}"),
    )?;
    h.host.close_view(view);
    Ok(())
}

pub fn limits(h: &Harness, _: &Context) -> Outcome {
    let view = view(h);
    let source = format!(
        "{}\n{}\n# Żółć\n",
        "![pixel](pixel.png) ".repeat(300),
        "# Same\n\n".repeat(2000)
    );
    h.load(view, document(&source))?;
    let checked = h.evaluate(view, r"document.images.length === 256
        && [...document.images].every(image => image.src.startsWith('data:image/png;base64,') && !image.hasAttribute('data-ngterm-image'))
        && [...document.querySelectorAll('h1')].slice(0,2000).every((heading,index) => heading.id === (index ? 'same-' + index : 'same'))
        && document.querySelectorAll('h1')[2000].id === 'żółć'", None)?;
    ensure(checked.as_deref() == Some("true"), || {
        format!("image/heading bounds: {checked:?}")
    })?;
    h.host.close_view(view);
    Ok(())
}

/// Evaluates `joined`, a script that joins its readings with `|`, and returns that string.
fn reading(h: &Harness, view: ngterm_wpe::ViewId, joined: &str) -> Result<String, String> {
    let encoded = h.evaluate(view, joined, None)?.ok_or("no DOM report")?;
    serde_json::from_str(&encoded).map_err(|error| error.to_string())
}

/// Upstream #897: a leading `---` block is a wrapping YAML code block, not a rule over a setext heading.
pub fn frontmatter(h: &Harness, _: &Context) -> Outcome {
    let view = view(h);
    h.load(
        view,
        document("---\nname: chat\ndescription: 'a <b> peer''s chat'\n---\n# Chat\n\nbody\n\n---\n\nafter\n\n```\ncode\n```\n"),
    )?;
    let rendered = reading(
        h,
        view,
        r"[document.querySelector('h1')?.textContent, document.querySelectorAll('h2').length,
        document.querySelectorAll('hr').length,
        document.querySelector('#ngterm-page > :first-child').matches('pre.ngterm-frontmatter'),
        document.querySelector('pre.ngterm-frontmatter code.language-yaml').textContent,
        document.querySelectorAll('pre.ngterm-frontmatter .hljs-attr').length > 0,
        document.querySelectorAll('b').length,
        getComputedStyle(document.querySelector('pre.ngterm-frontmatter')).whiteSpace,
        getComputedStyle(document.querySelector('pre:not(.ngterm-frontmatter)')).whiteSpace].join('|')",
    )?;
    ensure(
        rendered == "Chat|0|1|true|name: chat\ndescription: 'a <b> peer''s chat'|true|0|pre-wrap|pre",
        || format!("frontmatter DOM: {rendered:?}"),
    )?;
    // regression: with a `<pre>` holder the parser dropped the leading newline, so the rule read as frontmatter
    h.load(view, document("\n---\n## Heading\n\nbody\n---\n"))?;
    let rendered = reading(
        h,
        view,
        r"[document.querySelectorAll('pre.ngterm-frontmatter').length, document.querySelectorAll('hr').length,
        [...document.querySelectorAll('h2')].map(h => h.textContent).join(',')].join('|')",
    )?;
    ensure(rendered == "0|1|Heading,body", || {
        format!("a rule after a leading blank line: {rendered:?}")
    })?;
    h.host.close_view(view);
    Ok(())
}
