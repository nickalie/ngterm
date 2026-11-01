//! The bash and zsh shell integration the agent integrations install (`resources/agent-status/shell/integration.sh`),
//! sourced for real into bash with a recording stand-in for the agent-status wrapper (`NGTERM_AGENT_BIN`).
//!
//! Ported from upstream's `agtermCore/Tests/agtermCoreTests/AgentStatusIntegrationTests.swift` (v0.39.1), whole, in
//! this build's names. The integration is POSIX shell, so this test is Unix-only; fish's form did not change.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ngterm_host::fakes::write_executable;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn integration() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/shell/integration.sh")
}

/// Sources the integration into a bash whose `PROMPT_COMMAND` already holds a user entry, leaves the user's last
/// command with `status`, then evaluates `PROMPT_COMMAND` as bash does before a prompt. Returns what the user entry saw
/// and the stand-in's recorded calls.
fn run_prompt(agent_ran: bool, status: i32) -> TestResult<(String, Vec<String>)> {
    let dir = tempfile::tempdir()?;
    let calls = dir.path().join("calls");
    let stub = dir.path().join("agent-status");
    write_executable(
        &stub,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", calls.display()),
    )?;
    let integration = integration();
    let script = format!(
        "PROMPT_COMMAND='echo \"status=$? active=${{_ags_active-unset}}\"'\n\
         source '{}'\n\
         {}\n\
         (exit {status})\n\
         eval \"$PROMPT_COMMAND\"\n",
        integration.display(),
        if agent_ran { "_ags_active=1" } else { ":" },
    );
    let out = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("NGTERM_SESSION_ID", "sid")
        .env("NGTERM_AGENT_BIN", &stub)
        .output()?;
    let seen = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let recorded = fs::read_to_string(&calls)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Ok((seen, recorded))
}

#[test]
fn precmd_keeps_the_last_command_status_for_later_prompt_command_entries() -> TestResult {
    for agent_ran in [false, true] {
        for status in [0, 3] {
            let (seen, calls) = run_prompt(agent_ran, status)?;
            assert_eq!(
                seen,
                format!("status={status} active=unset"),
                "agent_ran={agent_ran} status={status}"
            );
            let expected: &[&str] = if agent_ran { &["idle"] } else { &[] };
            assert_eq!(calls, expected, "agent_ran={agent_ran} status={status}");
        }
    }
    Ok(())
}
