//! The `OpenCode` v1 status plugin the agent integrations install (`resources/agent-status/opencode/ngterm-status.js`),
//! run for real: its event-to-status mapping and its wrapper spawning belong to the installed plugin, not to the app's
//! status engine. Each test drives the plugin's `event` hook under `node` (the host `OpenCode` runs it in) with a
//! recording reporter in place of the status wrapper, and asserts the reporter command lines it was handed.
//!
//! Ported from upstream's `agtermCore/Tests/agtermCoreTests/OpenCodeStatusHookTests.swift` (v0.38.1), whole, in this
//! build's names (`NgtermStatusPlugin`, `NGTERM_SESSION_ID`, `NGTERM_STATUS_WRAPPER`, the `ngterm` default wrapper).
//!
//! The plugin is a bare ES module `.js` with no `package.json` above it, which only Node with module-syntax detection
//! (22.7+, or 20.19+ on the 20.x line) imports. A missing or older `node` prints `SKIPPED:` and passes, unless
//! `NGTERM_REQUIRE_NODE_TESTS` is set, as CI sets it: then it fails.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The PATH the plugin runs under: the reporter scripts' shell and nothing of the developer's.
const PATH: &str = "/usr/bin:/bin";

fn plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/opencode/ngterm-status.js")
}

/// The real `node` executable (past a version manager's shim, which needs the developer's environment) when it meets
/// the module-syntax-detection floor, or why there is none.
fn resolve_node() -> Result<PathBuf, String> {
    let out = Command::new("node")
        .args([
            "-e",
            "console.log(process.execPath); console.log(process.versions.node)",
        ])
        .output()
        .map_err(|err| format!("no node on PATH ({err})"))?;
    if !out.status.success() {
        return Err(format!("node exited {}", out.status));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let (Some(path), Some(version)) = (lines.next(), lines.next()) else {
        return Err(format!("node printed {text:?}"));
    };
    let mut parts = version.split('.').map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let meets_floor = match major {
        23.. => true,
        22 => minor >= 7,
        20 => minor >= 19,
        _ => false,
    };
    if meets_floor {
        Ok(PathBuf::from(path))
    } else {
        Err(format!("node {version} is older than 22.7 (or 20.19 on the 20.x line)"))
    }
}

/// The `node` to run the plugin under, or `None` after printing why the test skips; fails instead under
/// `NGTERM_REQUIRE_NODE_TESTS`.
fn node() -> Option<&'static Path> {
    static NODE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    match NODE.get_or_init(resolve_node) {
        Ok(path) => Some(path),
        Err(why) => {
            assert!(
                std::env::var_os("NGTERM_REQUIRE_NODE_TESTS").is_none(),
                "the OpenCode plugin needs Node.js 22.7+ (or 20.19+): {why}"
            );
            eprintln!("SKIPPED: the OpenCode plugin needs Node.js 22.7+ (or 20.19+): {why}");
            None
        }
    }
}

/// Runs `script` under `node` in exactly `env` (nothing of this process's environment, the user's `NGTERM_*` least
/// of all); fails with node's output when it exits non-zero.
fn run_node(node: &Path, script: &Path, env: &[(&str, &Path)]) -> TestResult {
    let mut command = Command::new(node);
    command.arg(script).env_clear().env("PATH", PATH);
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command.output()?;
    assert!(
        out.status.success(),
        "node exited {}\nscript: {}\nstderr:\n{}\nstdout:\n{}",
        out.status,
        script.display(),
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout),
    );
    Ok(())
}

fn write_executable(path: &Path, text: &str) -> TestResult {
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A reporter that appends its argv, one line per call, to `statuses`.
fn recorder(statuses: &Path) -> String {
    format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", statuses.display())
}

/// The harness stays out of the resources (they ship to users). It is `.mjs`, so Node loads the plugin's `.js` as ESM
/// through a dynamic import and module-syntax detection, with no `--experimental-*` flag.
const EVENTS_HARNESS: &str = r#"import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const { NgtermStatusPlugin } = await import(pathToFileURL(process.env.NGTERM_OPENCODE_PLUGIN).href);
const events = JSON.parse(readFileSync(process.env.NGTERM_OPENCODE_EVENTS, "utf8"));
const hooks = await NgtermStatusPlugin();
if (typeof hooks.event !== "function") process.exit(0);
for (const event of events) {
  await hooks.event({ event });
}
"#;

/// How the plugin finds its reporter and its pane.
#[derive(Clone, Copy)]
struct Setup {
    /// `NGTERM_STATUS_WRAPPER` names the recorder; without it the recorder sits at the default wrapper path.
    status_wrapper: bool,
    /// `NGTERM_SESSION_ID`, or none: outside ngterm.
    session_id: Option<&'static str>,
}

const IN_NGTERM: Setup = Setup {
    status_wrapper: true,
    session_id: Some("sid"),
};

/// The reporter command lines the plugin's `event` hook spawned for each `OpenCode` bus payload of `events`, in order.
fn run_events(node: &Path, events: &[Value], setup: Setup) -> TestResult<Vec<String>> {
    let dir = tempfile::tempdir()?;
    let statuses = dir.path().join("statuses");
    let home = dir.path().join("home");
    fs::create_dir(&home)?;
    let status_wrapper = dir.path().join("status-wrapper");
    write_executable(&status_wrapper, &recorder(&statuses))?;
    if !setup.status_wrapper {
        let default_dir = home.join(".config/ngterm/agent-status");
        fs::create_dir_all(&default_dir)?;
        write_executable(&default_dir.join("ngterm-agent-status.sh"), &recorder(&statuses))?;
    }
    let events_file = dir.path().join("events.json");
    fs::write(&events_file, serde_json::to_string(events)?)?;
    let harness = dir.path().join("harness.mjs");
    fs::write(&harness, EVENTS_HARNESS)?;

    let plugin = plugin();
    let session_id = setup.session_id.map(PathBuf::from);
    let mut env: Vec<(&str, &Path)> = vec![
        ("HOME", &home),
        ("NGTERM_OPENCODE_PLUGIN", &plugin),
        ("NGTERM_OPENCODE_EVENTS", &events_file),
    ];
    if let Some(session_id) = &session_id {
        env.push(("NGTERM_SESSION_ID", session_id));
    }
    if setup.status_wrapper {
        env.push(("NGTERM_STATUS_WRAPPER", &status_wrapper));
    }
    run_node(node, &harness, &env)?;
    Ok(lines(&statuses))
}

/// [`run_events`] inside an ngterm pane with `NGTERM_STATUS_WRAPPER` set.
fn calls(node: &Path, events: &[Value]) -> TestResult<Vec<String>> {
    run_events(node, events, IN_NGTERM)
}

fn event(kind: &str, properties: &Value) -> Value {
    json!({ "type": kind, "properties": properties })
}

fn bare(kind: &str) -> Value {
    event(kind, &json!({}))
}

fn status(kind: &str, session_id: &str) -> Value {
    event(
        "session.status",
        &json!({ "sessionID": session_id, "status": { "type": kind } }),
    )
}

fn root(kind: &str) -> Value {
    status(kind, "ses_root")
}

fn asked(kind: &str, id: &str) -> Value {
    event(kind, &json!({ "sessionID": "ses_root", "id": id }))
}

fn answered(kind: &str, request_id: &str) -> Value {
    event(kind, &json!({ "sessionID": "ses_root", "requestID": request_id }))
}

fn error(session_id: &str, name: &str) -> Value {
    event(
        "session.error",
        &json!({ "sessionID": session_id, "error": { "name": name } }),
    )
}

#[test]
fn the_plugin_exports_only_plugin_functions() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let dir = tempfile::tempdir()?;
    // .mjs: `node -e` would need --input-type=module, and --experimental-default-type=module is gone
    let harness = dir.path().join("exports.mjs");
    fs::write(
        &harness,
        r#"import { pathToFileURL } from "node:url";
const mod = await import(pathToFileURL(process.env.NGTERM_OPENCODE_PLUGIN).href);
const keys = Object.keys(mod);
if (keys.length !== 1 || keys[0] !== "NgtermStatusPlugin") process.exit(2);
if (typeof mod.NgtermStatusPlugin !== "function") process.exit(3);
for (const [name, value] of Object.entries(mod)) {
  if (typeof value !== "function") process.exit(4);
}
"#,
    )?;
    run_node(node, &harness, &[("NGTERM_OPENCODE_PLUGIN", &plugin())])
}

#[test]
fn lifecycle_events_drive_only_the_generic_status_wrapper() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [
        root("busy"),
        root("retry"),
        root("idle"),
        bare("permission.asked"),
        bare("question.asked"),
        bare("permission.replied"),
        bare("question.replied"),
        bare("question.rejected"),
    ];
    assert_eq!(
        calls(node, &events)?,
        [
            "active --blink",
            "active --blink",
            "completed --auto-reset",
            "blocked",
            "blocked",
            "active --blink",
            "active --blink",
            "active --blink",
        ]
    );
    Ok(())
}

#[test]
fn a_reply_to_one_prompt_stays_blocked_while_another_is_pending() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [
        root("busy"),
        asked("permission.asked", "per_1"),
        asked("question.asked", "que_1"),
        asked("permission.asked", "per_2"),
        answered("permission.replied", "per_1"),
        answered("question.rejected", "que_1"),
        answered("permission.replied", "per_2"),
    ];
    assert_eq!(
        calls(node, &events)?,
        [
            "active --blink",
            "blocked",
            "blocked",
            "blocked",
            "blocked",
            "blocked",
            "active --blink",
        ]
    );
    Ok(())
}

#[test]
fn a_prompt_abandoned_at_idle_does_not_hold_back_later_replies() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [
        root("busy"),
        asked("permission.asked", "per_1"),
        root("idle"),
        root("busy"),
        asked("permission.asked", "per_2"),
        answered("permission.replied", "per_2"),
    ];
    assert_eq!(
        calls(node, &events)?,
        [
            "active --blink",
            "blocked",
            "completed --auto-reset",
            "active --blink",
            "blocked",
            "active --blink",
        ]
    );
    Ok(())
}

#[test]
fn a_session_error_stays_blocked_through_the_following_idle() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // SessionProcessor.halt publishes session.error then status(idle) on the next line; without an errored latch the
    // serial queue would overwrite blocked with completed --auto-reset.
    let events = [root("busy"), error("ses_root", "ProviderError"), root("idle")];
    assert_eq!(calls(node, &events)?, ["active --blink", "blocked"]);
    Ok(())
}

#[test]
fn an_abort_error_does_not_latch_blocked() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // Esc → halt(AbortError) → MessageAbortedError + idle. Must end on completed --auto-reset (self-clears on visit),
    // not stuck blocked ahead of the keystroke clear.
    let events = [root("busy"), error("ses_root", "MessageAbortedError"), root("idle")];
    assert_eq!(calls(node, &events)?, ["active --blink", "completed --auto-reset"]);
    Ok(())
}

#[test]
fn a_context_overflow_followed_by_busy_does_not_paint_blocked() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // Auto-compaction publishes ContextOverflowError and then resumes the loop, so the next event is busy: the
    // overflow was a hiccup, not a failed turn, and must not flash blocked.
    let events = [
        root("busy"),
        error("ses_root", "ContextOverflowError"),
        root("busy"),
        root("idle"),
    ];
    assert_eq!(
        calls(node, &events)?,
        ["active --blink", "active --blink", "completed --auto-reset"]
    );
    Ok(())
}

#[test]
fn a_context_overflow_followed_by_idle_reports_blocked() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // With compaction disabled (or once it gives up) halt publishes the same error and then idle: the turn really
    // ended in failure, so this must land on blocked rather than completed.
    let events = [root("busy"), error("ses_root", "ContextOverflowError"), root("idle")];
    assert_eq!(calls(node, &events)?, ["active --blink", "blocked"]);
    Ok(())
}

#[test]
fn an_idle_without_a_preceding_busy_is_ignored() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [root("idle"), status("idle", "ses_never_busy")];
    assert!(calls(node, &events)?.is_empty());
    Ok(())
}

#[test]
fn a_subagent_idle_does_not_complete_while_the_parent_is_active() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // The task tool creates a child session with its own busy/idle while the parent stays busy. The permission between
    // the two idles is the oracle: a gate that completed on the child's idle would place completed BEFORE blocked
    // instead of after it.
    let events = [
        status("busy", "ses_parent"),
        status("busy", "ses_child"),
        status("idle", "ses_child"),
        event("permission.asked", &json!({ "sessionID": "ses_parent" })),
        status("idle", "ses_parent"),
    ];
    assert_eq!(
        calls(node, &events)?,
        ["active --blink", "active --blink", "blocked", "completed --auto-reset"]
    );
    Ok(())
}

#[test]
fn interleaved_top_level_sessions_complete_only_when_all_are_idle() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [
        status("busy", "ses_a"),
        status("busy", "ses_b"),
        status("idle", "ses_a"),
        status("busy", "ses_a"),
        status("idle", "ses_b"),
        status("idle", "ses_a"),
    ];
    assert_eq!(
        calls(node, &events)?,
        [
            "active --blink",
            "active --blink",
            "active --blink",
            "completed --auto-reset",
        ]
    );
    Ok(())
}

#[test]
fn a_sibling_idle_does_not_complete_over_another_sessions_error() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // halt() publishes session.error then that session's own idle while a sibling still runs: the latch must outlive
    // the swallowed idle, or the sibling's idle paints completed over blocked.
    let events = [
        status("busy", "ses_a"),
        status("busy", "ses_b"),
        error("ses_a", "ProviderAuthError"),
        status("idle", "ses_a"),
        status("idle", "ses_b"),
    ];
    assert_eq!(calls(node, &events)?, ["active --blink", "active --blink", "blocked"]);
    Ok(())
}

#[test]
fn a_turn_after_an_error_completes_normally() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // The set-wide latch must not leak into the next turn.
    let events = [
        root("busy"),
        error("ses_root", "ProviderAuthError"),
        root("idle"),
        root("busy"),
        root("idle"),
    ];
    assert_eq!(
        calls(node, &events)?,
        ["active --blink", "blocked", "active --blink", "completed --auto-reset"]
    );
    Ok(())
}

#[test]
fn the_deprecated_session_idle_and_non_status_noise_are_ignored() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let events = [
        event("session.idle", &json!({ "sessionID": "s1" })),
        bare("tool.execute.before"),
        bare("tool.execute.after"),
        bare("session.compacted"),
        bare("chat.message"),
        event(
            "session.created",
            &json!({ "info": { "id": "child", "parentID": "root" } }),
        ),
    ];
    assert!(calls(node, &events)?.is_empty());
    Ok(())
}

#[test]
fn the_plugin_is_a_silent_no_op_outside_ngterm() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let outside = Setup {
        session_id: None,
        ..IN_NGTERM
    };
    assert!(run_events(node, &[root("busy")], outside)?.is_empty());
    Ok(())
}

#[test]
fn the_default_wrapper_path_is_used_when_the_status_wrapper_is_unset() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    let default_wrapper = Setup {
        status_wrapper: false,
        ..IN_NGTERM
    };
    assert_eq!(
        run_events(node, &[bare("permission.asked")], default_wrapper)?,
        ["blocked"]
    );
    Ok(())
}

/// Fires every event at once and checks, while the reporter holds `blocked` until `release` exists, that the next
/// report has not started (exit 2) nor anything finished (exit 3).
const ORDER_HARNESS: &str = r#"import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
const { NgtermStatusPlugin } = await import(pathToFileURL(process.env.NGTERM_OPENCODE_PLUGIN).href);
const events = JSON.parse(readFileSync(process.env.NGTERM_OPENCODE_EVENTS, "utf8"));
const hooks = await NgtermStatusPlugin();
const pending = Promise.all(events.map((event) => hooks.event({ event })));
const startedPath = process.env.NGTERM_OPENCODE_STARTED;
const releasePath = process.env.NGTERM_OPENCODE_RELEASE;
const statusesPath = process.env.NGTERM_OPENCODE_STATUSES;
for (let i = 0; i < 500 && !existsSync(startedPath); i++) await sleep(10);
// While blocked is held, the follow-up active report must not have started (the queue serializes).
const started = existsSync(startedPath) ? readFileSync(startedPath, "utf8") : "";
if (started.includes("active")) {
  writeFileSync(releasePath, "1");
  await pending;
  process.exit(2);
}
if (existsSync(statusesPath) && readFileSync(statusesPath, "utf8").trim()) {
  writeFileSync(releasePath, "1");
  await pending;
  process.exit(3);
}
writeFileSync(releasePath, "1");
await pending;
"#;

#[test]
fn reports_run_strictly_in_order() -> TestResult {
    let Some(node) = node() else {
        return Ok(());
    };
    // The gate protocol: blocked holds until `release` exists; the next report must not start until then.
    let dir = tempfile::tempdir()?;
    let statuses = dir.path().join("statuses");
    let started = dir.path().join("started");
    let release = dir.path().join("release");
    let wrapper = dir.path().join("status-wrapper");
    write_executable(
        &wrapper,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"start $*\" >> '{started}'\nif [ \"$1\" = \"blocked\" ]; then\n  while [ ! -f \
             '{release}' ]; do sleep 0.01; done\nfi\nprintf '%s\\n' \"$*\" >> '{statuses}'\n",
            started = started.display(),
            release = release.display(),
            statuses = statuses.display(),
        ),
    )?;
    // permission.asked / replied always report (no active-set gate), so the queue order is isolated
    let events_file = dir.path().join("events.json");
    let events = [bare("permission.asked"), bare("permission.replied")];
    fs::write(&events_file, serde_json::to_string(&events)?)?;
    let harness = dir.path().join("harness.mjs");
    fs::write(&harness, ORDER_HARNESS)?;

    let plugin = plugin();
    run_node(
        node,
        &harness,
        &[
            ("HOME", dir.path()),
            ("NGTERM_SESSION_ID", Path::new("sid")),
            ("NGTERM_STATUS_WRAPPER", &wrapper),
            ("NGTERM_OPENCODE_PLUGIN", &plugin),
            ("NGTERM_OPENCODE_EVENTS", &events_file),
            ("NGTERM_OPENCODE_STARTED", &started),
            ("NGTERM_OPENCODE_RELEASE", &release),
            ("NGTERM_OPENCODE_STATUSES", &statuses),
        ],
    )?;
    assert_eq!(lines(&statuses), ["blocked", "active --blink"]);
    Ok(())
}
