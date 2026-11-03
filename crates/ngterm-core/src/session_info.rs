//! Runtime sidebar info and its inline spans, ported from upstream `SessionInfoMarkup.swift`.
//! Foundation's inline-only markdown maps to pulldown-cmark inline events; custom color/dim attributes are
//! represented as private inline tags during parsing. Links contribute their labels, never clickable targets.

use std::fmt::Write as _;
use std::time::Duration;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use uuid::Uuid;

use crate::Core;
use crate::hud::HudSpec;
use crate::model::TintColor;

/// One styled run (upstream's `SessionInfoSpan`), independent of the UI toolkit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "upstream's independent font/color traits combine freely, not a state machine"
)]
pub struct SessionInfoSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub dim: bool,
    pub color: Option<TintColor>,
}

/// A caller's source, configured lifetime and parsed visible runs. Never persisted or duplicated.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub source: String,
    pub markdown: bool,
    pub hide_after: f64,
    pub spans: Vec<SessionInfoSpan>,
}

impl SessionInfo {
    pub const MAX_SOURCE_BYTES: usize = 512;

    /// Validates in upstream's order: source byte cap, lifetime, then decoded visible markup.
    ///
    /// # Errors
    /// Returns upstream's rejection text for an oversized source, invalid lifetime or invalid visible markup.
    pub fn make(source: &str, markdown: bool, hide_after: f64) -> Result<Self, String> {
        if source.len() > Self::MAX_SOURCE_BYTES {
            return Err("info text is over 512 bytes".to_owned());
        }
        if !HudSpec::is_valid_hide_after(hide_after) {
            return Err("--hide-after must be 0...86400 seconds".to_owned());
        }
        let spans = if markdown {
            parse(source)?
        } else {
            vec![SessionInfoSpan {
                text: source.to_owned(),
                ..SessionInfoSpan::default()
            }]
        };
        validate(&spans.iter().map(|span| span.text.as_str()).collect::<String>())?;
        Ok(Self {
            source: source.to_owned(),
            markdown,
            hide_after,
            spans,
        })
    }
}

const INVALID_MARKDOWN: &str = "invalid info markdown; quote attribute values: ^[text](color: \"green\")";

fn validate(text: &str) -> Result<(), String> {
    for character in text.chars() {
        if matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{85}') {
            return Err("info text must be one line".to_owned());
        }
        if character.is_control() {
            return Err("info text contains a control character".to_owned());
        }
    }
    if text.trim_matches(char::is_whitespace).is_empty() {
        return Err("info text is empty".to_owned());
    }
    Ok(())
}

/// One queued hide; its revision becomes inert when the value is replaced or discarded.
#[derive(Debug, Clone, Copy)]
pub struct InfoHideTimer {
    pub session: Uuid,
    pub delay: Duration,
    pub(crate) revision: Uuid,
}

impl Core {
    /// Takes newly armed local expiries for the frontend to schedule on its main loop.
    pub fn take_info_timers(&mut self) -> Vec<InfoHideTimer> {
        std::mem::take(&mut self.info_timers)
    }

    /// Fires only the currently armed local value; callers commit after a successful clear.
    pub fn fire_info_hide(&mut self, timer: InfoHideTimer) -> bool {
        let Some(store) = self.library_mut().store_for_session_mut(timer.session) else {
            return false;
        };
        if store
            .session(timer.session)
            .is_none_or(|session| session.info_revision != Some(timer.revision))
        {
            return false;
        }
        store.set_info(None, timer.session)
    }

    pub(crate) fn set_session_info(&mut self, info: Option<SessionInfo>, id: Uuid) {
        let revision = Uuid::new_v4();
        let seconds = info.as_ref().map_or(0.0, |info| info.hide_after);
        if let Some(store) = self.library_mut().store_for_session_mut(id) {
            store.set_info(info, id);
            if let Some(session) = store.session_mut(id) {
                session.info_revision = (seconds > 0.0).then_some(revision);
            }
        }
        if seconds > 0.0 {
            self.info_timers.push(InfoHideTimer {
                session: id,
                delay: Duration::from_secs_f64(seconds),
                revision,
            });
        }
    }
}

fn push(spans: &mut Vec<SessionInfoSpan>, text: &str, style: &SessionInfoSpan) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.bold == style.bold
        && last.italic == style.italic
        && last.code == style.code
        && last.dim == style.dim
        && last.color == style.color
    {
        last.text.push_str(text);
    } else {
        spans.push(SessionInfoSpan {
            text: text.to_owned(),
            ..style.clone()
        });
    }
}

fn parse(source: &str) -> Result<Vec<SessionInfoSpan>, String> {
    let marker = format!("ngterm-info-{}-", Uuid::new_v4());
    let mut attributes = Vec::new();
    let source = attribute_source(source, &mut attributes, &marker)?;
    // Keep raw line separators in inline events instead of letting the block parser consume blank paragraphs.
    let line_break = format!("{marker}line-break");
    let carriage_return = format!("{marker}carriage-return");
    let source = source.replace('\n', &line_break).replace('\r', &carriage_return);
    // A non-markup prefix keeps the one line in a paragraph, preserving literal heading/list prefixes.
    let source = format!("x {source} x");
    let mut spans = Vec::new();
    let mut styles = vec![SessionInfoSpan::default()];
    let mut prefix = true;
    for (event, range) in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        let style = styles.last().cloned().unwrap_or_default();
        match event {
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough | Tag::Link { .. }) => {
                let mut next = style;
                match event {
                    Event::Start(Tag::Strong) => next.bold = true,
                    Event::Start(Tag::Emphasis) => next.italic = true,
                    _ => {}
                }
                styles.push(next);
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link) => {
                styles.pop();
            }
            Event::InlineHtml(html) if attribute_event(&html, &mut styles, &attributes, &marker) => {}
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                numeric_controls(&source[range])?;
                let text = if prefix {
                    prefix = false;
                    text.strip_prefix("x ").unwrap_or(&text)
                } else {
                    &text
                };
                let text = text.replace(&line_break, "\n").replace(&carriage_return, "\r");
                push(&mut spans, &text, &style);
            }
            Event::Code(text) => {
                let text = text.replace(&line_break, "\n").replace(&carriage_return, "\r");
                push(&mut spans, &text, &SessionInfoSpan { code: true, ..style });
            }
            Event::SoftBreak | Event::HardBreak => push(&mut spans, "\n", &style),
            _ => {}
        }
    }
    if let Some(last) = spans.last_mut() {
        last.text.truncate(last.text.len().saturating_sub(2));
    }
    spans.retain(|span| !span.text.is_empty());
    Ok(spans)
}

/// `CommonMark` remaps some numeric controls to Windows glyphs; Foundation validates the decoded scalar instead.
fn numeric_controls(source: &str) -> Result<(), String> {
    for part in source.split("&#").skip(1) {
        let Some((number, _)) = part.split_once(';') else {
            continue;
        };
        let value = if let Some(hex) = number.strip_prefix('x').or_else(|| number.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            number.parse::<u32>().ok()
        };
        if let Some(character) = value.and_then(char::from_u32)
            && (character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
        {
            validate(&character.to_string())?;
        }
    }
    Ok(())
}

/// Converts Foundation attribute runs to private inline tags, skipping escaped syntax and inline code.
fn attribute_source(source: &str, attributes: &mut Vec<InfoAttributes>, marker: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = source;
    let mut code_ticks = 0;
    while !rest.is_empty() {
        if code_ticks == 0 && rest.starts_with('\\') {
            let size = rest.chars().take(2).map(char::len_utf8).sum::<usize>();
            out.push_str(&rest[..size]);
            rest = &rest[size..];
            continue;
        }
        if rest.starts_with('`') {
            let ticks = rest.bytes().take_while(|byte| *byte == b'`').count();
            if code_ticks == 0 && has_closing_ticks(&rest[ticks..], ticks) {
                code_ticks = ticks;
            } else if code_ticks == ticks {
                code_ticks = 0;
            }
            out.push_str(&rest[..ticks]);
            rest = &rest[ticks..];
            continue;
        }
        if code_ticks == 0
            && rest.starts_with("^[")
            && let Some((label_end, end)) = attribute_bounds(rest)
        {
            let style = parse_attributes(&rest[label_end + 2..end])?;
            let label = attribute_source(&rest[2..label_end], attributes, marker)?;
            let index = attributes.len();
            attributes.push(style);
            let _ = write!(out, "<{marker}{index}>{label}</{marker}{index}>");
            rest = &rest[end + 1..];
            continue;
        }
        let character = rest.chars().next().unwrap_or_default();
        out.push(character);
        rest = &rest[character.len_utf8()..];
    }
    Ok(out)
}

fn has_closing_ticks(mut source: &str, count: usize) -> bool {
    while let Some(index) = source.find('`') {
        source = &source[index..];
        let ticks = source.bytes().take_while(|byte| *byte == b'`').count();
        if ticks == count {
            return true;
        }
        source = &source[ticks..];
    }
    false
}

fn attribute_bounds(source: &str) -> Option<(usize, usize)> {
    let mut depth = 1;
    let mut escaped = false;
    for (index, character) in source[2..].char_indices() {
        let index = index + 2;
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '[' {
            depth += 1;
        }
        if character == ']' {
            depth -= 1;
        }
        if depth == 0 {
            if !source[index..].starts_with("](") {
                return None;
            }
            let mut quote = None;
            let mut escaped = false;
            for (offset, character) in source[index + 2..].char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if character == '\\' {
                    escaped = true;
                    continue;
                }
                if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() && matches!(character, '\'' | '"') {
                    quote = Some(character);
                } else if quote.is_none() && character == ')' {
                    return Some((index, index + 2 + offset));
                }
            }
            return None;
        }
    }
    None
}

#[derive(Debug, Default)]
struct InfoAttributes {
    color: Option<TintColor>,
    dim: Option<bool>,
}

fn attribute_event(html: &str, styles: &mut Vec<SessionInfoSpan>, attributes: &[InfoAttributes], marker: &str) -> bool {
    if html
        .strip_prefix("</")
        .and_then(|raw| raw.strip_suffix('>'))
        .is_some_and(|raw| raw.starts_with(marker))
    {
        styles.pop();
        return true;
    }
    let Some(attributes) = html
        .strip_prefix('<')
        .and_then(|raw| raw.strip_suffix('>'))
        .and_then(|raw| raw.strip_prefix(marker))
        .and_then(|raw| raw.parse::<usize>().ok())
        .and_then(|index| attributes.get(index))
    else {
        return false;
    };
    let mut style = styles.last().cloned().unwrap_or_default();
    style.color = attributes.color.or(style.color);
    style.dim = attributes.dim.unwrap_or(style.dim);
    styles.push(style);
    true
}

fn parse_attributes(source: &str) -> Result<InfoAttributes, String> {
    let mut style = InfoAttributes::default();
    let mut rest = source.trim();
    while !rest.is_empty() {
        let (key, value) = rest.split_once(':').ok_or(INVALID_MARKDOWN)?;
        rest = value.trim_start();
        let (value, remaining) = attribute_value(rest)?;
        match key.trim() {
            "color" => {
                let raw = value.ok_or(INVALID_MARKDOWN)?;
                style.color = Some(
                    TintColor::from_wire(&raw.to_lowercase())
                        .ok_or_else(|| format!("unknown info color: {raw} ({})", TintColor::valid_names_list()))?,
                );
            }
            "dim" => {
                if value.is_some() {
                    return Err(INVALID_MARKDOWN.to_owned());
                }
                let token = rest[..rest.len() - remaining.len()].trim();
                style.dim = Some(token == "true");
                if token != "true" && token != "false" {
                    return Err(INVALID_MARKDOWN.to_owned());
                }
            }
            _ => {}
        }
        rest = remaining.trim_start();
        if rest.is_empty() {
            break;
        }
        rest = rest.strip_prefix(',').ok_or(INVALID_MARKDOWN)?.trim_start();
    }
    Ok(style)
}

fn attribute_value(source: &str) -> Result<(Option<String>, &str), String> {
    let first = source.chars().next().ok_or(INVALID_MARKDOWN)?;
    if matches!(first, '\'' | '"') {
        let mut value = String::new();
        let mut escaped = false;
        for (offset, character) in source[first.len_utf8()..].char_indices() {
            if escaped {
                value.push(character);
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == first {
                return Ok((Some(value), &source[offset + 2 * first.len_utf8()..]));
            } else {
                value.push(character);
            }
        }
        return Err(INVALID_MARKDOWN.to_owned());
    }
    let length = source.find(',').unwrap_or(source.len());
    Ok((None, &source[length..]))
}

#[cfg(test)]
mod tests;
