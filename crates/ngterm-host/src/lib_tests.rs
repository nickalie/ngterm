use super::*;
use crate::fakes::FakeSessionHost;

/// Uses the optional job defaults rather than the fake's job tracking.
struct HostWithoutJobs(FakeSessionHost);

impl SessionHost for HostWithoutJobs {
    fn is_available(&self) -> bool {
        self.0.is_available()
    }

    fn list(&self) -> Result<Vec<LiveSessionRecord>, HostError> {
        self.0.list()
    }

    fn kill(&self, name: &str, force: bool) -> KillOutcome {
        self.0.kill(name, force)
    }

    fn screen(&self, name: &str, all: bool) -> Result<LiveScreen, HostError> {
        self.0.screen(name, all)
    }

    fn type_input(&self, name: &str, bytes: &[u8]) -> Result<(), HostError> {
        self.0.type_input(name, bytes)
    }
}

#[test]
fn host_errors_preserve_their_messages_without_a_source() {
    let closed: &dyn std::error::Error = &MainLoopClosed;
    assert_eq!(closed.to_string(), "the main loop has shut down");
    assert!(closed.source().is_none());

    for message in [
        "",
        "session not found: ngterm-pane",
        "daemon refused input\ntry again: λ",
    ] {
        let error = HostError(message.to_owned());
        let error: &dyn std::error::Error = &error;
        assert_eq!(error.to_string(), message);
        assert!(error.source().is_none());
    }
}

#[test]
fn command_line_spec_preserves_the_shell_invocation_and_leaves_other_options_empty() {
    for line in ["", r#"printf '%s' "a b" && exit 7"#] {
        let shell = ShellCommand::new(line);
        assert_eq!(
            LaunchSpec::command_line(line),
            LaunchSpec {
                program: shell.program,
                args: shell.args,
                raw_tail: shell.raw_tail,
                env: Vec::new(),
                cwd: None,
                stdin: None,
                capture_stderr: false,
            }
        );
    }
}

#[test]
fn a_batch_forces_every_kill_even_after_a_refusal_or_stale_socket() {
    for outcome in [
        KillOutcome::Killed,
        KillOutcome::StaleSocket,
        KillOutcome::Failed("permission denied".to_owned()),
    ] {
        let host = FakeSessionHost::new();
        let names = ["ngterm-first", "ngterm-next", "ngterm-last"].map(str::to_owned);
        for name in &names {
            host.insert(name, "$ ");
        }
        let all_killed = outcome == KillOutcome::Killed;
        host.set_kill_outcome(&names[0], (!all_killed).then_some(outcome));

        assert_eq!(host.kill_batch(&names), all_killed);
        assert_eq!(host.kills(), names.map(|name| (name, true)));
        assert!(host.session("ngterm-next").is_none());
        assert!(host.session("ngterm-last").is_none());
        assert_eq!(host.session("ngterm-first").is_none(), all_killed);
    }
}

#[test]
fn an_empty_batch_succeeds_without_asking_the_host_to_kill_anything() {
    let host = FakeSessionHost::new();
    host.insert("ngterm-kept", "$ ");
    assert!(host.kill_batch(&[]));
    assert!(host.kills().is_empty());
    assert!(host.session("ngterm-kept").is_some());
}

#[test]
fn optional_job_defaults_report_no_job_and_do_not_signal_processes() {
    let host = HostWithoutJobs(FakeSessionHost::new());
    let job = [ProcessRecord {
        pid: 42,
        started: 123,
        group: 42,
        foreground: 42,
    }];
    host.0.set_job(41, job.to_vec());

    assert_eq!(host.foreground_job(41), Some(Vec::new()));
    assert!(!host.job_running(&[]));
    assert!(!host.job_running(&job));
    host.end_job(&[]);
    host.end_job(&job);
    assert!(host.0.ended().is_empty());
    assert!(host.0.job_running(&job));
    assert!(host.0.kills().is_empty());
}
