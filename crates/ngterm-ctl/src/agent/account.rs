//! `agent chat`'s account watcher: the account the pane's agent runs under and its weekly limit, read on a thread of
//! its own about every [`REFRESH`] and at once when the transcript switches, and handed to the stream when it changes
//! ([`Watch`]). The readers are `ngterm_core::agent_chat::account`'s; the IO is here, behind [`Io`], so the tests
//! touch no network.
//!
//! - **Claude Code:** the email from its global config; the weekly limit from its usage endpoint, asked with its
//!   sign-in token from `<config>/.credentials.json`, or on macOS from the Keychain item Claude Code keeps it in:
//!   `Claude Code-credentials`, and with `CLAUDE_CONFIG_DIR` set `-` and the first 8 hex digits of the SHA-256 of that
//!   directory after it. The default name is asked only for `$HOME/.claude`, so a test's tree never reaches the
//!   user's own sign-in.
//! - **Codex:** the email from `auth.json`; the weekly limit from the rollout's last [`TAIL`] bytes.
//!
//! A part that can't be read (no sign-in, an expired token, offline, a refusal) is left out, never an error.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ngterm_core::agent_chat::AgentKind;
use ngterm_core::agent_chat::account::{
    Account, CLAUDE_OAUTH_BETA, CLAUDE_USAGE_URL, claude_config_files, claude_email, claude_token, claude_weekly,
    codex_email, codex_weekly,
};
use ngterm_core::agent_chat::hook::config_dir;
use sha2::{Digest as _, Sha256};

/// How often the account is read again.
pub(super) const REFRESH: Duration = Duration::from_mins(1);

/// How much of a rollout's end is searched for its rate limits.
const TAIL: u64 = 1024 * 1024;

/// The most a usage request takes.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the account is read from: the pane's agent and its transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Source {
    pub agent: AgentKind,
    pub transcript: PathBuf,
}

/// The reads the watcher makes: files, the Keychain, the usage endpoint and the time.
pub(super) trait Io {
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    /// The last `bytes` of the file at `path`.
    fn tail(&self, path: &Path, bytes: u64) -> Option<Vec<u8>>;
    /// The password of the Keychain item `service` (macOS).
    fn keychain(&self, service: &str) -> Option<Vec<u8>>;
    /// Claude's usage endpoint's answer to `token`, when it answered 200.
    fn usage(&self, token: &str) -> Option<Vec<u8>>;
    fn home(&self) -> Option<PathBuf>;
    fn now_ms(&self) -> u64;
}

/// The account `source` runs under, as far as it can be read.
pub(super) fn read_account(source: &Source, io: &dyn Io) -> Account {
    let Some(config) = config_dir(source.agent, &source.transcript) else {
        return Account::default();
    };
    match source.agent {
        AgentKind::Claude => Account {
            email: claude_config_files(&config)
                .iter()
                .find_map(|file| io.read(file).and_then(|bytes| claude_email(&bytes))),
            weekly: claude_credentials(&config, io)
                .and_then(|credentials| claude_token(&credentials, io.now_ms()))
                .and_then(|token| io.usage(&token))
                .and_then(|answer| claude_weekly(&answer)),
        },
        AgentKind::Codex => Account {
            email: io.read(&config.join("auth.json")).and_then(|auth| codex_email(&auth)),
            weekly: io.tail(&source.transcript, TAIL).and_then(|tail| codex_weekly(&tail)),
        },
    }
}

/// Claude Code's credentials for config directory `config`: the file, else (on macOS) its Keychain item.
fn claude_credentials(config: &Path, io: &dyn Io) -> Option<Vec<u8>> {
    if let Some(file) = io.read(&config.join(".credentials.json")) {
        return Some(file);
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let default = io.home().is_some_and(|home| home.join(".claude") == config);
    keychain_services(config, default)
        .iter()
        .find_map(|service| io.keychain(service))
}

/// The Keychain items Claude Code may keep config directory `config`'s credentials in, the likelier first: with
/// `default` (the directory is `$HOME/.claude`, where it needs no `CLAUDE_CONFIG_DIR`) the plain name, then the one
/// `CLAUDE_CONFIG_DIR` set to it would give; otherwise only that one.
pub(super) fn keychain_services(config: &Path, default: bool) -> Vec<String> {
    let digest = Sha256::digest(config.to_string_lossy().as_bytes());
    let hex = digest.iter().take(4).fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    let custom = format!("Claude Code-credentials-{hex}");
    if default {
        vec!["Claude Code-credentials".to_owned(), custom]
    } else {
        vec![custom]
    }
}

/// The machine's own reads.
struct System;

impl Io for System {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    fn tail(&self, path: &Path, bytes: u64) -> Option<Vec<u8>> {
        let mut file = File::open(path).ok()?;
        let length = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(bytes))).ok()?;
        let mut tail = Vec::new();
        file.take(bytes).read_to_end(&mut tail).ok()?;
        Some(tail)
    }

    fn keychain(&self, service: &str) -> Option<Vec<u8>> {
        let output = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", service, "-w"])
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    }

    fn usage(&self, token: &str) -> Option<Vec<u8>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        let mut answer = agent
            .get(CLAUDE_USAGE_URL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-beta", CLAUDE_OAUTH_BETA)
            .header("User-Agent", concat!("ngtermctl/", env!("CARGO_PKG_VERSION")))
            .call()
            .ok()?;
        answer.body_mut().with_config().limit(1024 * 1024).read_to_vec().ok()
    }

    fn home(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }

    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
    }
}

/// The watcher's thread, and what it reads.
pub(super) struct Watch {
    sources: Sender<Source>,
    accounts: Receiver<Account>,
}

impl Watch {
    /// Starts reading `source`'s account; `None` when the thread can't start (the chat goes on without it).
    pub(super) fn start(source: Source) -> Option<Self> {
        Self::with(source, System, REFRESH)
    }

    fn with(source: Source, io: impl Io + Send + 'static, every: Duration) -> Option<Self> {
        let (sources, wanted) = mpsc::channel();
        let (found, accounts) = mpsc::channel();
        thread::Builder::new()
            .name("ngtermctl.agent.account".to_owned())
            .spawn(move || watch(source, &io, every, &wanted, &found))
            .ok()?;
        Some(Self { sources, accounts })
    }

    /// Reads `source` from now on, at once.
    pub(super) fn switch(&self, source: Source) {
        let _ = self.sources.send(source);
    }

    /// The account read last, when it changed since the last call.
    pub(super) fn changed(&self) -> Option<Account> {
        self.accounts.try_iter().last()
    }
}

/// The thread: reads `source`'s account, sends it when it changed, and again after `every` or a new source, until the
/// stream goes.
fn watch(mut source: Source, io: &dyn Io, every: Duration, wanted: &Receiver<Source>, found: &Sender<Account>) {
    let mut last = None;
    loop {
        let account = read_account(&source, io);
        if last.as_ref() != Some(&account) {
            if found.send(account.clone()).is_err() {
                return;
            }
            last = Some(account);
        }
        match wanted.recv_timeout(every) {
            Ok(next) => source = next,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use ngterm_core::agent_chat::account::Usage;

    use super::*;

    /// Files by path, a usage answer for one token, and the tokens asked with.
    #[derive(Default, Clone)]
    struct Fake {
        files: HashMap<PathBuf, Vec<u8>>,
        usage: Option<(String, Vec<u8>)>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Io for Fake {
        fn read(&self, path: &Path) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }

        fn tail(&self, path: &Path, bytes: u64) -> Option<Vec<u8>> {
            let file = self.files.get(path)?;
            let from = file.len().saturating_sub(usize::try_from(bytes).unwrap());
            Some(file[from..].to_vec())
        }

        fn keychain(&self, _service: &str) -> Option<Vec<u8>> {
            None
        }

        fn usage(&self, token: &str) -> Option<Vec<u8>> {
            self.asked.lock().unwrap().push(token.to_owned());
            self.usage
                .as_ref()
                .filter(|(known, _)| known == token)
                .map(|(_, answer)| answer.clone())
        }

        fn home(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/u"))
        }

        fn now_ms(&self) -> u64 {
            1_000
        }
    }

    fn claude() -> Source {
        Source {
            agent: AgentKind::Claude,
            transcript: PathBuf::from("/home/u/.claude/projects/-w/c1.jsonl"),
        }
    }

    #[test]
    fn claudes_account_is_its_configs_email_and_the_usage_its_token_gets() {
        let mut io = Fake::default();
        io.files.insert(
            PathBuf::from("/home/u/.claude.json"),
            br#"{"oauthAccount":{"emailAddress":"dev@example.com"}}"#.to_vec(),
        );
        assert_eq!(
            read_account(&claude(), &io),
            Account {
                email: Some("dev@example.com".to_owned()),
                weekly: None
            }
        );
        assert!(io.asked.lock().unwrap().is_empty(), "no token, no request");

        io.files.insert(
            PathBuf::from("/home/u/.claude/.credentials.json"),
            br#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":5000}}"#.to_vec(),
        );
        io.usage = Some(("tok".to_owned(), br#"{"seven_day":{"utilization":42.0}}"#.to_vec()));
        assert_eq!(
            read_account(&claude(), &io).weekly,
            Some(Usage {
                used_percent: 42,
                resets_at: None
            })
        );
        assert_eq!(*io.asked.lock().unwrap(), ["tok"]);
    }

    #[test]
    fn an_expired_token_asks_nothing() {
        let mut io = Fake::default();
        io.files.insert(
            PathBuf::from("/home/u/.claude/.credentials.json"),
            br#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":1000}}"#.to_vec(),
        );
        assert_eq!(read_account(&claude(), &io), Account::default());
        assert!(io.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn codexs_account_is_its_auth_email_and_the_rollouts_weekly_window() {
        let rollout = PathBuf::from("/home/u/.codex/sessions/2026/11/05/rollout-x.jsonl");
        let mut io = Fake::default();
        io.files.insert(
            rollout.clone(),
            br#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":72.0,"window_minutes":10080,"resets_at":9}}}}
"#
            .to_vec(),
        );
        let source = Source {
            agent: AgentKind::Codex,
            transcript: rollout,
        };
        assert_eq!(
            read_account(&source, &io),
            Account {
                email: None,
                weekly: Some(Usage {
                    used_percent: 72,
                    resets_at: Some(9)
                })
            }
        );
    }

    #[test]
    fn a_transcript_outside_an_agents_tree_reads_nothing() {
        let source = Source {
            agent: AgentKind::Claude,
            transcript: PathBuf::from("/tmp/c1.jsonl"),
        };
        assert_eq!(read_account(&source, &Fake::default()), Account::default());
    }

    #[test]
    fn the_keychain_item_is_named_as_claude_code_names_it() {
        let config = Path::new("/Users/u/.claude");
        // printf %s /Users/u/.claude | sha256sum
        let hashed = "Claude Code-credentials-ff2684fe".to_owned();
        assert_eq!(
            keychain_services(config, true),
            ["Claude Code-credentials".to_owned(), hashed.clone()]
        );
        assert_eq!(keychain_services(config, false), [hashed]);
    }

    #[test]
    fn the_watcher_sends_an_account_when_it_changes_and_reads_a_new_source_at_once() {
        let mut io = Fake::default();
        io.files.insert(
            PathBuf::from("/home/u/.claude.json"),
            br#"{"oauthAccount":{"emailAddress":"dev@example.com"}}"#.to_vec(),
        );
        let watch = Watch::with(claude(), io, Duration::from_millis(10)).unwrap();
        let first = watch.accounts.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.email.as_deref(), Some("dev@example.com"));
        // reading the same account again sends nothing
        thread::sleep(Duration::from_millis(50));
        assert_eq!(watch.changed(), None);
        watch.switch(Source {
            agent: AgentKind::Codex,
            transcript: PathBuf::from("/tmp/elsewhere.jsonl"),
        });
        let switched = watch.accounts.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(switched, Account::default());
    }

    #[test]
    fn system_files_are_read_whole_or_as_a_bounded_tail() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("rollout.jsonl");
        assert_eq!(System.read(&file), None);
        assert_eq!(System.tail(&file, 4), None);
        std::fs::write(&file, b"first\nlast\n").unwrap();
        assert_eq!(System.read(&file), Some(b"first\nlast\n".to_vec()));
        assert_eq!(System.tail(&file, 5), Some(b"last\n".to_vec()));
        assert_eq!(System.tail(&file, 99), Some(b"first\nlast\n".to_vec()));
        assert_eq!(System.tail(&file, 0), Some(Vec::new()));
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let now = u128::from(System.now_ms());
        let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        assert!((before..=after).contains(&now));
        assert_eq!(System.home(), std::env::home_dir());
    }

    #[test]
    fn the_watch_stops_when_either_channel_is_disconnected() {
        let (sources, wanted) = mpsc::channel();
        let (found, accounts) = mpsc::channel();
        drop(accounts);
        // A closed account stream ends immediately, even while the source stream is open.
        watch(claude(), &Fake::default(), REFRESH, &wanted, &found);
        drop(sources);
        let (found, accounts) = mpsc::channel();
        watch(claude(), &Fake::default(), REFRESH, &wanted, &found);
        assert_eq!(accounts.try_recv().unwrap(), Account::default());
    }

    #[test]
    fn a_system_watch_switches_between_private_config_trees() {
        let directory = tempfile::tempdir().unwrap();
        let source = Source {
            agent: AgentKind::Codex,
            transcript: directory.path().join("sessions/x.jsonl"),
        };
        std::fs::create_dir_all(source.transcript.parent().unwrap()).unwrap();
        let watch = Watch::start(source.clone()).unwrap();
        assert_eq!(
            watch.accounts.recv_timeout(Duration::from_secs(5)).unwrap(),
            Account::default()
        );
        watch.switch(source);
        drop(watch);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn an_unavailable_keychain_tool_leaves_credentials_unknown() {
        assert_eq!(System.keychain("ngterm-coverage-nonexistent-item"), None);
    }

    #[test]
    fn codex_reads_the_email_from_the_transcripts_own_config() {
        use base64::Engine as _;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"codex@example.com"}"#);
        let auth = serde_json::json!({"tokens":{"id_token":format!("header.{claims}.signature")}});
        let config = PathBuf::from("/private/codex");
        let mut io = Fake::default();
        io.files
            .insert(config.join("auth.json"), serde_json::to_vec(&auth).unwrap());
        let source = Source {
            agent: AgentKind::Codex,
            transcript: config.join("sessions/today/rollout.jsonl"),
        };
        assert_eq!(
            read_account(&source, &io),
            Account {
                email: Some("codex@example.com".to_owned()),
                weekly: None
            }
        );
        assert!(io.asked.lock().unwrap().is_empty());
    }
}
