//! Real OMP prompts, queueing, abort and shutdown through isolated ngterm panes.

#![cfg(target_os = "linux")]

#[expect(dead_code, unused_macros, reason = "the sandbox serves multiple process tests")]
mod support;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ngterm_app::integrations::{Environment, hooks, skill};
use serde_json::{Value, json};
use support::{Sandbox, TestResult, have, lavapipe_icd, live};

const REQUIRE: &str = "NGTERM_REQUIRE_OMP_E2E";
const RPC_LIMIT: Duration = Duration::from_secs(240);
const STATUS_LIMIT: Duration = Duration::from_secs(30);

fn missing(name: &str) -> TestResult {
    if std::env::var_os(REQUIRE).is_some() {
        return Err(format!("required OMP E2E prerequisite is unavailable: {name}").into());
    }
    eprintln!("SKIPPED: OMP E2E needs {name}");
    Ok(())
}

fn install_resources(sandbox: &Sandbox, ctl: &Path) -> TestResult {
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let home = sandbox.path("home");
    let environment = Environment {
        home: home.clone(),
        agent_status: Some(resources.join("agent-status")),
        skill: Some(resources.join("skills/ngterm")),
        bundled_cli: Some(ctl.to_owned()),
        vars: HashMap::new(),
    };
    let plan = hooks::plan(&environment, &hooks::opencode_start(&home, None), None).map_err(|error| error.0)?;
    assert_eq!(
        hooks::apply(&plan).map_err(|error| error.0)?.omp,
        hooks::OmpResult::Installed
    );
    let report = skill::apply(&skill::plan(&environment).map_err(|error| error.0)?);
    assert!(!report.is_warning(), "{report:?}");
    Ok(())
}

fn rpc_line(writer: &mut File, command: &Value) -> TestResult {
    writeln!(writer, "{command}")?;
    writer.flush()?;
    Ok(())
}

fn ctl(sandbox: &Sandbox, executable: &Path, args: &[&str]) -> TestResult<String> {
    let output = sandbox
        .command(executable)
        .args(args)
        .arg("--socket")
        .arg(sandbox.path("state/ngterm.sock"))
        .output()?;
    if !output.status.success() {
        return Err(format!("ngtermctl {args:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn session(sandbox: &Sandbox, id: &str) -> TestResult<Value> {
    let tree = live::call(sandbox, "tree", None, json!({}))?;
    live::find(&tree, id)
        .cloned()
        .ok_or_else(|| format!("session {id} missing: {tree}").into())
}

fn wait_status(sandbox: &Sandbox, id: &str, status: Option<&str>) -> TestResult<Value> {
    let deadline = Instant::now() + STATUS_LIMIT;
    loop {
        let current = session(sandbox, id)?;
        if current["status"].as_str() == status {
            return Ok(current);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "expected status {status:?}, got {current}\nOMP:\n{}\nngterm:\n{}",
                sandbox.log("omp-rpc.log"),
                sandbox.log("ngterm.log")
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn wait_rpc(sandbox: &Sandbox, id: &str, kind: &str) -> TestResult<Value> {
    let deadline = Instant::now() + RPC_LIMIT;
    loop {
        let log = sandbox.log("omp-rpc.log");
        for line in log.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && value["id"] == id
                && value["type"] == kind
            {
                return Ok(value);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "OMP never returned {kind} for {id}:\n{log}\n{}",
                sandbox.log("ngterm.log")
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn start_proxy(sandbox: &mut Sandbox) -> TestResult<String> {
    let script = sandbox.path("inference-proxy.py");
    fs::write(&script, include_str!("omp/inference_proxy.py"))?;
    let upstream = std::env::var("NGTERM_OMP_E2E_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:19451/v1".to_owned());
    let mut command = sandbox.command("python3");
    command.arg(script).arg(sandbox.root.path()).arg(upstream);
    sandbox.spawn(command, "inference-proxy.log")?;
    support::wait_for(|| fs::read_to_string(sandbox.path("proxy-url")).ok())
}

fn wait_inference(sandbox: &Sandbox, sequence: u8) -> TestResult {
    let deadline = Instant::now() + RPC_LIMIT;
    let ready = sandbox.path(&format!("ready-{sequence}"));
    while !ready.exists() {
        if Instant::now() >= deadline {
            return Err(format!(
                "inference {sequence} never reached its gate:\n{}",
                sandbox.log("inference-proxy.log")
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

fn release_inference(sandbox: &Sandbox, sequence: u8) -> TestResult {
    fs::write(sandbox.path(&format!("release-{sequence}")), "")?;
    Ok(())
}

fn configure_omp(
    sandbox: &Sandbox,
    ctl: &Path,
    omp: &Path,
    pane: &str,
    base_url: &str,
) -> TestResult<(PathBuf, PathBuf)> {
    let agent = sandbox.path("home/.omp/agent");
    fs::create_dir_all(&agent)?;
    fs::create_dir_all(sandbox.path("home/work"))?;
    fs::create_dir_all(sandbox.path("state/windows"))?;
    fs::write(agent.join("config.yml"), "setupVersion: 1\n")?;
    install_resources(sandbox, ctl)?;
    let model = std::env::var("NGTERM_OMP_E2E_MODEL").unwrap_or_else(|_| "qwen3:0.6b".to_owned());
    let config = json!({
        "providers": {
            "ngterm-e2e": {
                "baseUrl": base_url, "apiKey": "ollama", "api": "openai-completions",
                "models": [{
                    "id": model, "name": "ngterm E2E", "reasoning": false, "input": ["text"],
                    "contextWindow": 32768, "maxTokens": 64,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                    "compat": {"extraBody": {"reasoning_effort": "none"}}
                }]
            }
        }
    });
    fs::write(agent.join("models.yml"), serde_json::to_vec(&config)?)?;
    let fifo = sandbox.path("omp-rpc.in");
    let output = sandbox.command("mkfifo").arg(&fifo).output()?;
    assert!(
        output.status.success(),
        "mkfifo: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let runner = sandbox.path("run-omp.sh");
    let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    fs::write(
        &runner,
        format!(
            "{} --mode rpc --no-session --no-tools --no-lsp --no-rules --no-context-files --no-title \
             --thinking off --system-prompt 'Follow the user instruction exactly.' --model {} < {} 2>&1 | tee {}\n",
            quote(&omp.to_string_lossy()),
            quote(&format!("ngterm-e2e/{model}")),
            quote(&fifo.to_string_lossy()),
            quote(&sandbox.path("omp-rpc.log").to_string_lossy())
        ),
    )?;
    if pane == "right" {
        let entry = sandbox.path("pane-shell.sh");
        fs::write(
            &entry,
            format!(
                "if [ \"$NGTERM_PANE\" = right ]; then exec /bin/sh {}; else exec /bin/sh; fi\n",
                quote(&runner.to_string_lossy())
            ),
        )?;
        fs::create_dir_all(sandbox.path("state/config"))?;
        fs::write(
            sandbox.path("state/config/ghostty.conf"),
            format!("command = /bin/sh {}\n", entry.display()),
        )?;
    }
    Ok((runner, fifo))
}

fn launch_omp(
    sandbox: &Sandbox,
    ctl_path: &Path,
    runner: &Path,
    fifo: &Path,
    pane: &str,
) -> TestResult<(String, File)> {
    let work = sandbox.path("home/work").to_string_lossy().into_owned();
    let command = format!("/bin/sh {}", runner.display());
    let id = if pane == "left" {
        ctl(
            sandbox,
            ctl_path,
            &["session", "new", "--wait", "--cwd", &work, "--command", &command],
        )?
    } else {
        let id = ctl(sandbox, ctl_path, &["session", "new", "--cwd", &work])?;
        live::call(sandbox, "session.split", Some(&id), json!({"mode": "on"}))?;
        id
    };
    let deadline = Instant::now() + STATUS_LIMIT;
    loop {
        match OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo) {
            Ok(writer) => return Ok((id, writer)),
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn check_skill(sandbox: &Sandbox, rpc: &mut File) -> TestResult {
    rpc_line(rpc, &json!({"type": "get_available_commands", "id": "commands"}))?;
    let response = wait_rpc(sandbox, "commands", "response")?;
    assert_eq!(response["success"], true, "{response}");
    assert!(
        response["data"]["commands"].as_array().is_some_and(|commands| commands
            .iter()
            .any(|command| command["source"] == "skill" && command["name"] == "skill:ngterm")),
        "OMP did not discover ngterm: {response}"
    );
    Ok(())
}

fn check_queued_turn(sandbox: &Sandbox, rpc: &mut File, id: &str, pane: &str) -> TestResult {
    rpc_line(
        rpc,
        &json!({"type": "prompt", "id": "turn", "message": "Reply with READY. /no_think"}),
    )?;
    wait_inference(sandbox, 1)?;
    let active = wait_status(sandbox, id, Some("active"))?;
    assert_eq!(active["statusBlink"], true, "{active}");
    assert_eq!(active["statusPane"], pane, "{active}");
    rpc_line(
        rpc,
        &json!({"type": "follow_up", "id": "queued", "message": "Now reply with FINISHED. /no_think"}),
    )?;
    assert_eq!(wait_rpc(sandbox, "queued", "response")?["success"], true);
    release_inference(sandbox, 1)?;
    wait_inference(sandbox, 2)?;
    wait_status(sandbox, id, Some("active"))?;
    release_inference(sandbox, 2)?;
    let result = wait_rpc(sandbox, "turn", "prompt_result")?;
    assert_eq!(result["status"], "completed", "{result}");
    wait_status(sandbox, id, Some("completed"))?;
    rpc_line(rpc, &json!({"type": "get_messages", "id": "messages"}))?;
    let messages = wait_rpc(sandbox, "messages", "response")?;
    let completed = messages["data"]["messages"]
        .as_array()
        .ok_or_else(|| format!("no OMP messages: {messages}"))?
        .iter()
        .filter(|message| message["role"] == "assistant" && message["stopReason"] == "stop")
        .count();
    assert!(completed >= 2, "the queued follow-up did not finish: {messages}");
    Ok(())
}

fn check_abort_and_recovery(sandbox: &Sandbox, rpc: &mut File, id: &str) -> TestResult {
    rpc_line(
        rpc,
        &json!({"type": "prompt", "id": "abort", "message": "Reply with ABORTED. /no_think"}),
    )?;
    wait_inference(sandbox, 3)?;
    wait_status(sandbox, id, Some("active"))?;
    rpc_line(rpc, &json!({"type": "abort"}))?;
    let aborted = wait_rpc(sandbox, "abort", "prompt_result")?;
    assert_eq!(
        aborted["status"],
        "aborted",
        "{aborted}\n{}",
        sandbox.log("omp-rpc.log")
    );
    release_inference(sandbox, 3)?;
    rpc_line(
        rpc,
        &json!({"type": "prompt", "id": "recover", "message": "Reply with RECOVERED. /no_think"}),
    )?;
    wait_inference(sandbox, 4)?;
    wait_status(sandbox, id, Some("active"))?;
    release_inference(sandbox, 4)?;
    let recovered = wait_rpc(sandbox, "recover", "prompt_result")?;
    assert_eq!(recovered["status"], "completed", "{recovered}");
    wait_status(sandbox, id, Some("completed"))?;
    Ok(())
}

fn run_e2e(pane: &str) -> TestResult {
    let Some(omp) = std::env::var_os("NGTERM_OMP_E2E_BIN").map(PathBuf::from) else {
        return missing("NGTERM_OMP_E2E_BIN (absolute path to omp)");
    };
    if !omp.is_absolute() || !omp.is_file() {
        return missing("a valid absolute NGTERM_OMP_E2E_BIN");
    }
    if !have("Xvfb") {
        return missing("Xvfb on PATH");
    }
    if !have("python3") {
        return missing("Python 3 on PATH");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        return missing("lavapipe (scripts/fetch-lavapipe.sh)");
    };
    let ctl_path = Path::new(env!("CARGO_BIN_EXE_ngterm")).with_file_name("ngtermctl");
    if !ctl_path.is_file() {
        return missing("ngtermctl next to the built ngterm executable");
    }
    let xvfb_dir = std::env::split_paths(&std::env::var_os("PATH").ok_or("no PATH")?)
        .find(|directory| directory.join("Xvfb").is_file())
        .ok_or("no Xvfb")?;
    let mut sandbox = Sandbox::new()?;
    sandbox.set_env("PATH", format!("{}:/usr/local/bin:/usr/bin:/bin", xvfb_dir.display()));
    sandbox.start_xvfb()?;
    sandbox.use_software_rendering(&lavapipe);
    let proxy = start_proxy(&mut sandbox)?;
    let (runner, fifo) = configure_omp(&sandbox, &ctl_path, &omp, pane, &proxy)?;
    let primary = sandbox.start_gpui_primary("ngterm.log")?;
    let (id, mut rpc) = launch_omp(&sandbox, &ctl_path, &runner, &fifo, pane)?;
    let sibling = ctl(
        &sandbox,
        &ctl_path,
        &["session", "new", "--no-select", "--command", "sleep 600"],
    )?;
    let sibling_status = session(&sandbox, &sibling)?["status"].clone();
    check_skill(&sandbox, &mut rpc)?;
    check_queued_turn(&sandbox, &mut rpc, &id, pane)?;
    assert_eq!(session(&sandbox, &sibling)?["status"], sibling_status);
    live::call(&sandbox, "session.select", Some(&sibling), json!({}))?;
    live::call(&sandbox, "session.select", Some(&id), json!({}))?;
    wait_status(&sandbox, &id, None)?;
    check_abort_and_recovery(&sandbox, &mut rpc, &id)?;
    drop(rpc);
    wait_status(&sandbox, &id, None)?;
    assert_eq!(session(&sandbox, &sibling)?["status"], sibling_status);
    ctl(&sandbox, &ctl_path, &["session", "close", "--target", &sibling])?;
    sandbox.terminate(primary)?;
    assert!(sandbox.wait_exit(primary, Duration::from_secs(10))?.0.success());
    Ok(())
}

#[test]
fn real_omp_lifecycle_updates_only_its_session() -> TestResult {
    run_e2e("left")
}

#[test]
fn real_omp_split_lifecycle_updates_only_its_pane() -> TestResult {
    run_e2e("right")
}
