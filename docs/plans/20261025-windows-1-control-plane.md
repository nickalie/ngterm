# Windows 1: the control plane, ngtermctl, ngterm-app and the suite on Windows

## Overview

- **Goal** (user, 2026-10-25): a fully featured Windows version of ngterm. It comes as four plans, run in this order:
  1. this one: the control server, `ngtermctl` and `ngterm-app` on Windows (Windows as a remote origin included), the
     app building and opening a window there, and the conformance suite driving it on Windows CI with a ratchet of
     its own;
  2. `20261025-windows-2-gpui-frontend.md`: the GPUI frontend's Windows forms;
  3. `20261025-windows-3-html-webview2.md`: HTML overlays on WebView2;
  4. `20261025-windows-4-msi.md`: the per-user MSI.
- **Done bar for the series** (user, 2026-10-25): the conformance suite drives the Windows build on CI on
  `windows-2025` (x64) and `windows-11-arm` (arm64), with its own ratchet `conformance/rust-passing-windows.txt` over
  the protocol, cli and behaviors layers. That is the Linux bar: the ui layer has no Rust driver on any OS.
- **Targets** (user, 2026-10-25): Windows 11 (and Server 2025, the x64 runner) on x64 and arm64. No Windows 10 promise.
- **Decisions** (user, 2026-10-25):
  - command lines (hooks, keymap custom commands, the gone-guard, the overlay job's program) run through
    `cmd.exe /D /S /C` on Windows, as pane commands already do (`ngterm-term`'s `COMMAND_SHELL`);
  - Windows claims `live-sessions` for its PowerShell live sessions, beside `live-any-shell`;
  - Windows as a remote origin (attached to from another machine over ssh) is in this plan.
- **Out of this series**, so none of it reads as done:
  - the macOS forms of the CLI, the control server and the frontend (open under CLAUDE.md > Platforms);
  - a UIA ui-layer driver (`drivers/base.py` names `uia`; neither it nor AT-SPI is implemented);
  - signing the MSI.
- **Local work** (user, 2026-10-25): a task that needs Windows by hand (a red id to debug, a probe) starts the
  Win11-admin VMware VM, works there and stops it at the end of the task (memory: windows-macos-debug-in-vmware).
  CI on the Windows runners is the gate.

## Context (from discovery and the plan review)

- `cross-platform.yml` already builds and tests on all three OSes, from one `PACKAGES` list: `ngterm-session` (its
  named pipe, `endpoint/windows.rs`; detach; ConPTY handoff), `ngterm-proto`, `ngterm-host`, `ngterm-core`, the
  libghostty-vt crates, `ngterm-pty`, `ngterm-term`.
- `ngterm-core` already has, on Windows: `state_lock` (`LockFileEx`: a second instance on one state dir is refused),
  `owner_only` (owner-only files), `paths::unavailable_socket_path` (`<socket>.unavailable`).
- `ngterm-server` (`src/lib.rs`, `connection.rs`, `stream.rs`): AF_UNIX listener; `flock` on the socket separates
  instances on different state dirs that share one socket path; `UnixStream` in `StreamOwner::adopt` (the frontend
  goes through `LineStreamOwner`; `ngterm-ctl/tests/cli.rs` adopts directly).
- `ngterm-ctl`, Unix-only in: `client.rs`, `socket.rs` (`/proc/locks`), `stream_bridge.rs` (`poll`),
  `session_overlay.rs` and `live.rs` (the bridge over `STDIN_FILENO`), `overlay_job.rs` (signals, process group,
  `SHELL = "/bin/sh"`), `local.rs` (SIGPIPE, `SYS_close_range`), `clipboard.rs` (`access(X_OK)`), `session_wait.rs`
  and `events.rs` (errno, `localtime_r`), `usage.rs` (`TIOCGWINSZ`), `version.rs` (`/proc/self/exe`), `main.rs`
  (SIGPIPE), `terminfo.rs` (local `infocmp`, remote `tic` over ssh); `tests/cli.rs` (~4700 lines: `UnixStream`,
  `mknod`, `ExitStatusExt`).
- `ngterm-app`: `hook_runner.rs` (detached `/bin/sh -c` children: stdout null, stderr an unlinked owner-only temp file,
  a stdin writer stopped through a `poll` on a stop pipe, a process group of their own; descendants outlive the app by
  design), `logging.rs` (`O_NOFOLLOW`), `remote.rs` (fd, `ExitStatusExt`, `kill`), `integrations/` (file modes,
  symlinks; `opencode.rs` falls back to `/bin/sh`; `cli.rs` links `ngtermctl` into `~/.local/bin`).
- `/bin/sh -c` command lines in `ngterm-core`: `commands/hooks.rs:58`, `commands/keymap.rs:305`,
  `commands/live/lead.rs:231` (the gone-guard); the Reset Live Sessions relauncher in `ngterm-gpui/src/live_runtime.rs`.
- `resources/agent-status/` is upstream's tree plus `build/patches/agent-status/`
  (`.claude/skills/agterm-upstream-sync/scripts/sync-resources.sh`, `--check`).
- `ngterm-core/src/paths.rs`: the XDG state dir, `home_dir` (`HOME`, passwd), `config_dir`, `hud_body_dir`
  (`XDG_RUNTIME_DIR`), `global_ghostty_config_path`, `NGTERM_STATE_DIR`, `NGTERM_CONTROL_SOCKET`, `ngterm.sock`.
- `ngterm-gpui`: `ashpd`, `zbus`, `wayland-*`, `x11rb`, `signal-hook` unconditional; Linux-only code also in
  `activation.rs`, `appearance.rs`, `status_glyph.rs`, `external_texture.rs`, `terminal/surface.rs`, `live_runtime.rs`;
  `build.rs` runs `bash` from `PATH` for `fetch-fonts.sh`; `app.rs` `CAPABILITIES` is one constant for every OS.
- Harness: `impls/base.py` (private weston and bus, SIGTERM quit, process enumeration over `/proc` and `ps`, the
  leftover check by `NGTERM_LIVE_DIR`), `impls/rust.py`, `client.py` (AF_UNIX; `check_under_root` requires the
  socket under the instance root), `zmx_client.py` (`fcntl`/`termios` at import) used by `live_client.py`,
  `tests/cli/test_cli_only.py` and `test_socket_resolution.py` (AF_UNIX stand-in servers, a `#!/bin/sh` fake ssh),
  `behaviors/test_hooks.py` (POSIX hook lines), `tools/run-linux.sh`, `tools/check_ratchet.py` (`--list PATH`).
- Staging: `scripts/stage.sh`; dev launch: `scripts/run-gpui-dev.sh`.

## Development Approach

- **testing approach**: Regular (code first, then its tests in the same task).
- complete each task fully before moving to the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests** for its code changes, success and error cases; a ported
  test gets its Windows form beside the Unix one.
- **CRITICAL: all tests must pass before starting the next task.** Windows tests pass on the Windows runners (push
  to a `ci/**` branch; `rust-windows.yml` exists from Task 4 on), not only through `cargo check --target`.
- **CRITICAL: update this plan file when scope changes during implementation.**
- **Local Windows work in the VM** (user, 2026-10-25): a task that needs Windows by hand (a red test or id to
  debug, a probe, an interactive check) starts the Win11-admin VMware VM, works there and stops it at the end of
  the task (memory: windows-macos-debug-in-vmware). The VM is off by default and never left running between tasks;
  CI on the Windows runners stays the gate.
- Linux behavior stays as it is: every Linux test and ratchet id stays green.
- Scripts stay bash (Git Bash on Windows, as CI and `build-ghostty-vt.sh` already run), branching on the OS where
  needed; no PowerShell twins of bash scripts.

## Testing Strategy

- **unit tests**: per task, on Linux and on both Windows runners.
- **conformance suite**: the protocol, cli and behaviors layers against the Windows build, once at the end
  (Task 22), on `conformance-windows`. The Linux suite (`run-linux.sh`) also runs once at the end, since shared crates
  change.
- No reruns: a flaky Windows test is a bug in the build or the test (CLAUDE.md > Gates).

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- keep the plan in sync with the work done

## Solution Overview

- **The control address stays a path on every OS.** `<state dir>\ngterm.sock` is what `--socket`,
  `NGTERM_CONTROL_SOCKET`, `NGTERM_SOCKET`, the error strings and the `.unavailable` advertisement name, as on Linux,
  so the harness's isolation check and the socket-resolution tests hold unchanged. On Windows the transport maps the
  canonical path to a pipe, `\\.\pipe\ngterm-<hash of the path>`, the way `ngterm-session` names its pipe after the
  namespace directory.
- **One transport seam, two forms.** `ngterm-server` gets `ControlListener`/`ControlStream`: on Unix the AF_UNIX socket
  with `flock`, as today; on Windows a named pipe through `interprocess`, owner-only (the user's SID in its DACL),
  closed to remote clients, owned by its first instance (`FILE_FLAG_FIRST_PIPE_INSTANCE` plays `flock`'s part between
  instances on different state dirs sharing a socket path; one state dir is already `state_lock`'s). The pipe's DACL
  and server-owner check move from `ngterm-session/src/endpoint/windows.rs` into `ngterm-core` beside `owner_only`, so
  the session server, the control server and `ngtermctl` share one copy.
- **Command lines through `cmd.exe /D /S /C`** on Windows (user's decision): one helper in `ngterm-host` builds the
  `LaunchSpec` for a command line per OS, used by hooks, custom commands, the gone-guard and the overlay job; the
  hook runner starts it detached as on Unix. The relauncher gets a Windows form.
- **Paths**: the state dir `%LOCALAPPDATA%\ngterm`, the config dir `%APPDATA%\ngterm`, home `FOLDERID_Profile`, all
  from the Known Folders API (`SHGetKnownFolderPath`), `NGTERM_STATE_DIR` still first.
- **The app builds and runs on Windows** on the stock GPUI Windows backend. The Linux-only frontend modules are behind
  `cfg(target_os = "linux")` with a Windows form that logs the feature as not available yet; plan 2 replaces each one.
  The quit contract is in this plan, since the harness depends on it.
- **The quit contract on Windows**: SIGTERM's counterpart for a GUI process is the session-end protocol: the app takes
  `WM_QUERYENDSESSION`/`WM_ENDSESSION` with `ENDSESSION_CLOSEAPP` (what the Restart Manager sends, and plan 4's MSI
  relies on) on a hidden top-level window of its own and runs the same quit-and-save path. The harness quits an
  instance through the Restart Manager (`RmStartSession`, `RmRegisterResources` by pid, `RmShutdown`), within the 10 s
  `rust-build.md` gives SIGTERM.
- **Windows as a remote origin**: the remote CLI chains a viewer runs over ssh get a form that runs under Windows'
  OpenSSH server's default shell, and the overlay job runner, `live present` and `terminfo install` work against it.
- **The suite on Windows**: the runner's interactive desktop is the display (no compositor, no bus), GPUI renders on
  the runner's only adapter, WARP; every instance gets a private state dir, app id and so a private pipe and live
  namespace. The harness's Ghostty config points `command` at Git for Windows's `sh.exe`, so `posix-shell` holds where
  its tests pass. The capabilities become per OS; Windows claims `live-sessions` and `live-any-shell` for PowerShell.
- **References to study in their tasks** (not while writing this plan): `ngterm-session/src/endpoint/windows.rs`,
  `interprocess`'s local sockets, Windows Terminal (MIT) for session-end and the Restart Manager, herdr's Windows server
  (`.upstream/herdr`), pywinpty and psutil for the harness.

## Technical Details

- **`ControlStream`** offers what the connection thread and the stream owners use: `Read`, `Write`, `try_clone`, read
  and write timeouts, `shutdown`. On Windows a timeout is a deadline on overlapped I/O (or a reader thread), as
  `ngterm-session`'s client does.
- **Pipe naming**: `\\.\pipe\ngterm-<hex>`, `<hex>` the hash `ngterm-session` uses, over the canonical socket path.
  The `.unavailable` file sits beside the path, as on Linux.
- **Command lines**: `cmd.exe` from `%ComSpec%` (else `System32\cmd.exe`), args `/D /S /C "<line>"`, quoted the way
  `ngterm-term/src/launch.rs` `windows_command` quotes; on Unix `/bin/sh -c <line>` as today.
- **Exit statuses**: a Windows exit code is reported as is; Unix's signal statuses keep their Unix form under
  `cfg(unix)`.
- **Ratchet**: `check_ratchet.py --list conformance/rust-passing-windows.txt` (the option exists); Linux's file stays
  the default.
- **Harness dependencies**: `pywinpty` (ConPTY, in place of `zmx_client.py`'s `PtyProcess` on Windows) and `psutil`
  (cmdline and environment of other processes), Windows-only extras in `conformance/pyproject.toml`.

## What Goes Where

- **Implementation Steps**: code, tests, scripts, CI and docs in this repo.
- **Post-Completion**: manual checks in the Win11 VM.

## Implementation Steps

### Task 1: Windows paths

**Files:**
- Modify: `crates/ngterm-core/src/paths.rs`
- Modify: `crates/ngterm-core/Cargo.toml`

- [x] the Windows bases through `Env` (a `known_folder` read beside `passwd_home`, so tests stay on `MapEnv`): state
      `FOLDERID_LocalAppData\ngterm`, config `FOLDERID_RoamingAppData\ngterm`, `home_dir` `FOLDERID_Profile`
      (a non-empty `HOME` still first, as the override the harness isolates an instance's home with)
- [x] `hud_body_dir` on Windows (a private dir under the state dir's runtime part, owner-only through `owner_only`);
      `global_ghostty_config_path` where Ghostty at the pin keeps it on Windows; the fallback when no known folder
      resolves (`%TEMP%\ngterm`)
- [x] write tests for each resolution on Windows (override, known folder, fallback, an empty override as unset)
- [x] run tests on Linux and the Windows runner (`cross-platform.yml`) - must pass before next task
- ⚠️ `commands::tests::remote_roles`' three file-page tests were red on main's Windows leg already (since the
  v0.38.1 sync): a file page's path must start with `/` (`html::grant_error`). They are ignored on Windows; Windows'
  file-page paths go with the HTML overlays' Windows forms (plan 3)
- [x] ➕ `ngterm-session`'s `an_interrupted_powershell_replay_leaves_the_integrated_prompt` was red on the Windows
  runner too: a spawner in a process group of its own (the runner's steps, the session server's detached start)
  ignores Ctrl+C and its pane children inherited that. `ngterm-pty`'s Windows spawn now lets its children take Ctrl+C
  (`SetConsoleCtrlHandler(NULL, FALSE)`), with a test of its own

### Task 2: The control server's transport seam

**Files:**
- Create: `crates/ngterm-server/src/transport.rs`, `crates/ngterm-server/src/transport/unix.rs`
- Modify: `crates/ngterm-server/src/lib.rs`, `connection.rs`, `stream.rs`
- Modify: `crates/ngterm-ctl/tests/cli.rs` (its direct `StreamOwner` adopter)

- [ ] `ControlListener` and `ControlStream` with the Unix form over today's AF_UNIX socket and `flock`
- [ ] the connection thread, `LineStream` and `StreamOwner::adopt` over `ControlStream`; `LineStreamOwner`'s users in
      the frontend unchanged
- [ ] `crates/ngterm-server/tests/{server,stream}.rs` green unchanged on Linux; `cargo check --target` for macOS
- [ ] write tests for the seam's timeouts and `shutdown` through the Unix form
- [ ] run tests - must pass before next task

### Task 3: The shared pipe primitives and the control server on a named pipe

**Files:**
- Create: `crates/ngterm-core/src/owner_pipe.rs` (the DACL and server-owner check, moved)
- Modify: `crates/ngterm-session/src/endpoint/windows.rs` (uses the moved code)
- Create: `crates/ngterm-server/src/transport/windows.rs`
- Modify: `crates/ngterm-server/Cargo.toml`, `crates/ngterm-server/tests/{server,stream}.rs`

- [ ] move the owner-only DACL, the remote refusal and the server-owner check into `ngterm-core`; `ngterm-session`'s
      tests stay green
- [ ] the listener: the path mapped to the pipe name, `FILE_FLAG_FIRST_PIPE_INSTANCE` ownership, one instance per
      waiting connection, one thread per connection
- [ ] a pipe already owned by another instance: the `Refused` path and the `.unavailable` advertisement, as Linux's
      `flock` refusal
- [ ] the tests' Windows forms: request/response, the stream handoff, two instances on one socket path, the DACL
      read back and asserted
- [ ] run tests on Linux and the Windows runner - must pass before next task

### Task 4: Windows CI for the crates leaving Linux

**Files:**
- Modify: `.github/workflows/cross-platform.yml`
- Create: `.github/workflows/rust-windows.yml`

- [ ] `cross-platform.yml`: a per-OS extra package list in the matrix; the Windows leg gets `ngterm-server` now (and
      `ngterm-ctl`, `ngterm-app` as their tasks land); the macOS leg does not; the header comment says so
- [ ] `rust-windows.yml` on `windows-2025` and `windows-11-arm`: format, clippy `-D warnings` on the workspace without
      the WPE crates (`WORKSPACE_WITHOUT_WPE`) and on the frontend without its default features, the lint report, the
      workspace tests; caches keyed by OS and arch (the first native arm64 libghostty-vt prefix is built here)
- [ ] until Task 12 makes the frontend build, `rust-windows.yml` runs the crates that build; each later task widens it
- [ ] measure both runners once with cold caches and set the timeouts from it (recorded in the workflow comments)
- [ ] run on a `ci/**` push - must pass before next task

### Task 5: Command lines through cmd.exe, and the hook runner on Windows

**Files:**
- Modify: `crates/ngterm-host/src/` (the command-line `LaunchSpec` helper)
- Modify: `crates/ngterm-core/src/commands/hooks.rs`, `commands/keymap.rs`, `commands/live/lead.rs`
- Modify: `crates/ngterm-app/src/hook_runner.rs`, `crates/ngterm-app/src/integrations/opencode.rs`
- Modify: `crates/ngterm-app/Cargo.toml`

- [ ] the helper: `/bin/sh -c` on Unix, `cmd.exe /D /S /C` on Windows (Technical Details); hooks, custom commands and
      the gone-guard (`echo ngterm: session is gone & exit /b 1` on Windows) through it; `opencode.rs`'s shell fallback
      per OS
- [ ] the hook runner's Windows form, keeping the Unix design: a detached child (`CREATE_NEW_PROCESS_GROUP`,
      `DETACHED_PROCESS`), stdout null, stderr a temp file opened owner-only with `FILE_FLAG_DELETE_ON_CLOSE`, the stdin
      writer stopped through `CancelSynchronousIo` or by closing its handle; no job object, so descendants outlive the
      app as on Unix
- [ ] write tests for the helper (quoting of `"`, `%`, `^`, `&`), the gone-guard's output, and the runner (exit
      codes, stderr tail captured, a child that never reads stdin, a descendant outliving the run)
- [ ] add `ngterm-app` to the Windows leg's extra packages once it builds there (after Tasks 6-8)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 6: ngterm-app's log and remote presentation process on Windows

**Files:**
- Modify: `crates/ngterm-app/src/logging.rs`, `crates/ngterm-app/src/remote.rs`

- [ ] the log opened with `FILE_FLAG_OPEN_REPARSE_POINT` and refused when it is a reparse point (the `O_NOFOLLOW`
      rule), its rotation under the instance's lock as on Linux
- [ ] the remote presentation process: `ssh.exe` (Windows' OpenSSH client) in a job object (it is the app's own
      child, ended with the link), ended with `TerminateJobObject`, its exit code read as is
- [ ] write tests for the reparse-point refusal and for the presentation process's end and close reason
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 7: The agent integrations' Windows study and payloads

**Files:**
- Create: `build/patches/agent-status/0004-windows.patch` (the Windows payloads, in the patch series)
- Modify: `crates/ngterm-app/src/integrations/mod.rs` (the module docs record the study)

- [ ] study how each integrated agent (Claude Code, Codex, OpenCode, pi) runs hooks on native Windows and where it
      keeps its config there
- [ ] the payloads' Windows form follows it (the `.sh` hooks under Git Bash where the agent runs hooks through bash,
      a PowerShell twin where it runs them through PowerShell), each calling the staged `ngtermctl.exe`; added as a
      patch in `build/patches/agent-status/`, so `sync-resources.sh --check` still reports `unchanged`
- [ ] write tests that each payload runs under the shell its agent uses on Windows and posts its status
- [ ] run the ported OpenCode status tests (`crates/ngterm-app/tests/opencode_status.rs`) on both Windows runners
      (node there) - must pass before next task

### Task 8: The agent integrations' installers on Windows

**Files:**
- Modify: `crates/ngterm-app/src/integrations/{files,hooks,hooks_config,skill,cli,opencode}.rs`

- [ ] the installers copy where Linux links, owner-only through `ngterm_core::owner_only` where Linux sets modes; a
      reinstall over a copy behaves as over a link; a copied payload refreshed when the running build is newer than
      the copy (plan 4's upgrades)
- [ ] "Install Command Line Tool…" on Windows: the running build's `bin` appended to the user's `PATH`
      (`HKCU\Environment`, `WM_SETTINGCHANGE` broadcast), an entry already there left alone (plan 4's MSI does the same)
- [ ] write tests for each installer on Windows (fresh, reinstall, a foreign file left alone, a stale copy refreshed)
      and for the `PATH` edit (fresh, present, another ngterm's `bin` there)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 9: ngtermctl's client, socket resolution and stream bridge on Windows

**Files:**
- Modify: `crates/ngterm-ctl/src/{client,socket,session_wait,events,stream_bridge,live,session_overlay,usage,version,main}.rs`

- [ ] the client over the pipe the path maps to; upstream's resolution order; `/proc/locks` replaced on Windows by
      the pipe's existence and its server's owner (`ngterm_core::owner_pipe`)
- [ ] `session wait`'s and `events`' errors from Windows errors (`ERROR_FILE_NOT_FOUND`, `ERROR_PIPE_BUSY`) mapped to
      upstream's strings; local time without `localtime_r`; the terminal size from the console
      (`GetConsoleScreenBufferInfo`); the executable from `current_exe`; no SIGPIPE handling on Windows
- [ ] the stream bridge on Windows: a reader thread per direction, the console's stdin in raw VT mode for
      `live attach` and the overlay session bridge, restored on exit
- [ ] write tests for resolution, the error mapping and the bridge over a pipe pair (both directions, either end
      closing)
- [ ] run the crate's unit tests on Linux and both Windows runners - must pass before next task

### Task 10: ngtermctl's local subcommands on Windows

**Files:**
- Modify: `crates/ngterm-ctl/src/{local,clipboard,terminfo}.rs`, `crates/ngterm-ctl/Cargo.toml`

- [ ] `local.rs`: no SIGPIPE and no `close_range` on Windows (handles are not inherited unless asked);
      `ERROR_NO_DATA`/`ERROR_BROKEN_PIPE` read as the child closing its stdin
- [ ] `clipboard set`'s program lookup over `PATH` and `PATHEXT`
- [ ] `terminfo install` on Windows: the entry from the staged `share/terminfo/ghostty.terminfo` (no local `infocmp`),
      compiled by `tic` on the remote over `ssh.exe`, as on Linux
- [ ] write tests for each (a program found through `PATHEXT`, the terminfo source read from the stage)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 11: ngtermctl's CLI tests on Windows

**Files:**
- Modify: `crates/ngterm-ctl/tests/cli.rs`, `crates/ngterm-ctl/tests/cli_local.rs`

- [ ] the server fake over the transport seam; `mknod`, `ExitStatusExt` and AF_UNIX-only cases under `cfg(unix)`,
      each with its Windows counterpart where the behavior exists there
- [ ] add `ngterm-ctl` to the Windows leg's extra packages; the crate map's "macOS open" stays
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 12: ngterm-gpui builds and opens a window on Windows

**Files:**
- Modify: `crates/ngterm-gpui/Cargo.toml`, `crates/ngterm-gpui/build.rs`
- Modify: `crates/ngterm-gpui/src/{main,lib,instance,activation,appearance,status_glyph,notifications,global_hotkey,remote,monitors,folder_chooser,html_overlay,external_texture,live_runtime}.rs`,
  `src/quick/host.rs`, `src/terminal/surface.rs`

- [ ] `ashpd`, `zbus`, `wayland-*`, `x11rb` (also the dev-dependency), `signal-hook` under `cfg(target_os = "linux")`
      (or `cfg(unix)` where macOS needs them too)
- [ ] `build.rs` finds Git for Windows's `bash.exe` as the sys crate's build script does (never `bash` from `PATH`)
      and passes paths through `cygpath -m`
- [ ] each Linux-only module behind its `cfg` with a Windows form that logs once that the feature is not available on
      Windows yet and returns what the core expects of an absent feature, naming the plan-2 task replacing it; the
      existing fallbacks (`remote.rs`, `monitors.rs`, `quick/host.rs`) listed in plan 2 > Task 16
- [ ] `#![windows_subsystem = "windows"]` for release builds; the console-less process logs to the log file
- [ ] the app's own unit tests compile and pass on Windows (process tests needing Xvfb or weston stay
      `cfg(target_os = "linux")` until plan 2 gives them a Windows form); `rust-windows.yml` covers the frontend
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 13: The quit contract and the relauncher on Windows

**Files:**
- Create: `crates/ngterm-gpui/src/signals/windows.rs`
- Modify: `crates/ngterm-gpui/src/signals.rs`, `crates/ngterm-gpui/src/live_runtime.rs`
- Modify: `conformance/docs/rust-build.md`

- [ ] study Windows Terminal's session-end handling; a hidden top-level window of the app's own receives
      `WM_QUERYENDSESSION`/`WM_ENDSESSION` even with no terminal window open
- [ ] `ENDSESSION_CLOSEAPP` and a logoff run the same quit-and-save path as SIGTERM, without a prompt; a console
      control event (a dev run from a console) does too
- [ ] Reset Live Sessions' relauncher on Windows: a detached waiter (`ngterm.exe` with a relaunch-after-pid argument)
      that starts the app again with the same state dir and app id once the old pid is gone
- [ ] `rust-build.md` gives the quit contract its Windows form (the Restart Manager's shutdown within 10 s)
- [ ] write tests: a running instance shut down through the Restart Manager writes its quit save; the relauncher
      starts the app on the same state dir after the old one quit
- [ ] run tests on both Windows runners - must pass before next task

### Task 14: Windows as a remote origin

**Files:**
- Modify: `crates/ngterm-core/src/remote/` (the remote CLI chains, `remote::tree::REMOTE_CLI`)
- Modify: `crates/ngterm-ctl/src/overlay_job.rs`
- Modify: `crates/ngterm-app/src/remote.rs` (only if the chain's start needs it)

- [ ] study what a viewer runs on the origin over ssh (the chains, `live present`, `session overlay run-job`,
      `terminfo install`) and what Windows' OpenSSH server runs them under (its `DefaultShell`, `cmd.exe` unless set)
- [ ] the chains get a form that runs under `cmd.exe` and under PowerShell, picked by what the origin reports
      (the probe the viewer already makes, extended), POSIX `sh` unchanged for Unix origins
- [ ] the overlay job runner on Windows: the program through the command-line helper (Task 5) under the console the
      ssh session gives it, cancel through `CTRL_BREAK_EVENT` to its process group then `TerminateProcess` on the
      tree's job, its exit code reported as `exited`
- [ ] write tests for the chain per origin shell and the job runner (started, exited, canceled, launch-failed)
- [ ] write an end-to-end test against a local `sshd` on the Windows runner (Windows' OpenSSH server feature, a test
      key): a Linux-form viewer chain reaches a Windows origin's `live present`
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 15: The staged layout and the dev launch on Windows

**Files:**
- Modify: `scripts/stage.sh`, `scripts/lib/native.sh`, `scripts/run-gpui-dev.sh`

- [ ] `stage.sh` on Windows: `bin\ngterm.exe`, `ngtermctl.exe`, `ngterm-session.exe`, `share\…` as on Linux, the
      PowerShell integration and the Windows payloads; `app-BUILD` and `session-BUILD` written the same way
- [ ] `run-gpui-dev.sh` on Windows: the same isolation (scrubbed `NGTERM_*`/`AGTERM_*`/`ZMX_*`, a private state dir and
      app id), the `--socket` path printed for `ngtermctl`
- [ ] a check in `rust-windows.yml` that the stage holds every file the suite and the dev script read
- [ ] run the stage on Linux (layout unchanged) and on both Windows runners - must pass before next task

### Task 16: The harness's Windows instance: pipe client, isolation and quit

**Files:**
- Modify: `conformance/agterm_conformance/impls/base.py`, `impls/rust.py`, `client.py`
- Modify: `conformance/pyproject.toml` (Windows extras)

- [ ] a Windows instance: no compositor or bus, a private state dir and app id under the run's root, the environment
      scrubbed of `NGTERM_*`, `AGTERM_*` and `ZMX_*`
- [ ] the client over the pipe the socket path maps to (the same hash as Task 3, one shared definition the selftests
      check against the Rust one), the same API; `check_under_root` unchanged, since the path is still a path
- [ ] quit through the Restart Manager (Task 13) via `ctypes`; every `ngtermctl` call with an explicit `--socket`
- [ ] write selftests for the pipe-name derivation, the client and the quit
- [ ] run the selftests on Linux and both Windows runners - must pass before next task

### Task 17: The harness's Windows instance: processes and PTYs

**Files:**
- Modify: `conformance/agterm_conformance/impls/base.py` (process enumeration, the leftover check)
- Modify: `conformance/agterm_conformance/zmx_client.py`, `live_client.py`

- [ ] process enumeration through `psutil` on Windows (cmdline, environment), the leftover check by the instance's
      `NGTERM_LIVE_DIR`; live sessions reaped only through the instance's namespace server or the pids its namespace
      names, never by image name
- [ ] the PTY client on Windows through `pywinpty`, imported only there, so `live_client.py` imports on every OS
- [ ] write selftests for the enumeration and the PTY client's Windows form
- [ ] run the selftests on Linux and both Windows runners - must pass before next task

### Task 18: The suite's tests and capabilities on Windows

**Files:**
- Modify: `conformance/tests/cli/test_cli_only.py`, `conformance/tests/cli/test_socket_resolution.py`
- Modify: `conformance/tests/behaviors/test_hooks.py`
- Modify: `crates/ngterm-gpui/src/app.rs` (`CAPABILITIES` per OS), `crates/ngterm-gpui/src/session_host.rs`
  (`claims`)
- Modify: `conformance/docs/rust-build.md` (> Rust target profile)

- [ ] the AF_UNIX stand-in servers get a pipe form; the fake `ssh` a Windows form (a `.cmd` or a small Python
      script); `test_hooks`' hook lines in `cmd` form on Windows, the same assertions
- [ ] the harness's Ghostty config points `command` at Git for Windows's `sh.exe` on Windows
- [ ] `CAPABILITIES` per OS; `claims` gives Windows `live-sessions` and `live-any-shell` for its PowerShell login
      shell; `posix-shell` claimed on Windows only once its tests pass with `sh.exe`; rust-build.md says each
- [ ] write selftests for the per-OS claims (the version result read back)
- [ ] run the selftests and a targeted suite run (`--ratchet-only --also` the touched files) on both Windows runners -
      must pass before next task

### Task 19: The Windows suite run and its ratchet

**Files:**
- Create: `conformance/tools/run-windows.sh` (and `conformance/tools/lib/run-suite.sh` holding what it shares with
  `run-linux.sh`)
- Modify: `conformance/tools/run-linux.sh`
- Create: `conformance/rust-passing-windows.txt`

- [ ] the shared part (release build, stage, layers, `--ratchet-only`, `--also`, the ratchet's status as exit code)
      moved into the lib; `run-linux.sh` behaves as before
- [ ] `run-windows.sh` (Git Bash) over it, `--list conformance/rust-passing-windows.txt`
- [ ] the Windows ratchet file, empty at first, with Linux's header
- [ ] write selftests that `run-linux.sh`'s options still parse and pass through
- [ ] run the selftests - must pass before next task

### Task 20: The conformance-windows workflow

**Files:**
- Create: `.github/workflows/conformance-windows.yml`

- [ ] on both runners: the stage, `run-windows.sh`, `--must-pass live-sessions --must-pass live-any-shell` (as in
      `rust-linux.yml`), the JUnit and node-id reports uploaded as artifacts; on pull requests and on pushes to `main`
      and `ci/**`
- [ ] measure once and set the timeout from it
- [ ] run it on a `ci/**` push (red ids expected; the job's ratchet holds with the empty list) - must pass before next
      task

### Task 21: CLAUDE.md and README for Windows

**Files:**
- Modify: `CLAUDE.md`, `README.md`

- [ ] CLAUDE.md > Never touch the user's running ngterm: the Windows form (scrubbed env, private state dir and app id
      and so a private pipe, explicit `--socket`, live sessions reaped only through their namespace, never
      `taskkill /im`; a run draws only on a CI runner's session or the VM's desktop)
- [ ] CLAUDE.md > Gates and > Porting a command group, step 6: the Windows ratchet and its run beside Linux's
- [ ] CLAUDE.md > Crate map: `ngterm-server`, `ngterm-ctl`, `ngterm-app` on "Linux, Windows" (macOS open);
      `ngterm-gpui` "Linux; Windows builds, its forms in plan 2"
- [ ] README: the Windows paths, the pipe behind the socket path, `cmd.exe` for command lines, the Windows origin,
      `run-windows.sh`, the dev launch, the new workflows
- [ ] run the gates - must pass before next task

### Task 22: Verify acceptance criteria

- [ ] push to a `ci/**` branch; wait (in the background) for `conformance-windows` on both arches
- [ ] add its newly green ids with `check_ratchet.py --list conformance/rust-passing-windows.txt --add`
- [ ] the expected set: the Linux ratchet's protocol, cli and behaviors ids, minus those whose tests drive a feature
      still stubbed on Windows (the plan-2 modules, HTML pages, the MSI); each id of the expected set still red is fixed
      or recorded here with ⚠️ and its reason
- [ ] run `conformance/tools/run-linux.sh` once in the background; no Linux ratchet id went red, newly green ids added
- [ ] every gate of README > Test on Linux, and `rust-windows.yml`, green

### Task 23: [Final] Update documentation

- [ ] README and CLAUDE.md match what was built (Task 21 revisited)
- [ ] `cross-platform.yml`'s header comment matches its package lists
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

**Manual verification** (in the Win11 VM, started for it and stopped after):
- `ngtermctl` from a PowerShell prompt against a dev instance: `version`, `session list`, `live attach`
- a logoff with the instance running saves its state
- attaching to the VM's ngterm from the Linux box over ssh

**External system updates**: none.
