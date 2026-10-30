//! What a Windows toast is made of, host-free so every OS tests it: the slot a banner's identity takes in the
//! notification center (a toast's `Group` and `Tag`, which replace and withdraw it) and the toast's XML.
//!
//! A toast's tag and group hold at most 64 characters each, and a terminal's identity (`<windowID>:<sessionID>:<pane>`)
//! is longer: its window id is the group and the rest the tag. Any other identity (an app banner's: a diagnostics count,
//! a hook's or a command's failure) goes to the app's own group, as is when it fits and shortened with a hash of the
//! whole identity when it does not, so two identities never share a slot by accident.

/// The longest tag or group a toast takes (`ToastNotification.Tag`, Windows 10 Creators Update and later).
pub const MAX_KEY: usize = 64;

/// The group of the app's own banners (those of no terminal).
pub const APP_GROUP: &str = "ngterm";

/// Where a banner sits in the notification center: a repeat in the same slot replaces it, a withdrawal removes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub group: String,
    pub tag: String,
}

/// The slot of the banner posted under `identity` (see the module docs).
#[must_use]
pub fn slot(identity: &str) -> Slot {
    if let Some((window, rest)) = identity.split_once(':')
        && uuid::Uuid::parse_str(window).is_ok()
        && fits(rest)
        && !rest.is_empty()
    {
        return Slot {
            group: window.to_owned(),
            tag: rest.to_owned(),
        };
    }
    Slot {
        group: APP_GROUP.to_owned(),
        tag: shortened(identity),
    }
}

/// Whether `key` fits a toast's tag or group (counted in UTF-16 units, as WinRT's strings are).
fn fits(key: &str) -> bool {
    key.encode_utf16().count() <= MAX_KEY
}

/// `identity` as a tag: itself when it fits, else as much of it as leaves room for `~` and the 16 hex digits of its
/// FNV-1a 64 hash.
fn shortened(identity: &str) -> String {
    if fits(identity) {
        return identity.to_owned();
    }
    let room = MAX_KEY - 17;
    let mut used = 0;
    let prefix: String = identity
        .chars()
        .take_while(|ch| {
            used += ch.len_utf16();
            used <= room
        })
        .collect();
    format!("{prefix}~{:016x}", fnv1a64(identity.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The toast's XML: the generic template with the title and the body as its two lines, silent (the configured
/// notification sound plays beside it, as on Linux: `crate::notifications::banner_sound`).
#[must_use]
pub fn xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>\
         <audio silent=\"true\"/></toast>",
        escape(title),
        escape(body)
    )
}

/// `text` as XML character data: the markup characters escaped, and the control characters XML 1.0 does not allow
/// (all but tab, line feed and carriage return) dropped, so no banner's text makes the XML unreadable.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(ch),
            '\u{0}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}' => {}
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use ngterm_core::{PaneRole, control_notification_identity, notification_identity};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn a_terminal_banner_takes_its_window_as_the_group_and_the_rest_as_the_tag() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        for (identity, rest) in [
            (
                notification_identity(window, session, PaneRole::Overlay),
                format!("{session}:overlay"),
            ),
            (
                control_notification_identity(window, session),
                format!("{session}:notify"),
            ),
        ] {
            assert_eq!(
                slot(&identity),
                Slot {
                    group: window.to_string(),
                    tag: rest
                }
            );
        }
        let split = slot(&notification_identity(window, session, PaneRole::Split));
        let main = slot(&notification_identity(window, session, PaneRole::Main));
        assert_ne!(split, main, "each pane has a slot of its own");
        assert_eq!(split.group, main.group);
    }

    #[test]
    fn an_app_banner_takes_the_app_group_and_a_long_one_a_hashed_tag() {
        assert_eq!(
            slot("keymap-diagnostics"),
            Slot {
                group: APP_GROUP.to_owned(),
                tag: "keymap-diagnostics".to_owned()
            }
        );
        let long = format!("command-failure:{}", "x".repeat(80));
        let other = format!("command-failure:{}y", "x".repeat(79));
        let (first, second) = (slot(&long), slot(&other));
        assert_eq!(first.tag.encode_utf16().count(), MAX_KEY);
        assert!(first.tag.starts_with("command-failure:xxx"));
        assert_ne!(first.tag, second.tag, "the hash keeps two long identities apart");
        assert_eq!(slot(&long), first, "the same identity, the same slot");
        let wide = "ü".repeat(70);
        assert!(slot(&wide).tag.encode_utf16().count() <= MAX_KEY);
        assert_eq!(
            slot("not-a-uuid:rest").group,
            APP_GROUP,
            "only a window id names a group"
        );
    }

    #[test]
    fn the_xml_escapes_the_text_and_is_silent() {
        assert_eq!(
            xml("a <b> & \"c\"", "it's\u{7}\n2"),
            "<toast><visual><binding template=\"ToastGeneric\"><text>a &lt;b&gt; &amp; &quot;c&quot;</text>\
             <text>it&apos;s\n2</text></binding></visual><audio silent=\"true\"/></toast>"
        );
    }
}
