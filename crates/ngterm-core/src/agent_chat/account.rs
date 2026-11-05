//! The account an agent runs under and how much of its weekly limit is used, for the chat's `account` event: read on
//! the origin by `ngtermctl agent chat`, which does the file and network IO and hands the bytes to the readers here.
//!
//! - **Claude Code** keeps the account in its global config, `<config>/.claude.json` with `CLAUDE_CONFIG_DIR` set,
//!   else `~/.claude.json` beside the default `~/.claude` ([`claude_config_files`]): `oauthAccount.emailAddress`. Its
//!   weekly limit is on no disk: Claude Code's `/usage` asks `GET` [`CLAUDE_USAGE_URL`] with its sign-in token
//!   (`claudeAiOauth.accessToken` in `<config>/.credentials.json`, or the same JSON in the macOS Keychain), and the
//!   answer's `seven_day` holds `utilization` (0–100) and `resets_at` (RFC 3339); a newer `limits` list repeats it as
//!   the `weekly_all` entry's `percent`. The endpoint is undocumented: an answer without either reads as no limit.
//! - **Codex** keeps the sign-in in `<home>/auth.json`, whose `tokens.id_token` is a JWT naming the `email` (read
//!   unverified: it is only shown). Each turn's `event_msg` `token_count` line in the rollout carries `rate_limits`,
//!   whose `primary` and `secondary` windows name their length (`window_minutes`); the weekly one is
//!   [`WEEK_MINUTES`] long, with `used_percent` and `resets_at` (epoch seconds).
//!
//! What can't be read is left out; nothing here fails.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use super::{ChatEvent, EventBody, non_empty};

/// The `account` event's id: there is one, and a later one replaces it.
pub const ACCOUNT_ID: &str = "account";

/// Claude Code's usage endpoint, as its `/usage` asks it.
pub const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// The beta header value Claude Code's OAuth calls carry.
pub const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";

/// A week in minutes: the length of Codex's weekly window.
pub const WEEK_MINUTES: u64 = 7 * 24 * 60;

/// How much of a limit is used, and when it resets.
#[skip_serializing_none]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Percent used, rounded, 0–100.
    pub used_percent: u32,
    /// When the window starts over, epoch seconds.
    #[serde(default)]
    pub resets_at: Option<u64>,
}

/// What is known of the account: an `account` event once anything is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub email: Option<String>,
    pub weekly: Option<Usage>,
}

impl Account {
    /// Whether nothing is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.weekly.is_none()
    }

    /// The `account` event at `cursor` (where the stream stands). An empty one clears what an earlier one said.
    #[must_use]
    pub fn event(&self, cursor: u64) -> ChatEvent {
        ChatEvent {
            id: ACCOUNT_ID.to_owned(),
            cursor,
            parent: None,
            body: EventBody::Account {
                email: self.email.clone(),
                weekly: self.weekly,
            },
        }
    }
}

/// The global config files Claude Code may keep the account in, the likelier first, for its config directory `config`
/// (from the transcript, [`super::hook::config_dir`]): `<config>/.claude.json`, then, for a directory named `.claude`
/// (the default, `~/.claude`), the one beside it.
#[must_use]
pub fn claude_config_files(config: &Path) -> Vec<PathBuf> {
    let mut files = vec![config.join(".claude.json")];
    if config.file_name().is_some_and(|name| name == ".claude")
        && let Some(home) = config.parent()
    {
        files.push(home.join(".claude.json"));
    }
    files
}

/// The signed-in account's email in Claude Code's global config.
#[must_use]
pub fn claude_email(global_config: &[u8]) -> Option<String> {
    let config: Value = serde_json::from_slice(global_config).ok()?;
    non_empty(&config, "/oauthAccount/emailAddress").map(str::to_owned)
}

/// Claude Code's sign-in token in its credentials (the file's or the Keychain item's JSON), unless it expired by
/// `now_ms` (epoch milliseconds): Claude Code refreshes it while it runs, so an expired one would only be refused.
#[must_use]
pub fn claude_token(credentials: &[u8], now_ms: u64) -> Option<String> {
    let credentials: Value = serde_json::from_slice(credentials).ok()?;
    let expires = credentials.pointer("/claudeAiOauth/expiresAt").and_then(Value::as_u64);
    if expires.is_some_and(|expires| expires <= now_ms) {
        return None;
    }
    non_empty(&credentials, "/claudeAiOauth/accessToken").map(str::to_owned)
}

/// The weekly limit in an answer of [`CLAUDE_USAGE_URL`]: `seven_day`, else the `limits` list's `weekly_all`.
#[must_use]
pub fn claude_weekly(answer: &[u8]) -> Option<Usage> {
    let answer: Value = serde_json::from_slice(answer).ok()?;
    let usage = |used: Option<f64>, resets: Option<&str>| {
        used.map(|used| Usage {
            used_percent: percent(used),
            resets_at: resets.and_then(parse_rfc3339),
        })
    };
    if let Some(week) = answer.get("seven_day").filter(|week| week.is_object())
        && let Some(found) = usage(
            week.get("utilization").and_then(Value::as_f64),
            week.get("resets_at").and_then(Value::as_str),
        )
    {
        return Some(found);
    }
    answer
        .get("limits")
        .and_then(Value::as_array)?
        .iter()
        .find(|limit| limit.get("kind").and_then(Value::as_str) == Some("weekly_all"))
        .and_then(|limit| {
            usage(
                limit.get("percent").and_then(Value::as_f64),
                limit.get("resets_at").and_then(Value::as_str),
            )
        })
}

/// The signed-in account's email in Codex's `auth.json`: the `email` claim of its `tokens.id_token`.
#[must_use]
pub fn codex_email(auth: &[u8]) -> Option<String> {
    let auth: Value = serde_json::from_slice(auth).ok()?;
    let token = non_empty(&auth, "/tokens/id_token")?;
    let payload = token.split('.').nth(1)?;
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&claims).ok()?;
    non_empty(&claims, "/email").map(str::to_owned)
}

/// The weekly limit in the last `token_count` line of a rollout's `tail` (its last bytes, cut anywhere) that names one.
#[must_use]
pub fn codex_weekly(tail: &[u8]) -> Option<Usage> {
    tail.split(|byte| *byte == b'\n')
        .rev()
        .filter(|line| contains(line, b"\"rate_limits\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_slice(line).ok()?;
            let limits = entry.pointer("/payload/rate_limits")?;
            ["primary", "secondary"].iter().find_map(|window| {
                let window = limits.get(window)?;
                (window.get("window_minutes").and_then(Value::as_u64) == Some(WEEK_MINUTES)).then(|| Usage {
                    used_percent: percent(window.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0)),
                    resets_at: window.get("resets_at").and_then(Value::as_u64),
                })
            })
        })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// A percentage as a whole number from 0 to 100.
fn percent(used: f64) -> u32 {
    // clamped to 0..=100 first, so the cast neither wraps nor truncates a sign
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100"
    )]
    let whole = used.clamp(0.0, 100.0).round() as u32;
    whole
}

/// An RFC 3339 time (`2026-11-06T14:00:00.409138+00:00`, `…Z`) as epoch seconds, its fraction dropped; `None` for
/// anything else or a time before 1970.
#[must_use]
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = text.get(range)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let separators = [(4, b'-'), (7, b'-'), (13, b':'), (16, b':')];
    if bytes.len() < 20
        || separators.iter().any(|(at, want)| bytes[*at] != *want)
        || !matches!(bytes[10], b'T' | b't' | b' ')
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = rest.get(1..)?;
            if zone.len() != 5 || zone.as_bytes()[2] != b':' {
                return None;
            }
            let (hours, minutes): (i64, i64) = (zone[..2].parse().ok()?, zone[3..].parse().ok()?);
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
