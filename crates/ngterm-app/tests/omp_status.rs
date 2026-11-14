//! Runs the installed OMP extension under Node's TypeScript loader, without an agent or a live ngterm instance.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const NODE_FLAGS: [&str; 2] = ["--experimental-strip-types", "--disable-warning=ExperimentalWarning"];

const HARNESS: &str = r"import assert from 'node:assert/strict';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const { default: extension } = await import(pathToFileURL(process.env.EXTENSION).href);

function setup(fail = false) {
  const handlers = new Map();
  const calls = [];
  extension({
    on(event, handler) { handlers.set(event, handler); },
    async exec(program, args, options) {
      calls.push({ program, args, options });
      if (fail) throw new Error('wrapper unavailable');
      return { stdout: '', stderr: '', code: 0, killed: false };
    },
  });
  return {
    calls,
    async emit(type, fields = {}) {
      const handler = handlers.get(type);
      if (handler) await handler({ type, ...fields });
    },
  };
}

for (const platform of ['linux', 'darwin', 'win32']) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
  process.env.NGTERM_SESSION_ID = 'private-test-session';
  const run = setup();
  await run.emit('agent_start');
  await run.emit('agent_end', { willContinue: true });
  await run.emit('agent_end', { willContinue: true, awaitingAsyncWork: true });
  await run.emit('agent_start');
  await run.emit('agent_end', { willContinue: false });
  await run.emit('agent_start');
  await run.emit('agent_end');
  await run.emit('session_shutdown');
  const statuses = [
    ['active', '--blink'], ['active', '--blink'], ['active', '--blink'],
    ['active', '--blink'], ['completed', '--auto-reset'], ['active', '--blink'],
    ['completed', '--auto-reset'], ['idle'],
  ];
  const wrapper = join(homedir(), '.config', 'ngterm', 'agent-status', 'ngterm-agent-status.sh');
  const prefix = platform === 'win32'
    ? ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', wrapper.replace(/\.sh$/, '.ps1')]
    : [];
  assert.deepEqual(run.calls, statuses.map(args => ({
    program: platform === 'win32' ? 'powershell.exe' : wrapper,
    args: [...prefix, ...args], options: { timeout: 1000 },
  })));

  const failing = setup(true);
  await failing.emit('agent_start');
  await failing.emit('agent_end');
  await failing.emit('session_shutdown');
  assert.deepEqual(failing.calls.map(call => call.args.slice(prefix.length)), [
    ['active', '--blink'], ['completed', '--auto-reset'], ['idle'],
  ]);

  delete process.env.NGTERM_SESSION_ID;
  const outside = setup();
  await outside.emit('agent_start');
  await outside.emit('agent_end');
  await outside.emit('session_shutdown');
  assert.deepEqual(outside.calls, []);
}
console.log('OMP lifecycle: active, continuations, background work, completed, shutdown, failures, outside ngterm; POSIX and Windows commands');
";

#[test]
fn omp_status_follows_turns_and_never_interrupts_the_agent() -> Result<(), Box<dyn std::error::Error>> {
    let resolved = Command::new("node")
        .args(NODE_FLAGS)
        .args(["-p", "process.execPath"])
        .output();
    let node = match resolved {
        Ok(output) if output.status.success() => PathBuf::from(String::from_utf8(output.stdout)?.trim()),
        result => {
            assert!(
                std::env::var_os("NGTERM_REQUIRE_NODE_TESTS").is_none(),
                "OMP extension tests need Node.js with TypeScript support: {result:?}"
            );
            eprintln!("SKIPPED: OMP extension tests need Node.js with TypeScript support: {result:?}");
            return Ok(());
        }
    };
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("omp-status.mjs");
    fs::write(&script, HARNESS)?;
    let extension = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/omp/ngterm-status.ts");
    let mut command = Command::new(node);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(NODE_FLAGS)
        .arg(script)
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("EXTENSION", extension)
        .output()?;
    assert!(
        output.status.success(),
        "OMP extension failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    Ok(())
}
