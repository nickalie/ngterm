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

/// Claude's usage endpoint and transport. Shipped builds only construct [`UsageEndpoint::claude`]; tests can use
/// plain HTTP on a local server without changing where a user's sign-in token goes.
struct UsageEndpoint {
    url: String,
    agent: ureq::Agent,
}

impl UsageEndpoint {
    fn claude() -> Self {
        Self {
            url: CLAUDE_USAGE_URL.to_owned(),
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .timeout_global(Some(TIMEOUT))
                .build()
                .into(),
        }
    }

    #[cfg(test)]
    fn local(url: String) -> Self {
        Self {
            url,
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .timeout_global(Some(TIMEOUT))
                .build()
                .into(),
        }
    }
}

/// The machine's own reads.
struct System {
    usage: UsageEndpoint,
}

impl System {
    fn new() -> Self {
        Self {
            usage: UsageEndpoint::claude(),
        }
    }
}

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
        let mut answer = self
            .usage
            .agent
            .get(&self.usage.url)
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
        Self::with(source, System::new(), REFRESH)
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
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use ngterm_core::agent_chat::account::Usage;

    use super::*;

    /// One loopback request, with a bounded wait even if the client fails before connecting.
    struct UsageServer {
        url: String,
        request: thread::JoinHandle<String>,
    }

    impl UsageServer {
        fn new(response: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/api/oauth/usage", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let request = thread::spawn(move || {
                let mut stream = accept_usage_request(&listener);
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let request = usage_request_headers(&mut stream);
                // A refused or oversized response may be closed before its body is sent.
                let _ = stream.write_all(&response);
                request
            });
            Self { url, request }
        }

        fn answer(status: &str, body: &[u8]) -> Self {
            let mut response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(body);
            Self::new(response)
        }

        fn system(&self) -> System {
            System {
                usage: UsageEndpoint::local(self.url.clone()),
            }
        }
    }

    fn accept_usage_request(listener: &TcpListener) -> TcpStream {
        let started = Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(started.elapsed() < Duration::from_secs(5), "no usage request");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("usage server: {error}"),
            }
        }
    }

    fn usage_request_headers(stream: &mut TcpStream) -> String {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            request.push_str(&line);
            if line == "\r\n" {
                return request;
            }
        }
    }

    fn signed_in_account(io: &System) -> Account {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join(".claude");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(
            config.join(".claude.json"),
            br#"{"oauthAccount":{"emailAddress":"dev@example.com"}}"#,
        )
        .unwrap();
        std::fs::write(
            config.join(".credentials.json"),
            br#"{"claudeAiOauth":{"accessToken":"test-token"}}"#,
        )
        .unwrap();
        read_account(
            &Source {
                agent: AgentKind::Claude,
                transcript: config.join("projects/-w/run.jsonl"),
            },
            io,
        )
    }

    #[test]
    fn the_shipped_usage_endpoint_is_fixed_and_requires_https() {
        let endpoint = UsageEndpoint::claude();
        assert_eq!(endpoint.url, "https://api.anthropic.com/api/oauth/usage");
        assert!(endpoint.agent.config().https_only());
        assert_eq!(endpoint.agent.config().timeouts().global, Some(Duration::from_secs(10)));
        assert!(matches!(
            endpoint.agent.get("http://127.0.0.1:1/api/oauth/usage").call(),
            Err(ureq::Error::RequireHttpsOnly(_))
        ));
    }

    #[test]
    fn system_usage_sends_the_oauth_headers_and_reads_the_weekly_limit() {
        let server = UsageServer::answer(
            "200 OK",
            br#"{"seven_day":{"utilization":42.4,"resets_at":"1970-01-01T00:00:09Z"}}"#,
        );
        let account = signed_in_account(&server.system());
        let request = server.request.join().unwrap();
        assert_eq!(request.lines().next(), Some("GET /api/oauth/usage HTTP/1.1"));
        for header in [
            "authorization: Bearer test-token".to_owned(),
            "anthropic-beta: oauth-2025-04-20".to_owned(),
            format!("user-agent: ngtermctl/{}", env!("CARGO_PKG_VERSION")),
        ] {
            assert!(
                request.lines().any(|line| line == header),
                "missing {header}: {request}"
            );
        }
        assert_eq!(
            account,
            Account {
                email: Some("dev@example.com".to_owned()),
                weekly: Some(Usage {
                    used_percent: 42,
                    resets_at: Some(9),
                }),
            }
        );
    }

    #[test]
    fn system_usage_parses_new_limits_and_leaves_unknown_limits_out() {
        for (body, weekly) in [
            (
                br#"{"limits":[{"kind":"weekly_all","percent":73}]}"#.as_slice(),
                Some(Usage {
                    used_percent: 73,
                    resets_at: None,
                }),
            ),
            (b"not json", None),
            (br#"{"seven_day":null}"#, None),
            (b"", None),
        ] {
            let server = UsageServer::answer("200 OK", body);
            let account = signed_in_account(&server.system());
            server.request.join().unwrap();
            assert_eq!(account.email.as_deref(), Some("dev@example.com"));
            assert_eq!(account.weekly, weekly);
        }
    }

    #[test]
    fn system_usage_refusals_leave_the_signed_in_email() {
        for status in [
            "401 Unauthorized",
            "403 Forbidden",
            "429 Too Many Requests",
            "500 Internal Server Error",
        ] {
            let server = UsageServer::answer(status, br#"{"seven_day":{"utilization":42}}"#);
            let account = signed_in_account(&server.system());
            server.request.join().unwrap();
            assert_eq!(
                account,
                Account {
                    email: Some("dev@example.com".to_owned()),
                    weekly: None,
                }
            );
        }
    }

    #[test]
    fn system_usage_rejects_broken_responses_and_incomplete_bodies() {
        for response in [
            b"not an HTTP response\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\ninvalid\r\n",
        ] {
            let server = UsageServer::new(response.to_vec());
            let usage = server.system().usage("test-token");
            server.request.join().unwrap();
            assert_eq!(usage, None);
        }
    }

    #[test]
    fn system_usage_reads_at_most_one_mebibyte() {
        // ureq's limit is exclusive: reaching it is a refusal, even at the body's end.
        let body = vec![b' '; 1024 * 1024 - 1];
        let server = UsageServer::answer("200 OK", &body);
        let usage = server.system().usage("test-token");
        server.request.join().unwrap();
        assert_eq!(usage.as_deref().map(<[u8]>::len), Some(body.len()));
        assert!(usage.unwrap().iter().all(|byte| *byte == b' '));

        for length in [1024 * 1024, 1024 * 1024 + 1] {
            let server = UsageServer::answer("200 OK", &vec![b' '; length]);
            let usage = server.system().usage("test-token");
            server.request.join().unwrap();
            assert_eq!(usage, None);
        }
    }

    #[test]
    fn system_usage_leaves_connection_failures_unknown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/oauth/usage", listener.local_addr().unwrap());
        drop(listener);
        let io = System {
            usage: UsageEndpoint::local(url),
        };
        assert_eq!(io.usage("test-token"), None);
    }

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
        assert_eq!(System::new().read(&file), None);
        assert_eq!(System::new().tail(&file, 4), None);
        std::fs::write(&file, b"first\nlast\n").unwrap();
        assert_eq!(System::new().read(&file), Some(b"first\nlast\n".to_vec()));
        assert_eq!(System::new().tail(&file, 5), Some(b"last\n".to_vec()));
        assert_eq!(System::new().tail(&file, 99), Some(b"first\nlast\n".to_vec()));
        assert_eq!(System::new().tail(&file, 0), Some(Vec::new()));
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let now = u128::from(System::new().now_ms());
        let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        assert!((before..=after).contains(&now));
        assert_eq!(System::new().home(), std::env::home_dir());
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
        use base64::Engine as _;
        let codex_tree = |directory: &Path| {
            let source = Source {
                agent: AgentKind::Codex,
                transcript: directory.join("sessions/x.jsonl"),
            };
            std::fs::create_dir_all(source.transcript.parent().unwrap()).unwrap();
            source
        };
        let unknown = tempfile::tempdir().unwrap();
        let signed_in = tempfile::tempdir().unwrap();
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"codex@example.com"}"#);
        let auth = serde_json::json!({"tokens":{"id_token":format!("header.{claims}.signature")}});
        std::fs::write(signed_in.path().join("auth.json"), auth.to_string()).unwrap();
        let watch = Watch::start(codex_tree(unknown.path())).unwrap();
        assert_eq!(
            watch.accounts.recv_timeout(Duration::from_secs(5)).unwrap(),
            Account::default()
        );
        watch.switch(codex_tree(signed_in.path()));
        assert_eq!(
            watch.accounts.recv_timeout(Duration::from_secs(5)).unwrap(),
            Account {
                email: Some("codex@example.com".to_owned()),
                weekly: None
            }
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn an_unavailable_keychain_tool_leaves_credentials_unknown() {
        assert_eq!(System::new().keychain("ngterm-coverage-nonexistent-item"), None);
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
    #[test]
    fn unavailable_or_malformed_usage_keeps_the_signed_in_email() {
        let mut io = Fake::default();
        io.files.insert(
            PathBuf::from("/home/u/.claude.json"),
            br#"{"oauthAccount":{"emailAddress":"dev@example.com"}}"#.to_vec(),
        );
        io.files.insert(
            PathBuf::from("/home/u/.claude/.credentials.json"),
            br#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":5000}}"#.to_vec(),
        );
        for answer in [
            None,
            Some(b"not json".to_vec()),
            Some(br#"{"seven_day":null}"#.to_vec()),
        ] {
            io.usage = answer.map(|answer| ("tok".to_owned(), answer));
            io.asked.lock().unwrap().clear();
            assert_eq!(
                read_account(&claude(), &io),
                Account {
                    email: Some("dev@example.com".to_owned()),
                    weekly: None,
                }
            );
            assert_eq!(*io.asked.lock().unwrap(), ["tok"]);
        }
    }
}
