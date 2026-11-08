//! `ngtermctl agent commands`: the slash commands and skills of a pane's agent, for the Android chat composer's `/`
//! dropdown, which runs it on the origin under its ssh once per conversation. It reads the pane's record
//! (`agent.get`) and lists what the agent's config dir and project hold ([`catalog`]), the config dir taken from the
//! record's transcript so the agent's own `CLAUDE_CONFIG_DIR` or `CODEX_HOME` holds.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ngterm_core::agent_chat::catalog::catalog;
use ngterm_core::agent_chat::{AgentKind, NO_AGENT};
use ngterm_core::paths::Env;

use crate::{Failure, TargetOptions, client, socket};

/// `ngtermctl agent commands`: the pane's agent's slash commands and skills, as JSON.
#[derive(Debug, clap::Args)]
#[command(
    about = "Print the slash commands and skills of a pane's agent, as JSON.",
    long_about = "Reads the pane's agent record and prints the agent's built-in slash commands, its user and project \
                  skills and custom commands, and its enabled plugins' ones, as one JSON array of {name, \
                  description, kind, argument_hint}, argument_hint present only when the command declares one. A \
                  Codex skill is named as Codex mentions it ($name)."
)]
pub struct Commands {
    #[arg(long, help = "Which pane the agent runs in: left or right. Defaults to left.")]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[arg(long, help = "Override the control socket path.")]
    socket: Option<PathBuf>,
}

impl Commands {
    pub(super) fn validate(&self) -> Result<(), String> {
        super::validate_pane_side(self.pane.as_deref())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = socket::resolve(self.socket.as_deref(), env);
        let Some(record) = super::look(&socket, &self.target.target, self.pane.as_deref())? else {
            return Err(Failure::Local(NO_AGENT.to_owned()));
        };
        let agent = AgentKind::from_name(&record.agent)
            .ok_or_else(|| Failure::Local(format!("unknown agent: {}", record.agent)))?;
        let commands = catalog(agent, Path::new(&record.transcript_path)).map_err(Failure::Local)?;
        // a `SlashCommand` holds only strings and enums: serializing it cannot fail
        let json = serde_json::to_string(&commands).unwrap_or_default();
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{json}")
            .and_then(|()| stdout.flush())
            .map_err(|error| Failure::Local(format!("could not write the catalog: {}", client::os_message(&error))))
    }
}

#[cfg(test)]
pub(super) mod tests {
    #[cfg(target_os = "linux")]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn an_unknown_agent_is_reported_without_printing_a_catalog() {
        let (output, requests) = crate::messages::tests::run_command(
            &["agent", "commands", "--target", "S", "--pane", "right"],
            b"",
            &[],
            &[
                r#"{"ok":true,"result":{"agent":{"agent":"other","pane":"right","transcriptPath":"/private/c.jsonl","conversationId":"c","updatedAt":1.0}}}"#,
            ],
        );
        assert_eq!(
            requests,
            [serde_json::json!({"cmd":"agent.get","target":"S","args":{"pane":"right"}})]
        );
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(r#"PROBE_RESULT:Err(Local("unknown agent: other"))"#)
        );
        assert!(!String::from_utf8(output.stdout).unwrap().contains("[{"));
    }

    /// A process boundary gives the command its own stdout, so failing it cannot affect the test harness.
    #[cfg(target_os = "linux")]
    pub(crate) fn with_full_stdout(probe: &str, run: impl FnOnce()) {
        use std::os::fd::{AsFd as _, AsRawFd as _};
        if std::env::var("CTL_TAIL_FULL_STDOUT").ok().as_deref() != Some(probe) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([probe, "--exact", "--nocapture", "--test-threads=1"])
                .env_clear()
                .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
                .env("CTL_TAIL_FULL_STDOUT", probe)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("FULL_STDOUT_CHECKED")
            );
            return;
        }
        let full = std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
        let original = io::stdout().as_fd().try_clone_to_owned().unwrap();
        // SAFETY: `full` is live, and this subprocess owns its stdout descriptor.
        let replaced = unsafe { libc::dup2(full.as_raw_fd(), libc::STDOUT_FILENO) };
        assert_eq!(replaced, libc::STDOUT_FILENO);
        run();
        // SAFETY: `original` is live and restores this subprocess's stdout for the harness.
        let restored = unsafe { libc::dup2(original.as_raw_fd(), libc::STDOUT_FILENO) };
        assert_eq!(restored, libc::STDOUT_FILENO);
        eprintln!("FULL_STDOUT_CHECKED");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_catalog_write_failure_is_returned_to_the_caller() {
        with_full_stdout(
            "agent::commands::tests::a_catalog_write_failure_is_returned_to_the_caller",
            check_catalog_output,
        );
    }

    #[cfg(target_os = "linux")]
    fn check_catalog_output() {
        use std::os::unix::net::UnixListener;
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let transcript = directory.path().join("codex/sessions/c.jsonl");
        let response = serde_json::json!({"ok":true,"result":{"agent":{
            "agent":"codex","pane":"left","transcriptPath":transcript,"conversationId":"c","updatedAt":1.0
        }}});
        let server = std::thread::spawn(move || answer_catalog(&listener, &response));
        let command = Commands {
            pane: None,
            target: TargetOptions { target: "S".to_owned() },
            socket: Some(socket),
        };
        let result = command.run(&ngterm_core::paths::MapEnv::default());
        server.join().unwrap();
        assert!(matches!(result, Err(Failure::Local(message))
            if message == "could not write the catalog: No space left on device"));
    }

    #[cfg(target_os = "linux")]
    fn answer_catalog(listener: &std::os::unix::net::UnixListener, response: &serde_json::Value) {
        use std::io::BufRead as _;
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "catalog never connected");
                    std::thread::yield_now();
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            serde_json::json!({"cmd":"agent.get","target":"S","args":{}})
        );
        writeln!(stream, "{response}").unwrap();
    }
}
