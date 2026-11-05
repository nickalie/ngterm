use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::agent_chat::account::{
    Account, Usage, claude_config_files, claude_email, claude_token, claude_weekly, codex_email, codex_weekly,
    parse_rfc3339,
};
use crate::agent_chat::{ChatEvent, EventBody};

#[test]
fn rfc3339_times_read_as_epoch_seconds_with_their_offset_and_no_fraction() {
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339("2026-11-06T14:00:00.409138+00:00"), Some(1_793_973_600));
    assert_eq!(parse_rfc3339("2026-11-06T16:00:00+02:00"), Some(1_793_973_600));
    assert_eq!(parse_rfc3339("2026-11-06T09:30:00-04:30"), Some(1_793_973_600));
    // a leap year's last day
    assert_eq!(parse_rfc3339("2024-12-31T23:59:59Z"), Some(1_735_689_599));
    for bad in [
        "",
        "2026-11-06",
        "2026-11-06T14:00:00",
        "2026-13-06T14:00:00Z",
        "2026-11-06T14:00:00.Z",
        "x026-11-06T14:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn claudes_global_config_is_in_its_config_dir_or_beside_the_default_one() {
    assert_eq!(
        claude_config_files(Path::new("/home/u/.claude")),
        [
            PathBuf::from("/home/u/.claude/.claude.json"),
            PathBuf::from("/home/u/.claude.json")
        ]
    );
    assert_eq!(
        claude_config_files(Path::new("/srv/cc")),
        [PathBuf::from("/srv/cc/.claude.json")]
    );
}

#[test]
fn claudes_email_and_token_come_from_its_config_and_credentials() {
    let config = br#"{"oauthAccount":{"emailAddress":"dev@example.com","organizationName":"x"}}"#;
    assert_eq!(claude_email(config).as_deref(), Some("dev@example.com"));
    assert_eq!(claude_email(br#"{"oauthAccount":{"emailAddress":""}}"#), None);
    assert_eq!(claude_email(b"{}"), None);
    assert_eq!(claude_email(b"not json"), None);

    let credentials = br#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":2000}}"#;
    assert_eq!(claude_token(credentials, 1999).as_deref(), Some("tok"));
    // expired: Claude Code refreshes it while it runs
    assert_eq!(claude_token(credentials, 2000), None);
    assert_eq!(
        claude_token(br#"{"claudeAiOauth":{"accessToken":"tok"}}"#, 5).as_deref(),
        Some("tok")
    );
    assert_eq!(claude_token(b"{}", 5), None);
}

#[test]
fn claudes_weekly_limit_is_seven_day_or_the_weekly_all_limit() {
    let answer =
        br#"{"five_hour":null,"seven_day":{"utilization":96.6,"resets_at":"2026-11-06T14:00:00.409138+00:00"}}"#;
    assert_eq!(
        claude_weekly(answer),
        Some(Usage {
            used_percent: 97,
            resets_at: Some(1_793_973_600)
        })
    );
    let newer = br#"{"seven_day":null,"limits":[
        {"kind":"weekly_scoped","percent":3,"resets_at":"2026-11-06T14:00:00+00:00"},
        {"kind":"weekly_all","percent":40,"resets_at":"2026-11-06T14:00:00+00:00"}]}"#;
    assert_eq!(
        claude_weekly(newer),
        Some(Usage {
            used_percent: 40,
            resets_at: Some(1_793_973_600)
        })
    );
    assert_eq!(
        claude_weekly(br#"{"seven_day":{"utilization":120.0}}"#),
        Some(Usage {
            used_percent: 100,
            resets_at: None
        })
    );
    assert_eq!(claude_weekly(br#"{"seven_day":null}"#), None);
    assert_eq!(claude_weekly(br#"{"error":{"type":"authentication_error"}}"#), None);
}

fn jwt(claims: &str) -> String {
    let part = |text: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text);
    format!("{}.{}.sig", part(r#"{"alg":"RS256"}"#), part(claims))
}

#[test]
fn codexs_email_is_its_id_tokens_claim() {
    let auth = format!(
        r#"{{"tokens":{{"id_token":"{}"}}}}"#,
        jwt(r#"{"email":"dev@example.com","sub":"1"}"#)
    );
    assert_eq!(codex_email(auth.as_bytes()).as_deref(), Some("dev@example.com"));
    let none = format!(r#"{{"tokens":{{"id_token":"{}"}}}}"#, jwt(r#"{"sub":"1"}"#));
    assert_eq!(codex_email(none.as_bytes()), None);
    assert_eq!(codex_email(br#"{"OPENAI_API_KEY":"sk"}"#), None);
    assert_eq!(codex_email(br#"{"tokens":{"id_token":"garbage"}}"#), None);
}

fn token_count(primary: &str, secondary: &str) -> String {
    format!(
        r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{primary},"secondary":{secondary}}}}}}}"#
    )
}

#[test]
fn codexs_weekly_limit_is_the_last_token_counts_week_long_window() {
    let older = token_count(
        r#"{"used_percent":10.0,"window_minutes":10080,"resets_at":100}"#,
        "null",
    );
    let five_hours = r#"{"used_percent":5.0,"window_minutes":300,"resets_at":50}"#;
    let newer = token_count(
        five_hours,
        r#"{"used_percent":72.4,"window_minutes":10080,"resets_at":200}"#,
    );
    // the tail starts mid-line and ends on a line still being written
    let tail = format!("ial line\"}}\n{older}\n{{\"type\":\"response_item\"}}\n{newer}\n{{\"type\":\"event_");
    assert_eq!(
        codex_weekly(tail.as_bytes()),
        Some(Usage {
            used_percent: 72,
            resets_at: Some(200)
        })
    );
    // a turn whose limits name no weekly window leaves the older one's
    let tail = format!("{older}\n{}\n", token_count(five_hours, "null"));
    assert_eq!(
        codex_weekly(tail.as_bytes()),
        Some(Usage {
            used_percent: 10,
            resets_at: Some(100)
        })
    );
    assert_eq!(codex_weekly(b"{\"type\":\"session_meta\"}\n"), None);
}

#[test]
fn an_account_event_says_what_is_known() {
    assert!(Account::default().is_empty());
    assert_eq!(
        Account::default().event(3).to_ndjson(),
        r#"{"id":"account","cursor":3,"type":"account"}"#
    );
    let account = Account {
        email: Some("dev@example.com".to_owned()),
        weekly: Some(Usage {
            used_percent: 72,
            resets_at: None,
        }),
    };
    assert!(!account.is_empty());
    let event = account.event(9);
    assert_eq!(
        event.to_ndjson(),
        r#"{"id":"account","cursor":9,"type":"account","email":"dev@example.com","weekly":{"used_percent":72}}"#
    );
    let read: ChatEvent = serde_json::from_str(&event.to_ndjson()).unwrap();
    assert_eq!(read, event);
    let only_email: ChatEvent =
        serde_json::from_str(r#"{"id":"account","cursor":0,"type":"account","email":"a@b"}"#).unwrap();
    assert!(matches!(only_email.body, EventBody::Account { weekly: None, .. }));
}
