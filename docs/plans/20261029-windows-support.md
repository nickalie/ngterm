# Windows support

## Overview

ngterm treats macOS, Linux and Windows as equal targets (CLAUDE.md > Platforms). On Windows today, the terminal core,
the pane core, the PTY layer (ConPTY), the live-session server and `ngterm-core` build and test (`cross-platform.yml`).
`ngterm-core`'s paths use the known folders there, and a pane's children take Ctrl+C. Nothing a user can run is there
yet:

- `ngterm-server` (the control socket) is AF_UNIX and `flock`.
- `ngtermctl` is an AF_UNIX client with Unix job control (`overlay_job.rs`), `close_range` and `/proc/locks`.
- `ngterm-app` uses Unix forms throughout: its hook runner (`poll` over pipes), its log file (`O_NOFOLLOW`), its remote
  presentation process (`fcntl`, `kill`), and its agent installers (file modes, symlinks).
- `ngterm-gpui` links Wayland, X11, zbus, ashpd, netlink and xkbcommon unconditionally. Its single instance, activation,
  notifications, appearance, global hotkey, network and sleep sources and quit signal are Linux forms. Its HTML
  overlays are WPE WebKit, Linux only.
- No installer, no app build on a Windows CI runner, and no conformance run on Windows.

This plan takes every part to Windows: the same features, the same CLI, the same suite. It was written from the code
on `main` at `55a5cd1`, not from earlier Windows plans. Where a mechanism changes, its macOS form comes with it when
that form is the shared Unix one (the server and `ngtermctl` on macOS). The macOS frontend (GPUI's macOS platform,
WKWebView overlays, a bundle) is not in this plan's scope.

## Decisions

- **Control endpoint**: a named pipe through `interprocess` (the crate `ngterm-session` already uses for its
  endpoint), owner-only and closed to remote clients, as the session server's pipe is. The socket *path* stays the
  identity on every OS. `NGTERM_CONTROL_SOCKET`, `--socket`, `app_socket_path` and the unavailable-socket marker keep
  their meaning; on Windows the pipe's name is derived from that path, with the same scheme `ngterm-session` uses for
  its namespace's pipe. A client refuses a pipe served by another user, as `ngterm-session`'s does.
- **HTML overlays on Windows**: WebView2 through `wry` (tauri's, the standard Rust WebView crate), as a child webview
  of the GPUI window placed over the page's bounds. This follows upstream's model, a native `WKWebView` over the
  window. No offscreen frames and no `external_texture` on Windows. `wry`'s macOS form (WKWebView) is the same crate
  later.
- **Notifications**: Windows toasts through the `windows` crate (Microsoft's official bindings, WinRT
  `ToastNotificationManager`), under ngterm's own AppUserModelID. An unpackaged run registers it under HKCU; the MSI's
  Start-menu shortcut carries it. A click on a toast reveals the pane, a withdrawal removes it.
- **Single instance**: a per-app-id, per-state-dir named mutex or pipe with a forward to the primary, which raises its
  window (`AllowSetForegroundWindow` by the forwarding launch). The task picks the exact primitive; the rule is
  "forward before GPUI starts and before any state read", as on Linux.
- **Installer**: an MSI built with the WiX Toolset (the standard Windows installer toolchain), per user or per machine
  as the task settles. It installs `ngterm.exe`, `ngtermctl.exe` on `PATH`, the resources and shell integration, and a
  Start-menu shortcut with the AUMID.
- **Suite on Windows**: the harness gets a named-pipe transport and a Windows launch (private state dir, stripped
  `NGTERM_*`/`AGTERM_*`/`ZMX_*`). A `windows` CI job runs the protocol, cli and behaviors layers there against the
  staged build, holding the ids that pass with `--must-pass`.

## Development Approach

- **Scope by the code.** Each task finds every Unix-only or Linux-only spot in its part (`std::os::unix`, `libc::`,
  `rustix`, D-Bus, Wayland/X11, `/proc`, signals), gives it its Windows form behind `cfg`, and keeps the Linux form
  unchanged. A behavior with no meaning on Windows is named, with the reason, in the module docs.
- **Cross-platform crates first** (CLAUDE.md > Platforms, memory "popular tools only"): `interprocess`, `portable-pty`,
  `wry`, `windows`/`windows-sys`. A new dependency goes into `[workspace.dependencies]` with a comment saying what it
  serves, the way the existing ones do. Linux-only dependencies of `ngterm-gpui` move under
  `[target.'cfg(target_os = "linux")'.dependencies]`.
- **Proven on Windows.** Every task builds, lints and tests its crates on the Windows VM (below). The app tasks also
  launch the app in the VM (a private state dir, its own desktop, never the host's display). `cross-platform.yml`
  gains each crate that becomes cross-platform, as its header says. CI is the gate, run by a push to a `ci/**`
  branch or a PR, which only the orchestrator or the user does, never a task agent.
- **Linux gate**: README > Test's checks on the changed part only (clippy on the touched crates and their dependents,
  tests on the touched crates, a partial `run-linux.sh` over the ids covering the touched behavior). A ratchet id
  never goes red.
- **Identity**: every new name (pipe, mutex, AUMID, registry key, install dir, MSI product) is ngterm's
  (CLAUDE.md > References > Identity), never agterm's.
- **Windows VM workflow**: see "Windows VM" below. The orchestrator starts and stops the VM; a task agent never does.

## Windows VM

The orchestrator's brief for each task gives the access details (ssh, the per-task guest directory, the shared target
dir and the host-side lock). Builds and tests in the guest run one at a time, under the host's lock, because the
guest has 4 CPUs and 6.5 GB of memory.

## Implementation Steps

### Task 1: Control server on Windows and macOS (`ngterm-server`)

- [x] the listener and connections behind an endpoint seam: AF_UNIX on Unix, the named pipe on Windows (Decisions >
  Control endpoint), streams (`stream.rs`, the live attach and job channels) over both
- [x] the binder's lock and stale-socket handling in their Windows form (one server per endpoint; a second start is
  refused with the same error the Unix form gives)
- [x] `ngterm-core`'s socket-path helpers name the Windows pipe where they need to (one place, shared with the client)
- [x] the server's tests pass on Windows and macOS (Unix-specific assertions gated, their Windows twins added)
  (macOS checked with `cargo check --target x86_64-apple-darwin --all-targets` only; its tests run on CI's runner)
- [x] `ngterm-server` joins `cross-platform.yml`'s PACKAGES; its header comment updated
- [x] Windows VM: clippy and tests of `ngterm-server` and `ngterm-core` green; Linux gate green
- ⚠️ ngterm-session's `tests/pwsh.rs` did not run on the Windows VM: the guest has only Windows PowerShell 5.1 and
  the test requires PowerShell 7 there (CI's Windows runners carry it); every other ngterm-session test is green there
- ➕ ngterm-session's detach-behind-a-write client test raced its writing thread on a loaded guest (a fixed 300 ms
  pause); it now detaches once the scripted peer has read the write's first byte
- ➕ the pipe naming scheme and the pipe's descriptor and server-user check moved to `ngterm-core` (`paths::pipe_name`,
  `paths::control_pipe_name`, `pipe_security`), shared by ngterm-session's namespace pipe and the control pipe
- ➕ CLAUDE.md's crate table still lists `ngterm-server` as "Linux, macOS"; it is `any` now (left to the docs pass,
  Task 9)
- ➕ (side task 13) live panes missed a winget/Store PowerShell 7: its only `pwsh.exe` on `PATH` is the zero-byte
  App Execution Alias, which the PE-image check skipped; an `IO_REPARSE_TAG_APPEXECLINK` alias whose package image is
  launchable now counts, and the pane runs the alias itself (`launch/windows_shell.rs`)
- ➕ (side task 19) a forced kill on a loaded Windows sometimes reported "did not end after its hangup": the job's
  tree took about half a second to end and wake the waiter, and the PTY's `EXIT_GRACE`, counted by the waiter from
  the exit, then outran the kill's single budget counted from the signal. A kill now waits for the exit to be seen,
  then for the end the waiter records (`Pane::kill`): a kill that ends the program for certain (forced, or any on
  Windows) waits up to `TERMINATION_LIMIT` for the exit, a plain hangup on Unix `KILL_LIMIT`; the client's kill call
  waits `KILL_WAIT` beyond its timeout to match. Unix had the same flaw for a late exit whose PTY stays open
  (`tests/pane.rs` covers it)
- ➕ (side task 20) ngterm-session's `the_screen_comes_from_the_servers_terminal_…` test failed under load on
  Windows ("the scrollback, then the screen", about 1 round in 90): `cmd /q /k` writes a blank line and its prompt
  after the 60-row command, in a read of its own, and one landing between the test's two reads scrolled the screen
  (the revision moved with it, so the server's rule holds). The typed line now ends in a program that says `ready`
  and waits, and both reads come once exactly that screen shows
- ⚠️ (side task 20) the same test's closing forced kill once reported "did not end after its hangup" in a stress
  round on an overloaded guest (4 busy processes on its 4 CPUs; the round took 25 s against about 4 s): the 5 s
  `TERMINATION_LIMIT` left as it is, pending the user's call
- ➕ (side task 34) ngterm-session's `tests/snapshot.rs` flood tests failed under load on Windows (the watcher waited
  out its 60 s for a chunk's end; a stress loop on the runner failed 2 tests in its first round): cmd titles its
  console with every command it runs, each `echo` of the flood's `for` loop too, and ConPTY passes each title on as
  an OSC, so a chunk was 26.5 KB of output against the tests' 16 KiB client queue, and the watcher, a few reads
  behind, overflowed mid-chunk. A chunk is now a file printed by one `type` (`cat` on Unix), 9 KB, which fits in the
  queue the watcher starts it with empty. The same symptom on the run before c1b510c5 may have been this overflow;
  that commit's wait for cmd's prompt stays, as it keeps the watcher's queue empty at each chunk's start
- ⚠️ (side task 34) seen on Linux, not fixed (outside the task): ngterm-session's `tests/handoff.rs` spawns of a
  staged binary copy fail now and then with `ETXTBSY` (1 round in 8 of the binary: "Text file busy" in
  `a_client_of_another_build_hands_the_server_over_…` and `an_update_hands_a_running_server_…`): `support::stage`
  writes the copy in the test process, and a child another test thread forks meanwhile holds the copy's write
  descriptor until its own exec

### Task 2: `ngtermctl` on Windows and macOS (`ngterm-ctl`)

- [x] the client over the endpoint seam (Task 1), with its connect errors mapped to the same messages on every OS
- [x] `overlay_job.rs`'s Windows form: the job's process tree in a Job object, the grace stop and kill, the console
  Ctrl events in place of signals, the exit status mapping; its macOS form is the Unix one
- [x] the live attach bridge (`stream_bridge`, `live.rs`) on the Windows console: raw input with VT input, VT output,
  resize, restore on exit
- [x] the clipboard helper lookup through `PATHEXT` on Windows; `close_range` and `/proc/locks` uses with their macOS and
  Windows forms
- [x] the CLI's tests against the server pass on all three OSes (Linux and the Windows VM run; macOS is
  clippy-checked locally and tested by CI's macOS runner)
- [x] `ngterm-ctl` joins PACKAGES; README's statement that it is Linux only goes
- [x] Windows VM: clippy and tests green; `ngtermctl --help` and a round trip against a test server run there
- ➕ the Windows gate's first run (2026-10-29 ~21:55) died with the VM (the host's /home filled up); rerun green
  after the VM came back
- ➕ on Windows a Ctrl+Break sent before `cmd.exe` has started the program reaches `cmd.exe` alone, which outlives
  it: such an early cancel ends at the kill past the grace (137); one sent later reads 130 (`STATUS_CONTROL_C_EXIT`
  passes through `cmd /c`, checked in the VM)
- ➕ `cli_local`'s tests that run a fake (`ngterm-session`, `ssh`, `tic`) stay Unix only, their fakes being `sh`
  scripts; the print's run has Windows twins over a `.cmd` fake in `clipboard`'s tests, `terminfo install` has none
  on Windows (it needs `infocmp` and `ssh` there)
- ➕ macOS CI's first ngterm-ctl run failed 4 unit tests on Linux assumptions (`mknod` of a socket, `/bin/true`,
  `/proc/*/status`, `/proc/*/fd`); with those fixed, 2 CLI tests failed the same way (`mknod`, tic's hex directory
  layout) and a stream-bridge test hung: macOS can leave a `recv` blocked in one thread asleep after another thread's
  `shutdown` of the same socket, so the bridge and run-job's listener now wait on a wake pipe too (`read_wake`)
- ➕ (side task 25) `ngtermctl terminfo install` failed on Windows, which has no `infocmp` (two `test_cli_only.py` ids
  red in the suite). The CLI now decompiles the staged compiled entry itself on every OS (`terminfo/source.rs`, a port
  of ncurses 6.6's entry reader and `infocmp -x`'s layout, credited in `NOTICE`), byte for byte what `infocmp -x`
  prints apart from its comment line: its tests compare it with the real `infocmp` on Linux over a `tic`-compiled
  fixture and the system database's entries. A failed dump reads `could not read <file>: <reason>` where upstream's
  read `infocmp exited N: ...`. The install's unit tests run on Windows over a `.cmd` fake `ssh`; the suite's
  Windows stand-in is an `ssh.exe` (distlib's script launcher over Python) running the remote line in Git for
  Windows' `sh` and `tic`, and the killed-by-a-signal half stays POSIX only
- ➕ (side task 25) seen on its CI run: ngterm-app's `opencode_status` order test read the reporter's `started` file
  while the Windows reporter's `Add-Content` still held it (EBUSY); the harness now retries that read

### Task 3: App services on Windows (`ngterm-app`)

- [x] the hook runner's Windows form (no `poll`; pipe readers on threads, the same timeout and output caps)
- [ ] the log file's Windows form (open without following a reparse point, shared for appending, the same rotation
  under the running instance's lock)
- [ ] the remote presentation process's Windows form (its lifetime, stop and reap)
- [ ] the agent integrations' installers without file modes or symlinks on Windows (what each one installs and where,
  per agent, as each agent documents for Windows)
- [ ] `ngterm-app` joins PACKAGES; its tests pass on all three
- [ ] Windows VM: clippy and tests green; Linux gate green
- ➕ (side task 12) the agent-status package's Windows forms: `ngterm-agent-status.ps1`, `ngterm-claude-status.ps1`
  (the process walk through `Win32_Process` where the POSIX one runs `ps`) and `ngterm-agent-restore.ps1`, for
  Windows PowerShell 5.1 and PowerShell 7; on Windows the Claude Code hooks call them through `powershell.exe`, and
  the Pi extension and both `OpenCode` plugins start the PowerShell wrapper (`0004-windows-forms.patch`); the adapter,
  restore and `OpenCode` plugin tests run on Windows too
- ➕ left open: Codex's Windows hooks (`command_windows`) still run the POSIX Codex adapter under Git Bash: it has no
  PowerShell form yet (its blocker watcher and question check are the largest script of the package)
- ➕ (side task 27) hooks and custom commands ran `/bin/sh -c` on Windows too, so none could start there. One place now
  chooses the shell of a user's command line, `ngterm-host`'s `command_line`: `/bin/sh -c` on Unix, `%ComSpec% /D /S /C
  "<line>"` on Windows (`cmd.exe` when `ComSpec` names none; the system's interpreter, as Node's `shell: true` and
  Ghostty's command use it, and fast enough for a hook per event), the quoted line passed as written (`raw_arg`).
  Hooks, custom commands (`LaunchSpec::command_line`), a pane's command (`ngterm-term`'s launch) and the overlay job
  (`ngtermctl`) share it. The starter `hooks.conf` and `keymap.conf` name `cmd.exe` and `%VAR%` on Windows. The hook
  runner was checked end to end on Windows: the event on stdin, the `AGT_*` environment, the stderr tail's cap and the
  exit status (`ngterm-app`'s launcher tests through `cmd`: a hook line reading stdin into files, a quoted program path
  with spaces and a quoted argument, a failing line's status and stderr), and there is no timeout, as upstream has
  none. The hook and custom-command conformance tests build their lines in either dialect
  (`agterm_conformance.command_line`) under a new capability, `cmd-command-line`, which the Windows build claims: 11
  ids joined the Windows ratchet. `test_run_in_a_removed_session_directory_starts_in_home` stays POSIX-only: Windows
  keeps the directory a pane's shell works in from being removed. Their external programs (`more`) needed side task
  22's `PATH` fix, merged in first
- ➕ found and left to their tasks, the other `/bin/sh` lines on Windows: an overlay program's wrapper
  (`overlay_capture::surface_command`, `/usr/bin/env /bin/sh -c '( eval "$NGTERM_OVL_CMD" ); …'`, handed to `cmd`
  there, so the GPUI overlay's program does not start on Windows; its tests stay under `posix-command-line`), the
  Edit Keymap/Hooks editor command (`keymap::editor_command`, `$SHELL -ilc`), the live re-attach's fallback creation
  argv (`commands/live/lead.rs`, a fixed line that should fail anyway), and `OpenCode`'s version probe (a POSIX login
  shell on Unix, its own `cmd.exe` form on Windows)
- ➕ (side task 22) hooks, keymap and custom commands got a broken `PATH` on Windows: `widened_path` (`ngterm-core`)
  split and joined it with `:`, cutting every `C:\...` entry at the drive's colon and never splitting at `;`. It now
  splits and joins with `std::env::split_paths`/`join_paths` (a quoted Windows entry kept whole), compares entries
  without case or a trailing separator on Windows, falls back to Windows' default system `Path` (`SystemRoot`
  expanded) when the app has none, and appends `~/.local/bin` on Unix only: on Windows the CLI install puts the
  bundled tool's folder on the user's `PATH`, which the widened one already leads with. Its test runs on Windows
  (drive letters, a `;` input, a quoted entry, duplicates in another case). The CLI installer's Unix `on_path` check
  splits by the platform's rules too. Checked and left: `ngterm-term`'s `GHOSTTY_BIN_DIR` append (the platform's
  separator already, exact match as Ghostty's `Exec.zig`), the Windows CLI install's `HKCU` `Path` (`;`, no case),
  the live fish `XDG_DATA_DIRS` (`:` by the XDG spec), the remote tree's `PATH` (a POSIX remote shell), the session's
  Windows shell lookup (`split_paths`), `ngtermctl`'s `PATHEXT` (`;`), and the `ngterm-gpui` build's WPE libdirs
  (Linux only)

### Task 4: The GPUI frontend on Windows (`ngterm-gpui`, depends on Tasks 1 and 3)

- [x] Linux-only dependencies move under target sections; the crate builds on Windows without the `html` feature
- [x] single instance and activation (Decisions > Single instance)
- [x] quit: console Ctrl events and the session's end (`WM_QUERYENDSESSION`/`WM_ENDSESSION`) run the orderly shutdown
  that SIGTERM runs on Linux
- [x] appearance: the system's light or dark setting, followed live
- [x] keyboard: the native key behind each key event, if Ghostty's key encoding needs it on Windows. A
  `gpui-pre-windows` patch through the vendor script, like `gpui-pre-linux`'s; recorded in `build/patches`
- [x] motion (status glyph), folder chooser, monitors and window frames, the quick terminal's window host, the live
  reset relauncher, the selection clipboard: each checked and given its Windows form where the Linux one does not
  carry over
- [x] `ngterm-gpui` (without `html`) builds, lints and tests on Windows; the tests that need Linux (Xvfb, weston,
  lavapipe, D-Bus) are gated
- [x] a `cross-platform.yml` job (or step) builds and tests the app on the Windows runner
- [x] Windows VM: the app launches with a private state dir, opens a window, runs PowerShell in a pane, splits,
  restores; `ngtermctl --socket` drives it (a smoke script kept under `scripts/`)
- ➕ `scripts/smoke-windows-app.ps1` passed in the VM (2026-10-30, debug build) on every step: the control pipe and
  `ngtermctl --socket` (tree, type, text), PowerShell 7 in a pane and in its split, Ctrl+C on the console quits with
  status 0 and the relaunch restores the split, keys typed on the desktop (`SendKeys`: text, Enter, the Up arrow through
  PowerShell's history) run in the pane, the quick terminal shows, runs PowerShell, zooms and hides, a second launch
  forwards over the instance pipe and exits 0, a flip of `AppsUseLightTheme` reloads a two-slot theme,
  `WM_QUERYENDSESSION`/`WM_ENDSESSION` to the hidden window quits with status 0. It needs a staged tree
  (`bin\ngterm.exe` beside `share\ghostty` with its themes, shell integration and terminfo, scripts/stage.sh's layout),
  staged by hand until Task 8's Windows staging script
- ➕ the smoke's desktop typing found Windows keys synthesized with no scan code (`SendInput`/`keybd_event`, as on-screen
  keyboards and automation tools send) dropped when they type no text (Enter): such a key is now placed by its virtual
  key's scan code (`MapVirtualKeyW`), with the `0xE0` prefix of the extended keys, which Windows Server's
  `MapVirtualKeyW` leaves off (the Up arrow came back as the keypad's 8)
- ➕ a Windows test run found the pane reporting a config's own color change as a program's at its next output batch (any
  platform: a prompt redraw after a background change raced the deck's OSC 11 handling); fixed in `ngterm-term`'s
  `Pane::set_colors`, with a test
- ➕ on Windows the unit tests whose panes run POSIX `sh` scripts (the terminal, input, Kitty and overlay-program tests),
  measure DejaVu through fontconfig, or have `cat` repeat an OSC sequence are ignored there with their reason (82
  tests); the deck's and quick terminal's tests run on Windows over an idle `findstr` pane
- ➕ checked on the desktop: the quick terminal's window host (shown, zoomed, hidden), the appearance flip, the session's
  end, Ctrl+C. Checked by build and tests only, not on the desktop: the primary's window rising on an activation, a
  console window's close (`CTRL_CLOSE_EVENT`), the motion setting (`SPI_GETCLIENTAREAANIMATION` on `WM_SETTINGCHANGE`),
  the folder chooser, monitors and window frames, the live reset relauncher (`--relaunch-after`)
- ➕ the HTML clipboard is plain text on Windows (GPUI's Windows clipboard writes no `CF_HTML`)
- ➕ the folder chooser on Windows has no title and no starting folder (GPUI's path prompt takes the accept label only)
- ➕ (side task 23) the HUD works on Windows: its painter, upstream's `hud.sh` (run as `/bin/sh <path>`, which Windows
  handed to `cmd.exe`, so the panel closed at once), is ported to Rust (`ngterm-core`'s `hud::painter`, tested byte for
  byte against the script's frames) and run in process by `ngterm-term` (a painted pane: no PTY, no child, no shell on
  any OS) through `Launch::painter`; the body file, its header, the lifetime (the file's removal ends the panel, a
  replacement keeps the slot), the slot sharing and the close are unchanged. `hud.sh` and its install under the state
  dir are gone, `Core::set_hud_helper` became `set_hud_body_dir`, and the upstream-sync skill files a change to
  upstream's `hud.sh` as a port into the painter. `test_hud_hides_itself_after_its_interval` no longer reads the panel
  back after a 2 s auto-hide was armed (it could be down by then). The windows job ran every HUD id green (60; the two
  needing `posix-command-line` skip there), and they joined the Windows ratchet (702 ids). On the VM's desktop (the
  windows job's stage, a private state dir) a braille-spinner HUD with a dimmed detail showed over the PowerShell pane,
  an update repainted it as markdown in its text color, and a close took it down, with no child process for the panel
- ➕ (side task 30) an overlay program runs on Windows: the POSIX wrapper (`overlay_capture::surface_command`) stays
  Unix's; on Windows there is no wrapper: the overlay terminal runs the command line itself through `cmd /D /S /C`
  (`ngterm_host::command_line`), and `ngterm-term` writes the program's exit status to the overlay's status file when
  it ends by itself (`Launch::status_file`; a program the pane hangs up writes none), `STATUS_CONTROL_C_EXIT` read as
  130 as the overlay job reads it (`overlay_capture::status_of_exit_code`, shared with `ngtermctl`). Edit
  Keymap/Hooks on Windows opens `VISUAL`, else `EDITOR`, else Notepad waited for (`start "" /wait`), read from the
  app's environment (`keymap::windows_editor_command`). The overlay, surface, HUD, split and CLI conformance tests
  build their programs in either dialect (`CommandLines.hold`: `more` under `cmd`, which holds on the console's
  input as `cat` does but erases the line the cursor is on as it starts, so the cursor test holds with `ping` after
  its `set /p`, as a VM probe showed; `pwd`) under `cmd-command-line`. Linux partial run green (156 ids, ratchet holds); the windows
  job ran all 36 of them green (`exit 7` reads back 7, the program runs in its cwd, its text reads back) and they
  joined the Windows ratchet (749 ids). On the VM's desktop (the windows job's stage, a private state dir) an
  overlay program covered the PowerShell pane with its output and working directory, and `ngtermctl session overlay
  open --block 'echo … & exit 7'` exited 7, which `session overlay result` read back in the pane
- ➕ (side task 30) the other `/bin/sh` lines left as they are: the live takeover's fallback creation argv
  (`commands/live/lead.rs`, a line that fails anyway; its attach line is POSIX-quoted), the remote ssh lines (the
  remote host is POSIX), and `cfg(unix)` or test-only code; `ngterm-gpui`'s `privileges` test fails on this dev box
  with its system WPE WebKit at the base commit too (not this change)
- ➕ (side task 29) Ctrl+C in a Windows pane: a key pressed on the desktop always interrupted the pane's program, in
  an in-app pane and a live one, under PowerShell and `cmd.exe` (GPUI's Ctrl+C encodes `0x03`, the pseudoconsole
  makes it the console's Ctrl+C event; no process of the pane ignored it: `ConsoleFlags` read from each one's
  parameters, `ngterm-pty` clearing the spawner's inherited ignore flag). What failed was a `^C` typed through
  `session.type` (`ngtermctl session type`) into an in-app pane, on every OS: each run of the text went to the key
  encoder as an unidentified key carrying it, and `ngterm-ghostty-vt` drops a key's text that holds a control
  character (`event.h` takes none), so the whole run (`f^Cg` too) sent nothing. Live panes type through their daemon,
  raw, and were not affected. `ngterm-term`'s `Pane::type_text` now writes such text as Ghostty's encoder writes an
  unidentified key's text under every keyboard mode (upstream's `inject`: `ghostty_surface_key` with only the text).
  Tests: `platform.rs`'s `a_typed_ctrl_c_interrupts_the_program` (on Windows `ping` ends with
  `STATUS_CONTROL_C_EXIT`), `test_session_text.py`'s `test_a_typed_ctrl_c_interrupts_the_running_program` (green on
  the windows job, in its list); `scripts/smoke-windows-app.ps1` checks a typed `^C` and a pressed Ctrl+C. In the VM
  (the windows job's stage, a private state dir) both ended `ping -t` in in-app and live panes, PowerShell and
  `cmd.exe`, typed and pressed; before the fix the typed ones never did
- ➕ (side task 33) the macOS flake of `ngterm-pty`'s `paused_reads_hold_the_output_back_but_not_the_input` (CI run
  41181727627, no output at all for 60 s) was the build's: its `/bin/sh` died of `SIGILL` between the fork and the
  exec (macOS crash reports: `os_once_t is corrupt` in libnotify's fork handler `_notify_fork_child`, "crashed on
  child side of fork pre-exec"), and the spawn still succeeded. A fork that catches another thread in the process's
  first libnotify use (here `the_environment_and_the_working_directory_are_as_given`, whose command without `HOME`
  makes portable-pty call `getpwuid`; in the app any first `getpwuid`, `localtime` or `confstr`) leaves the child a
  once-gate held by a thread it lacks. `ngterm-pty`'s Unix spawn now finishes libnotify's setup before the fork on
  macOS (`notify_is_valid_token`, which waits for a setup in progress). In the macOS VM, four parallel loops of the
  `pty` binary for 8 minutes: 9 failures in 1472 runs before (8 this test, 1 another's `openpty` `ENXIO`), all 17
  crash reports that signature; after, none of this test in 1306 runs. New `tests/fork_notify.rs` (macOS) runs the
  race in 60 fresh processes: 10 of 10 runs failed before, 0 of 10 after. `ngtermctl`'s own `pre_exec` forks
  (overlay job, terminfo) were left as they are
- ➕ (side task 33, open) another macOS flake of `pty.rs`, there before this change:
  `killing_the_tree_ends_the_child_and_its_descendants` sees its `/bin/sh` exit 0 after `kill_tree`
  (`assert!(!status.success())`). The shell outlives the group's `SIGKILL` and reports its `sleep` killed
  (`Killed: 9 sleep 300`), though `killpg` returned 0 and the group listed both just before. About 1 run in 3000
  run alone (2 of 3113 at the base commit, 1 of 5992 with this change), 3 of 1306 runs of the whole binary; rarer
  still once the kill was instrumented (1 in 19950, then 0 in 23892), so the cause (the kernel skipping the group's
  leader, or the test) is not found yet; `Killer::signal` is unchanged
- ➕ (side task 36) that `killing_the_tree_ends_the_child_and_its_descendants` flake was the build's: macOS signals a
  process group one member at a time (`killpg1` → `pgrp_iterate` takes the members' ids, then signals each in turn,
  nothing holding off their exits meanwhile) and drops a signal to a process already exiting (`P_LEXIT`). The
  `sleep`, the group's newest member, is signalled first and dies; the shell waiting on it wakes, prints
  `Killed: 9 sleep 300` and exits 0 before `killpg` reaches it, so its `SIGKILL` is dropped. A kqueue
  (`EVFILT_PROC`, `NOTE_SIGNAL`/`NOTE_EXIT`) read the moment `killpg` returned found the shell already exited 0 in
  35 of the 97 such runs below, the rest in their exit. Linux holds the task list across the group's signal, so no
  member's exit is reported before all are signalled. `Killer::signal` now signals the child itself first (unreaped,
  its id is still its own), then the group, so it has its signal before any member can end. Measured in the macOS VM
  by the test's scenario run in a loop by 2 threads in each of 4 test processes: 15 runs of 20000 with the shell
  exiting 0 before; 0 of 20000 with the child signalled first; 82 of 20000 with it signalled after the group (the
  order is the fix, not the second signal; a host build ran beside that one); the committed fix 0 of 40000. The test
  alone hit it in none of 12000 runs, nor a one-thread C reproducer of the same kill in 50000: the window needs the
  killing thread held up between two members, which the test binary's busy threads and a loaded VM give.
  Also: the test reads the `sleep`'s id once its line's closing colon came. `ngtermctl`'s overlay job and terminfo
  forks (`pre_exec`) settle libnotify first too: the CLI forks from its only working thread, but its test binary
  runs other tests' threads beside the fork
- ➕ (side task 36, open) macOS `openpty` fails now and then while PTY tests run in parallel: `ENXIO`, or `-6`
  (XNU's internal `EREDRIVEOPEN` from the ptmx clone open, leaking out); 3 failed runs of 1000 runs of the whole
  `pty` binary in 4 parallel loops. Not this task's: left for one of its own
- ➕ (side task 39) that `openpty` flake is XNU's ptmx clone race (`bsd/kern/tty_ptmx.c`, `tty_dev.c`): two opens
  handed one minor, or a minor freed before its `/dev/ttysNNN` node goes, fail with `EREDRIVEOPEN` (-6) or `ENXIO`
  until the racing open or close is through. ngterm-pty opens again on those two errnos only, at most 3 times;
  portable-pty's patch 0007 keeps the failure's `io::Error`, so the errno is readable

### Task 5: Desktop integrations on Windows (depends on Task 4)

- [x] notifications: toasts under the AUMID with the slot identities and replace rules of `notifications.rs`, click to
  reveal and raise, withdrawals
- [x] global hotkey: the chord registered system-wide by physical key position, the press posted to the main loop
- [x] remote links: the network's reachability and resume from sleep as `LinkEvent`s
- [x] status sounds and the keyboard-layout lock re-checked end to end on Windows
- [x] tests on the seams; Windows VM checks of each (a toast shown and clicked, the hotkey toggling the quick terminal)
- ➕ the AUMID's registration (decided here): every launch sets the process's explicit AppUserModelID to the app id and
  writes `HKCU\Software\Classes\AppUserModelId\<app id>` (`DisplayName` ngterm, `IconUri` the bundled icon written to
  `<state dir>\notification-icon.png`), before any window opens. That alone shows toasts for a build run from its
  folder (dev builds, the smoke); Task 7's Start-menu shortcut carries the same id as `System.AppUserModel.ID`, and its
  uninstall should remove that registry key. A test instance's own app id leaves its own key behind (a few bytes)
- ➕ a toast's group is its window id and its tag the rest of the identity (a tag holds 64 characters, an identity is
  longer); app banners go to the `ngterm` group, a long identity shortened with its hash. Toasts are silent: the
  configured sound plays beside them through the status sounds' player, as on Linux
- ➕ a toast left in the notification center after the app quit opens nothing (no COM activator is registered), as a
  freedesktop banner of an app that is gone; relaunching the app from such a click is open work if wanted
- ➕ the global hotkey follows the foreground window's layout (`MapVirtualKeyExW` of the chord's US scan code), read
  again on each foreground change (`EVENT_SYSTEM_FOREGROUND`) and each second; the starter keymap's text names the
  Windows form
- ➕ remote links: the network is the IP helper's connectivity hint (`NotifyNetworkConnectivityHintChange`, resolved at
  run time: a Windows before 10 2004 reads as always reachable); the resume is `PBT_APMRESUMEAUTOMATIC` and the
  console display turning on after off (modern standby's wake) on the app's hidden window
- ➕ status sounds: an asynchronous `PlaySoundW` returned before looking the alias up, so a name Windows has no alias of
  (all of upstream's macOS names) played nothing; it now plays to the end on its queue and falls back to the alert
- ➕ `scripts/smoke-windows-app.ps1` extended and run in the VM (2026-10-30, debug build, run 11 on a guest loaded by
  other tasks' builds): every Task 4 step again, then the global hotkey (`ctrl+shift+\``) pressed with Character Map in
  front shows the quick terminal and pressed again hides it; a `notify` to a session not shown posts a toast under the
  app id (shown with ngterm's name and icon), a click on it reveals that session and raises the window, the clicked
  toast leaves the notification center, a toast sits in its session's slot and a second one replaces it, selecting
  the session withdraws it; `session.status --sound Glass` reaches the player (the alert); the connectivity hint is
  reported at start and a `PBT_APMRESUMEAUTOMATIC` sent to the hidden window retries the remote links; the layout lock
  on Russian selects it and the US keys Q, W, E type `йцу` in the pane. One step stayed red under the load: Character
  Map's window did not come up in 10 s to take the keyboard (it did in run 10; the wait is now 30 s), the hotkey still
  toggling. Not checked on the desktop: a real sleep and resume, a network change, a display-off wake (unit tests on
  the folds only), a toast clicked after the app quit (opens nothing, above)
- ⚠️ the guest showed `ngterm.exe - Application Error (0xc0000142)` (a DLL's initialization failed at process start)
  for a launch while it was loaded by other tasks' builds and installs, and in an earlier run a relaunch sat in process
  creation for 18 minutes with no CPU time; every launch the smoke made in an idle guest started. Not traced to this
  task's code (it fails before `main`); worth watching in Task 8's suite runs
- ➕ the first-run welcome's command line tool sentence is per platform: upstream's Homebrew one on macOS, a package
  install on Linux, the installer on Windows (upstream has no wording for either)
- ➕ (side task 31) the smoke's global-hotkey and layout-lock checks failed on a CI release stage (side task 29's run)
  not because of a later merge or the release build: that run's smoke ran under Windows PowerShell 5.1, whose
  `Set-Content -Encoding utf8` writes a UTF-8 BOM, so `keymap.conf`'s first line read as an unknown binding (the
  "Keymap: 1 issue" banner) and `settings.json` did not decode (no `terminalInputSource`); Task 5's runs used
  PowerShell 7, which writes none. The app now drops a leading BOM from `keymap.conf`, `hooks.conf`, `remotes.conf`
  and `settings.json` (`ngterm_core::hooks::read_config_text`, as Ghostty's config reader does), since Windows' own
  tools write one; the smoke writes its files without one, waits for a relaunched app's window before raising it (a
  single raise 0.2 s after the launch found none and the desktop keys went elsewhere), and on a release build (no
  console: the PE header's subsystem) skips the Ctrl+C quit, saying so, and quits at the session's end instead. The
  full smoke passed in the VM on CI's release stage (Windows PowerShell 5.1, 0 failures) and on a debug build
  (PowerShell 7 and 5.1, 0 failures, the Ctrl+C quit included)

### Task 6: HTML overlays on Windows (WebView2, depends on Task 4)

- [x] a Windows page host behind `ngterm_core::html::HtmlHost` on `wry` (Decisions > HTML overlays): pages over the
  pane and the session, their bounds, visibility, focus and input, the theme, navigation policy and the system
  browser hand-off, the page bridge, saved logins (a persistent profile under the state dir), devtools
  (`ngterm-gpui/src/html_overlay/webview2*`: a `WebView2` child window per page through wry, webview2-com for the
  navigation policy, load events, focus, keys, context menu, script dialogs and the store's clear; devtools off, as
  upstream's pages are not inspectable; the app's keymap chords taken from a focused page; core's file pages take
  Windows paths; `gpui-pre-windows` 0002 puts GPUI's composition target beneath child windows)
  - ➕ a page sits above everything GPUI draws over its content area: it is hidden while a dialog, the palette, a
    popover, the switcher or an alert is up (`set_covered`), but GPUI's in-pane drawing over a page (the HUD, a
    pane ask's dim) stays under it
  - ➕ a file page without `--cwd` loads its text through `NavigateToString`, which `WebView2` caps at 2 MB (a larger
    one fails with `WebView2`'s reason)
- [x] the `html` feature builds on Windows; what `ngterm-wpe` holds that has no WebView2 meaning is named
  (`html_overlay/webview2.rs` module docs)
- [x] tests of the host's frontend-neutral parts on Windows; Windows VM: `ngtermctl session overlay page` shows, its
  bridge round-trips, a login survives a restart (`scripts/smoke-windows-pages.ps1`; CI's Windows job runs
  ngterm-gpui's clippy and tests with `html`)
- [x] the WebView2 runtime's presence is checked at start, with a message when it is missing
- ➕ (side task 26) the gaps Task 8's suite found, fixed in `ngterm-gpui/src/html_overlay/webview2*`:
  - a file page without a grant never reported its title (the hooks were there: granted and URL pages reported
    theirs; a `NavigateToString` document did not): its text now loads through a protocol of its own
    (`https://ngterm-text.<page id>/`, serving that text alone, read back as the `about:blank` it loads at on Linux),
    which also lifts `NavigateToString`'s 2 MB cap
  - the reload of such a page added a history entry: a reload now loads the text (read again) at that one address,
    which replaces the shown entry, so `canGoBack` stays false as upstream's does (VM: two reloads, `navigate back`
    refused, an edited file's new title after a third)
  - a chromeless URL page asked another origin for a frame: `WebView2` sends a frame's request even when
    `FrameNavigationStarting` cancelled its navigation (VM log: the decision `allowed=false`, the server still asked);
    a frame's document request is now decided by the policy again (`WebResourceRequested`, documents only) and a
    refused one answered with a 403 before it reaches the network; the app's protocols and the main frame's document
    are left alone
  - the 10 ids (`test_overlay.py`'s eight page tests and the chromeless frame test, `test_cli_output.py`'s
    `test_overlay_page_blocks_until_submitted`) are in the Windows ratchet
- ➕ (side task 18) Linux: `tests/html_overlay`'s 1080p animation case failed on the dev box's Mesa GPU under host
  load (11.8–23.5 frames/s against a 24 frames/s floor). A bug in the test: the engine drew a new frame for 88 of 89
  presents at load 19 and 6 of 6 at load 70; what slowed was the harness's own present, a debug build's 1080p
  read-back on the test's thread (about 20 ms each idle, 300 ms loaded). The case now presents a fixed 40 times and
  asserts the pace against that run's own presents, capped by the 60 Hz refresh (≥ 0.75 of them on dmabufs, ≥ 1/8 in
  SHM on lavapipe), every frame handed over answered, and still no SHM frame or fall-back on dmabufs; a 50 ms delay
  in answering a frame fails it (0.23)

### Task 7: The Windows installer (depends on Tasks 4–6)

- [x] a WiX MSI (Decisions > Installer) built by a script under `scripts/` and kept in `packaging/windows/`
  (`packaging/windows/ngterm.wxs`, `scripts/build-msi.sh`, which builds and stages the release binaries itself or takes
  a tree `scripts/stage.sh` staged; the installed folder keeps the staged layout)
  - ➕ WiX 5.0.2 (`WIX_VERSION` in `build/pins.env`, the `wix` .NET tool installed into the vendor root), not the current
    7.0.1: 5.0.2 is the last release under the plain MS-RL; 6.x and 7.x ship under the Open Source Maintenance Fee EULA
    (a fee for revenue-generating users, an explicit acceptance in 7.x), which is the user's call. The source's schema
    is the same in 5–7, so a bump is the pin line (plus, for 7, the EULA's acceptance)
  - ➕ per user, not per machine: everything the app registers is the user's (the AUMID key under HKCU, the CLI
    installer's user `PATH` entry, the state and config dirs), so the package owns and removes exactly what it adds,
    the AUMID key included, and installs and upgrades without elevation into `%LOCALAPPDATA%\Programs\ngterm`. ICE38,
    ICE64 and ICE91 (per-machine roaming rules) are suppressed in the validation `build-msi.sh` runs, with the reason in
    the .wxs header
  - ➕ it installs `bin\` (`ngterm.exe`, `ngtermctl.exe`, `ngterm-session.exe`, and the Visual C++ runtime DLLs they
    import, deployed app-locally from the build machine's Visual Studio redist: the binaries link the dynamic CRT, which
    a fresh Windows lacks; the UCRT is part of Windows), `bin\` on the user's `PATH`, `share\` (Ghostty's themes, shell
    integration, terminfo; ngterm's PowerShell integration, the agent-status package, the skill, `app-BUILD`), the
    licenses under `share\doc` (the terminal fonts are compiled into `ngterm.exe`, so only their licenses are files),
    and a Start-menu shortcut with `System.AppUserModel.ID` = the app id, started in the user's profile. Uninstall
    removes `HKCU\Software\Classes\AppUserModelId\<app id>` too. MajorUpgrade under one stable UpgradeCode; the version
    is the staged binary's Cargo version (`--version` overrides)
  - ➕ `share\ngterm\session-BUILD` is not installed: its stat line names the staged binary's mtime to the nanosecond,
    which an installed file never keeps, so the installed session server runs as an unstaged one (no outdated reset
    reason, no handoff to a new build after an upgrade), as `cargo run` does. Open work if handoffs are wanted for MSI
    installs: a build id tied to the binary by something an installer keeps (its content, or a stamp written at install)
- [x] the installed app finds its resources, shell integration and `ngtermctl`; the AUMID shortcut makes toasts work
  (`scripts/check-msi.ps1` on CI and in the VM; the desktop checks below)
- [x] a CI job builds the MSI on the Windows runner and keeps it as an artifact (`.github/workflows/windows-installer.yml`,
  artifact `ngterm-msi`: the release build, the MSI, then `check-msi.ps1` installs it for the runner's user, checks
  the files, the build identity, `ngtermctl` from the registry's `PATH`, the shortcut's target, start folder and AUMID,
  upgrades to an MSI of the next patch version, and uninstalls, checking no folder, shortcut, `PATH` entry, product,
  `HKCU\Software\ngterm` or AUMID key is left and the user's `PATH` keeps its entries and `REG_EXPAND_SZ` kind;
  a separate workflow so it runs beside `cross-platform`'s jobs; 12.5 minutes with a warm cache, 20 with a cold one)
- [x] Windows VM: install, run, upgrade, uninstall (2026-10-30/31, Win11-admin, CI's MSI of `019a2ae` and two MSIs of
  0.1.1 and 0.1.2 repackaged from its own files with `build-msi.sh --version`, on the guest's desktop through
  `vmrun -interactive`, the real per-user locations and state dir): the install (basic UI) puts ngterm in the Start
  menu's search with its icon; a shell Explorer starts afterwards finds and runs `ngtermctl` from the new user `PATH`;
  the app launched from the Start menu's entry for the app id (`shell:AppsFolder\com.nickalie.ngterm`, which only the
  shortcut's AUMID makes) serves its socket, its pane runs PowerShell 7 in the profile with the installed `ngtermctl`
  on its `PATH` and `GHOSTTY_RESOURCES_DIR` at the installed `share\ghostty` (so `xterm-ghostty` resolved); a `notify`
  for a session not shown shows a toast titled with ngterm's name and icon and lands in the notification center under
  the app id; the upgrade to 0.1.1 run without UI while the app ran closed it through Restart Manager (no reboot) and
  the relaunched app restored the session renamed before it; the upgrade to 0.1.2 with the app quit kept it too, and
  the upgraded shortcut carries the AUMID; the uninstall leaves no install folder, shortcut, `PATH` entry,
  `HKCU\Software\ngterm` or AUMID key, and the Start menu no longer finds the app
  - ➕ left after the uninstall, as for any uninstalled desktop app: the user's state and config dirs
    (`%LOCALAPPDATA%\ngterm`, `%APPDATA%\ngterm`, never the installer's) and Windows' own per-app records
    (`Notifications\Settings\<app id>`, `PushNotifications\Backup\<app id>`, `Start\TileProperties`, WebView2's
    profile MACs of the pages' profiles). Removing `Notifications\Settings` would also drop the user's toast choice at
    every major upgrade (the old product's uninstall runs then), so the package leaves it
  - ➕ a Start-menu launch right after a major upgrade can miss for a minute: the shell's index of the shortcut the
    upgrade recreated lags behind it (`shell:AppsFolder\<app id>` opened Documents once; the shortcut itself works)
  - ➕ seen beside the checks, not installer matters: `WM_CLOSE` to the main window (`CloseMainWindow`) did not end the
    app within 15 s (it was stopped by its pid); `GHOSTTY_RESOURCES_DIR` reads `...\ngterm\share/ghostty`, a `/` from
    `ngterm-term`'s `STAGED_RESOURCES` join (Windows takes it)
    - ➕ (side task 21) the close was the app asking, not a hang: a window running sessions asks "Close “window 1”?"
      first (upstream's `windowShouldClose`, as on Linux), and on a fresh state dir the Welcome comes up before it.
      Answered Close, the last window's close quits in under a second with status 0 after the quit's save (VM, debug
      build on the guest's desktop). `scripts/smoke-windows-app.ps1` now checks it: `WM_CLOSE` to the main window, the
      app still running (it asked, `close-asked.png`), Return as a key press (`SendKeys`' Return did not reach the
      alert), exit 0 within 15 s and a new "quit: the state is saved" line; the whole smoke passed on the VM (61
      checks, 0 failures)
    - ➕ (side task 21) the separators: `ngterm_core::paths::join_relative` joins a `/`-separated relative path one
      component at a time, for the staged resources (`ngterm-term`, `ngterm-session`), both build id files,
      `ngtermctl`'s `share/terminfo`, the bundled `share/ngterm`, the XDG data home, bash's `ENV` and the agent hooks'
      config paths (their command strings keep forward slashes on purpose); a Windows unit test holds the staged
      resources at `...\share\ghostty`. Build scripts' `../..` joins stay as they are (never exported or shown)
    - ➕ (side task 21) seen: the quit after the last window's close logs GPUI's "window not found" and "Invalid window
      handle" errors from the teardown of the window already gone; harmless (exit 0), left as is
  - ⚠️ the guest, loaded by other tasks' suite runs, again showed `ngterm.exe - Application Error (0xc0000142)` for one
    launch of a debug build (Task 5's ⚠️); every launch of the release MSI's build started

### Task 8: The conformance suite on Windows (depends on Tasks 2 and 4)

- [x] the harness's transport over the named pipe and its launch of the staged Windows build (private state dir,
  stripped env, the app's process tree reaped at the end)
- [x] a staging script for Windows beside `run-linux.sh`
- [x] a `windows` CI job: protocol, cli and behaviors on the Windows runner, the ids green there held by
  `--must-pass` (a Windows list beside `rust-passing.txt`, kept by `check_ratchet.py`)
- [x] the selftests cover the transport and the launch
- [x] Windows VM: the suite runs there; the failures are fixed in the build or recorded as platform divergences
  (conformance/README.md > Reference discipline)
- ➕ the VM runs (2026-10-31, the windows job's own stage of 13d0004 through `run-windows.sh --stage`): the full suite
  658 passed, 55 failed, 4 errors in 1 h 45 min, while another agent's CPU stress (four busy loops on the guest's four
  CPUs) and a cargo build ran beside it; besides the windows job's red set (below) its failures were control-socket
  timeouts and waits run out on two shared instances. A re-run of the modules holding those ids (protocol's session,
  session_text, surface, window, ask_pick, live_any_shell, config_browser and the chromeless and page-result overlay
  tests, 255 ids) on the quieter guest: 251 passed; red were the CI-red chromeless frame test, `browser.clear`
  outliving the client's 10 s (green in three windows-job runs, kept out of the list), and the run's first instance,
  the profile probe, quitting 12 s after the session's end on a freshly unpacked stage. No pane shell, `ngterm*.exe`,
  `OpenConsole.exe`, PowerShell or session server was left after either run (`ps -ef`, `tasklist`). The windows job is
  the gate: green on every job at f71891c (run 41139442458, 696 passed, the ratchet holding)
- ➕ `run-windows.sh` no longer ends before its report on a source copy without `.git` (the VM's layout)
- ➕ the pane shells "leaked" in the VM run of 2026-10-30 (about 35 `/usr/bin/bash` between 22:02 and 22:08) were no
  leak: they were the panes of `test_dashboard_quick.py`'s shared instance, which its tests open and leave until the
  module's stop (19 at once in a 2026-10-31 slice, none left once the module stopped; `ps -ef` and `tasklist` showed
  no `bash.exe`, `ngterm*.exe`, `OpenConsole.exe` or session server of the run afterwards). Their parent pid of 1 and
  console of their own are what every pane shell shows there: Git for Windows' `bin\sh.exe` is a native launcher of
  `usr\bin\bash.exe`, and each pane has its own pseudoconsole. The guest was loaded by another agent's CPU stress at
  the time. A probe showed the launcher's `bash.exe` in the pane's kill-on-close job, outliving the launcher ended
  alone by pid but not the job's close; the harness now holds each instance's processes in a Job object of its own
  (breakaway allowed for the session server), closed at the stop, which also ends them should the harness die
- ➕ the `windows` job runs with `--must-pass live-any-shell`: the login shell there is Windows PowerShell, so the
  live-any-shell tests type PowerShell's lines on Windows (`Write-Output`, `ping -n` for a long program, the prompt
  waited for after a Ctrl+C, which the console drops typing before); the stage it ran is kept as an artifact
  (`rust-windows-stage`) for a VM run against the same build
- ➕ the Windows ratchet holds 638 ids: the 620 less one found flaky (below), the 16 more green in three windows-job
  runs, and the three live-any-shell tests (held by `--must-pass` too); HUD ids stay out (below)
- ➕ red on Windows and not in the list, each a build gap found by the suite and left to its task:
  - ⚠️ the HUD (`session.hud.*`, 4–6 ids red per run, the rest green only by timing): its painter is
    `ngterm-gpui/resources/hud/hud.sh`, which `ngterm-core`'s `HudRuntime::helper_command` runs as `/bin/sh <path>`,
    a command line Windows hands to `cmd.exe`: it exits at once and the HUD's slot closes. A Windows painter (or the
    panel drawn by GPUI) is open work for the frontend (Task 4)
  - in-app panes report no foreground on Windows (`ngterm-term`'s `Pty::foreground_pid` is `None` there; only live
    panes get the console stand-in, `ngterm-session`'s `console_foreground`): `restore.capture`, `foreground` and the
    relaunch's rerun stay red (`test_restore.py`'s three, `test_relaunch.py`'s
    `test_rerun_replays_the_captured_foreground_program`). Under Git for Windows' sh a newest-child stand-in would
    still name `bash.exe` (the launcher's) or an MSYS fork stub rather than the program
  - ➕ (side task 24) done: an in-app pane's foreground on Windows is read from its program's kill-on-close job
    (`ngterm-pty`'s `JobView`, which never holds the job open): the oldest member started since the pane's program
    that is no idle shell, else that program; a `cmd.exe /D /S /C` wrapper holds its own foreground, as `/bin/sh -c`
    leads its group (`ngterm-session`'s `console_foreground`, `job_foreground`). The job, not the parent links: an
    MSYS program's parent (the fork) is gone by the time it runs, and Git's `bin\sh.exe` launcher's `bash.exe` and
    MSYS forks are idle shells skipped by the rule; `GetConsoleProcessList` would need a helper attached to each
    console. Live panes take the same rule over their shell's descendants (it was the newest direct child). The
    Windows process table models sessions (a process the app started leads one) and groups (a process with its
    descendants) for core's reads; `foregroundShell` drops a Windows `.exe`; the suite puts Git's `usr\bin` on the
    app's `PATH` (for a command line's `cmd.exe`) and compares a foreground argv by its program's name
    (`match.program_argv`). The four ids above are green on the `windows` job and in its list (642). Left as they
    are: a background program (`sleep 600 &`) reads as the foreground until it ends, and a shell typed into the pane
    reads as the pane's shell; a rerun types a POSIX-quoted line, right for the suite's Git sh but not for a
    PowerShell or `cmd.exe` pane, whose replay quoting is open work
  - ➕ (side task 28) a rerun's replay is quoted for the pane's shell (`ngterm-core`'s `shell_dialect`): the seed
    carries the captured argv (`PaneInput::Replay`) and `ngterm-term`'s launch types it for the shell it runs, by the
    program's name. POSIX shells keep upstream's line and its line feed; fish gets its own quoting; PowerShell (7 and
    5.1) `& 'program' 'word'`, an argv the argument passings would not all hand on alike run under `Legacy` in a scope
    of its own with each word quoted for the C runtime's split (exact on 5.1 and every 7, on every OS); `cmd.exe` the
    C runtime's words with `^"` for every quote, carets before its metacharacters and after each `%` (limits: batch
    files and internal commands, delayed expansion); both submitted with a carriage return. An unknown shell gets only
    plain words, else nothing (a plain shell). A pinned line takes the shell's Enter too. The live replay's PowerShell
    call is the same, so Windows PowerShell 5.1 no longer loses an empty argument or an embedded quote there. Tested
    per dialect with nasty arguments through each shell on the host (`sh`, fish, `pwsh` on every OS; `powershell.exe`
    and `cmd.exe` on Windows), in real panes (`ngterm-term`'s `tests/replay.rs`: PowerShell 7, 5.1 and `cmd.exe` on
    Windows), and in the suite: the `windows-shells` capability (claimed on Windows) and `launch(pane_shell=...)` run
    `test_relaunch.py`'s replay and pin tests in PowerShell 5.1, PowerShell 7 and `cmd.exe` panes; their six ids
    passed on the `windows` suite job and joined the Windows ratchet
  - `ngtermctl terminfo install` dumps the entry with `/usr/bin/infocmp`, which Windows lacks (two
    `test_cli_only.py` ids): a Windows form (Task 2) would read the staged `share/terminfo` instead
  - a file page's title (`htmlOverlays[].title`) never comes, on the CI runner and in the VM, so eight of
    `test_overlay.py`'s page tests wait out their read-back; and a chromeless URL page asks another origin for a
    frame (`test_chromeless_url_page_keeps_frames_on_its_origin`) (Task 6)
  - a file page without a grant reloads its text through `NavigateToString` again, which adds a history entry, so
    `canGoBack` turns true once `WebView2`'s `HistoryChanged` lands: `navigate back` after a reload is refused or not
    by that race (`test_cli_output.py::test_overlay_page_blocks_until_submitted`, green in three runs, red in the
    fourth, taken out of the list); upstream's reload of such a page adds none (Task 6)
- ➕ (side task 37) `ngterm-session`'s `tests/handoff.rs` failed now and then with ETXTBSY ("Text file busy") starting a
  staged copy (`a_client_of_another_build_hands_the_server_over_…`, `an_update_hands_a_running_server_…`): the
  multithreaded write-then-exec race. `support::stage` wrote the copy in the test process (std's `File`, already
  `O_CLOEXEC`; the temp name and rename it used change nothing, the inode is the same), and a child another test
  thread forked during that write inherited the write descriptor and held it until its own exec, so starting the copy
  first failed. A diagnostic scan of the test process's children (`/proc/self/task/*/children`) caught them: 47
  children holding the copy while it was written in 30 runs, and three still holding it (`O_WRONLY|O_CLOEXEC`, the
  test binary's image not yet exec'd, named after the forking thread: another test's or a session server's) 0.5 to
  0.7 ms after the test had closed it. Fixed in the test support: `ngterm-host`'s fakes gain `copy_executable`, a
  child `install` writing the copy, beside `write_executable` (the same fix for scripts, which now goes through it),
  so the test process never holds a write descriptor to anything it runs; `stage`, `tests/replay.rs`'s scripts and
  `ngterm-gpui`'s `tests/live_update.rs` stage use them. No `O_PATH`/`fexecve` route helps: the kernel refuses the
  exec on the inode's open writers, however it is named. macOS has the same race (its exec refuses a vnode open for
  writing) and the same fix (`install` is BSD's there); Windows has no ETXTBSY, and no equivalent sharing violation
  here: a file's handle is not inheritable, so no child takes it, and its copy stays `fs::copy`. The app itself has
  no such write: on Unix the stager (`scripts/stage.sh`), another process, writes the binary before the app runs it,
  and the Windows server's private image is written in-process but runs only after it is closed, with nothing
  inheriting its handle. Rate on Linux (the handoff test binary in a loop): 3 ETXTBSY failures in 200 runs before,
  0 failures in 400 after (where 3 in 200 would leave about a 0.2% chance of none). Left as they are, the same
  pattern in other crates' tests, none seen failing: scripts written in process and run by `ngterm-app`
  (`hook_runner`'s quoted command line, `adapter_tests`, `restore_tests`, `tests/opencode_status.rs`,
  `tests/show_image.rs`) and by `ngterm-gpui`'s app (`tests/keyboard_layout.rs`, `tests/throughput.rs`);
  `write_executable` is their fix
- ➕ (side task 40) the write-then-run sweep: every test that wrote a file in its own process and then ran it now
  stages it through `ngterm-host`'s fakes (`write_executable` for a script, `copy_executable` for a binary). The
  seven sites side task 37 listed take `write_executable`; the sweep (every exec mode set, `fs::copy`/`hard_link`
  of something then started, every shebang written) found two more, both copies of a binary: `ngterm-ctl`'s
  `tests/cli_local.rs` copies `ngtermctl` when a hard link fails, and `ngterm-wpe`'s `engine::spares` tests copy
  `/bin/sleep` as a stand-in web process (`ngterm-wpe` gains the `fakes` dev-dependency). Checked and left: files
  given an exec mode but never run (`restore::mode`'s and `launch`'s probes, `integrations::files`' mode tests, the
  WPE provider's stand-in `WPEWebProcess`), scripts copied and run through `bash`/`powershell` (a read, not an exec:
  `show_image`'s skill script, the `-sys` crates' build scripts), stubs written without an exec mode (never run),
  the session server's private image (run only on Windows, and in tests only compared), Windows-only copies
  (`adapter_tests`' `cmd.exe`), and the app's own installs (`integrations::files::write_file` writes the agent hooks
  for the agents to run, never this process). No Rust helper sits under `conformance/`.
- ➕ (side task 43) `test_cli_output.py::test_tree_lists_workspaces_and_tagged_sessions` failed now and then on
  Windows (CI run 41221397790): the human `tree` carried `title: C:\Program Files\Git\bin\sh.exe` on the last session
  where the `--json` read just before it had none. The test's: the console host (ConPTY) titles each pane with its
  program's path (`untitled-panes` unclaimed, rust-build.md), and a VM probe over 20 new sessions found the title
  absent at `wait_mounted` every time and arriving 0.3 to 1.1 s later, so the fixture's newest session could take
  its title between the two reads. The build is right as it stands: the tree reports the left pane's terminal title
  as upstream's does (`SocketClient.swift`'s `  title: ` suffix), and macOS's and Linux's `/bin/sh` set none.
  `_titles_settled` waits until every realized session has its title where `untitled-panes` is not claimed (no-op
  elsewhere), before each `--json`/human `tree` comparison: the tree test and `test_session_overlay_visibility`'s
  two, the sweep's only other two-read `tree` comparisons in `tests/cli` (the title is the one session field
  `ngtermctl tree` prints). On the VM, the CI stage of main at 1a38010d, one fresh instance per run of both tests:
  the old test failed 1 in 25 runs with this very diff, the new one passed 40 of 40

### Task 9: Docs

- [x] README: Windows build, run, install and test; CI list; prerequisites (WebView2, WiX)
- [x] CLAUDE.md's crate table: each crate's platform column as it now stands
- [x] `conformance/docs/rust-build.md`: the Windows profile, divergences and renamed strings, if any
- ➕ the bundled skill's copy had drifted from upstream plus its patches (side task 25 edited `terminfo install`'s text
  in `reference.md` without a patch, so `sync-resources.sh --check` read `would-apply`); `0007-windows.patch` carries
  that text with the skill's Windows paths, and the check reads `unchanged` again
- ➕ `scripts/run-gpui-dev.sh` has no Windows form and was not run there; README > Windows names what a private dev
  instance needs on Windows (the smoke scripts' and the harness's launch)
- ➕ left open: the skill's `scripts/show-image.sh` opens its overlay as `/bin/bash <script>`, a POSIX line `cmd.exe`
  cannot run on Windows
- ➕ (side task 32) the skill's `show-image.sh` and `scripts/run-gpui-dev.sh` have Windows forms. A Windows pane's
  program writes through the pseudoconsole, and kernel32's ConPTY drops the kitty graphics protocol's APC escapes (a VM
  probe: an overlay running Git Bash's `printf` of a PNG transmission showed the text around it and no image; its
  `CSI 14t`/`18t` geometry replies did come back), so no program overlay can draw an image on Windows, whatever runs
  it. `scripts/show-image.ps1` (PowerShell 5.1 and 7) shows the image in a page overlay instead: a self-contained
  page under `%TEMP%` holding it as a data URI, scaled to fit and centered above "press Enter to close", opened with
  `session overlay open --html FILE --js --chromeless --size-percent N` on the pane's `NGTERM_SOCKET`; Enter closes it
  through the page bridge (the page takes the keyboard as it opens, as a program overlay does). `show-image.sh` under
  Git Bash hands over to it, so the skill's one line works from Claude Code's Git Bash; SKILL.md, reference.md and
  examples.md give the PowerShell line. Recorded as `build/patches/skill-ngterm/0008-show-image-windows.patch`;
  `sync-resources.sh --check v0.38.1 v0.38.1` reads `unchanged`. `ngterm-app`'s `tests/show_image.rs` runs the
  script with a recording `ngtermctl`: the program overlay and its `--emit` line on Unix, the page and its contents on
  Windows from PowerShell and from Git Bash, and the refusals (no image, a missing one, outside ngterm) everywhere.
  `run-gpui-dev.sh` (`scripts/lib/dev-instance.sh`) runs under Git Bash: the root under `%TEMP%`, Windows paths in
  what it sets and prints, no socket-length limit (the path names a pipe), and the app started as its child, whose
  Windows pid it prints (`taskkill /F /PID`) before waiting for it; on Linux it now prints the pid too (`exec` keeps
  the script's). README > Windows points at it in place of Task 9's note. Checked on Linux headless (Xvfb, a private
  bus, lavapipe): the dev instance started, `show-image.sh` opened its overlay there, `kill <pid>` stopped it. On the
  VM's desktop: the CI stage with a private state dir showed the image from PowerShell and from Git Bash and Enter
  closed it; `run-gpui-dev.sh` from Git Bash built, staged and launched the debug app on a private state dir and app
  id, `ngtermctl --socket` drove it, the image showed there too, and `taskkill /F /PID` on the printed pid stopped it
  and ended the script

### ➕ revmux round 1 fixes (side task 38)

The final revmux round over the whole plan found seven; each is fixed in a commit of its own:

- ➕ [major] `is_command_wrapper` knew a Windows `--command` pane's `cmd.exe /D /S /C "<line>"` only as five argv
  words, and `CommandLineToArgvW` splits a line holding a quoted word with a space into more (`"C:\Program
  Files\nodejs\node.exe" server.js`, `echo "a b"`, a redirection to a quoted path), so the console's foreground went
  to the program and the capture replayed it over the creation command. The Windows process table now reads such a
  command line whole (`command_restore::command_wrapper_argv`: the program as the C runtime reads it, the three
  switches, the line as `cmd` reads it under `/S`), and `is_command_wrapper` knows the wrapper by its four leading
  words whatever follows. Tests: core's round trip of `ShellCommand::cmd` lines with quoted paths, spaces and
  redirections through three `%ComSpec%` spellings and the `/S` rule's edges; the session's `console_foreground` on
  a split wrapper; the Windows process table's live test spawning those lines (`ping` behind a quoted path writing
  to a directory with a space, `echo "a b" & ping`), reading back the five words and the wrapper as the foreground
- ➕ [major] the restore denylist (`should_restore`, `restart_replay::resolve`) and `$SHELL`'s match in
  `is_known_shell` compared basenames exactly, so `ZELLIJ.EXE` or `ssh.exe` was never denylisted on Windows. They now
  go through `command_restore::same_program`: on Windows in any case and without a `.exe`, `.com`, `.bat` or `.cmd`
  on either side (`shell_name`'s normalisation, generalised); Unix stays exact. The sweep found no other basename
  comparison over a captured argv (`LoginShell::of` reads the configured shell, a Windows live pane runs PowerShell
  only; `ngtermctl live list` only shows the name). Tests: `same_program`'s Windows and Unix readings, the denylist
  and `$SHELL` on each OS, `resolve` refusing Windows-spelled programs (Windows only)
- ➕ [major] a granted file on a network share lost its server in `WebView2`'s grant address. A share's file now
  loads as `https://ngterm-file.localhost/UNC/<server>/<share>/...` and reads back as `file://<server>/<share>/...`
  (`app_uri`, `request_app_uri`); a drive path's first segment is always `X:`, so the two never collide.
  `policy::granted_file` refuses a path on another volume (drive, or server and share, in any spelling) before it
  asks the file system, so no page's root-relative `/UNC/...` makes Windows reach a server the grant does not name.
  Tests: the UNC and verbatim-UNC round trips, `volume`'s spellings, and the grant served end to end for a drive's
  and a share's file (the runner's administrative share of the same directory), inside and outside the grant
- ➕ [minor] `of_shell` reads a configured shell past `env` (its switches, `-u`/`-C`/`-P` with their values, long
  options, `NAME=value`, `-S`/`--split-string`'s words, `--`, a nested `env`, an `env.exe` path with spaces), and
  `POSIX_SHELLS` gains `ksh93`, `loksh`, `lksh`, `rbash`, `posh`, `bosh` and `hush`. Unknown shells keep their
  documented plain-words policy. Tests: `a_shell_is_known_by_its_programs_name`,
  `a_configured_shell_is_read_through_its_spaces_and_arguments`
- ➕ [minor] the `WebView2` file-chooser guard is still page-world JS: `WebView2` has no native refusal (webview2-com
  0.39.1's API has only `SaveAsUIShowing`, the page's own Save As, and `PermissionRequested`'s `FILE_READ_WRITE`,
  which comes after a File System Access picker), no setting for it and no script worlds. The guard now binds every
  DOM function it calls before the page's scripts run, tells a file input by its own `type` getter's brand check,
  and refuses `click()` and `showPicker()` on one (a detached input's click never reached the window's listener); its
  docs say it is defence in depth, with the bypass left (a realm it does not run in, such as a fresh same-origin
  `about:blank` frame). Checked in a headless Chromium: after a page replaced `preventDefault`, `composedPath`,
  `Array.prototype.some`, `type` and `Symbol.hasInstance`, a file input's click was still prevented, a detached one's
  `click()` and `showPicker()` refused, a text input untouched (no automated test: no DOM in the unit tests)
- ➕ [minor] toast tags hash with `ngterm_core::live::fnv1a64`; the private copy is gone. Test: two shortened tags
  pinned to their text as the old code made them
- ➕ [minor] `ngterm_core::pipe_security::check_server_user` holds the pipe-server user refusal and its message for
  the session pipe's, the control socket's and the instance pipe's clients; each still builds its own descriptor.
  Test: `a_pipe_served_by_another_user_is_refused_by_name` (Windows)

### ➕ revmux round 2 fixes (side task 41)

Round 2 (`02-after-fix`) found six and one pre-existing; each is fixed in a commit of its own:

- ➕ [major] `policy::granted_file`'s volume pre-check (side task 38) compared the grant's real path with the request
  as the page's address spells it, so a page on a mapped network drive, a `subst` drive or a junction to another
  volume was refused its own files. The pre-check stays, as it keeps a page's `/UNC/<server>/...` from making Windows
  open an SMB connection (and hand over NTLM credentials) to a server the page chose; a grant
  (`policy::GrantRoot`) now keeps the volume its page named beside its real path, and a request on either is
  resolved, anything else still refused before the file system is asked
- ➕ [major] a share's server reaches the request lowercased (and punycoded) through the `file:` URI's host, while the
  grant keeps the spelling it was resolved from, and `Path::starts_with` compared the prefix's server exactly.
  Containment (`strictly_inside`) now compares the volume as Windows reads it (case-insensitive, the server through
  `url::Host::parse` as the address spells it) and the rest exactly. Tests: `strictly_inside` and `volume` on
  differently spelled servers (Windows), and `the_grant_serves_a_drives_and_a_shares_files_only_inside_it` grants a
  page through the machine's name in capitals (checked to stay capitalised in the real root), a `subst` drive and a
  `net use` drive besides the drive and the `127.0.0.1` share
- ➕ [minor] `ShellDialect::of_shell` read `env` off the value's last component whatever came first, so a shell's
  argument (`bash --rcfile /etc/env`) made it Unknown; the same construct took a last-component shell over the
  first word (`pwsh -File C:\scripts\bash` read as POSIX). The program is now the value's shortest run of leading
  words naming a known shell or `env` (a path with spaces read whole). Tests: the cases above and their kin
- ➕ [minor] after env's `--`, its `-` and `NAME=value` words are skipped before the program, as GNU env reads them
- ➕ [minor] the `WebView2` file-chooser guard refuses `dispatchEvent` of a `click` on a file input (Chromium activates
  a page's synthetic click under a user gesture), and a root `attachShadow` makes gets the window's click listener,
  which a closed root hides its inputs from. The guard moved to `webview2/guard.js`; the WPE harness runs it in
  `WebKit` (`the_webview2_guard_keeps_scripted_and_shadowed_clicks_off_file_inputs`: a dispatched and a called click
  never reach a file input, a text input's does, a real click on a file input in a closed shadow root arrives
  cancelled). Gaps left, in the module docs: another realm, a closed root declared in markup, a click a page stops
  before it reaches the root
- ➕ [minor] `portable_ptys_openpty_failure_keeps_its_errno` reaches portable-pty's real `openpty` failure (a child
  process that spent its descriptor limit) and reads `EMFILE` through the chain the clone race's retry reads
  (`openpty_errno`); without patch 0007 it fails. Patch 0007 names that test; `vendor-portable-pty.sh --check` holds
- ➕ [pre-existing, major] `parse_denylist` reads CRLF and lone-CR line endings, as `keymap.conf`, `hooks.conf` and
  `remotes.conf` already did (the sweep of core's config readers found no other: `ghostty.conf` trims `\r`).
  Test: `the_denylist_reads_windows_and_old_mac_line_endings`

### ➕ one volume spelling for every grant check (side task 42)

- ➕ [pre-existing, major, revmux round 4] the navigation policy's containment (`html::contains`, which
  `navigation::decide` runs on every navigation, `WebView2`'s `NavigationStarting` included) and `grant_error`
  lowercased a Windows path's prefix and compared it as written. A grant on `\\ÑAS\share\site` met the page's address
  as `\\xn--as-yja\share\...` (a `file:` URL's host), and a grant on `\\?\C:\site` met `C:\...`, so the page's own
  navigation was cancelled before the grant protocol's `granted_file` was asked. Side task 41's `volume` and
  `strictly_inside` moved from `ngterm-gpui`'s `html_overlay::policy` into `ngterm-core` (`html::volume`, `any`),
  which `policy` now reuses: a drive or share in one spelling whether verbatim or not, the server as an address
  spells it (`url::Host::parse`: lowercased, punycoded), share and drive in any case; the rest of the path compares
  as before (lexically and in any case in `components`, exactly on real paths in `strictly_inside`). `components`
  takes its root from `volume`, so `contains`, `grant_error` and its home check read every spelling alike
- ➕ the sweep (`starts_with`, `strip_prefix`, `components`, `Prefix` over grant and root paths in `ngterm-core`,
  `ngterm-gpui`, `ngterm-wpe`, `ngterm-app`) found one more: `remote::page_serve::resolve`, which serves a replica's
  resource requests on the machine that opened the page. It canonicalized any absolute path a presenter asked for, so a
  replica page could make this machine open an SMB connection to a server it chose, and compared the real paths with
  `Path::starts_with`. It now refuses a request on a
  volume the page's grant (or, without one, its file) names neither as given nor as it resolves before the file
  system is asked, as `granted_file` does, and checks containment with `strictly_inside`. Left as they are, not
  grant containment: `link_policy`'s automount roots (Unix paths), `live`'s verbatim-prefix strip (a hash's
  spelling), `remotes_watcher::concerns` (watched paths against the watcher's own events), `integrations::files`'
  test walk. Tests (Windows, CI's crates job): `a_file_page_stays_inside_its_grant_whatever_its_volumes_spelling`
  (`decide` on the finding's two cases and their kin), `a_windows_volume_compares_in_any_of_its_spellings`
  (`grant_error`, `contains`), `a_grant_serves_only_requests_on_the_volumes_it_names` (`resolve` serves this
  machine's administrative share by its name in capitals, lowercased and verbatim, and a `subst` grant on its named
  and its real drive, and refuses a `subst` drive onto the grant's drive that the page never named, which resolves
  inside the grant), and the moved `volume` and `strictly_inside` cases; each of the first three fails without the
  fix. Windows canonicalizes a share's server and name to one spelling, so `resolve`'s real paths never differed
  there; its change shows in the volume check
- ➕ [major, revmux round 5] `volume` spelled a share's server through `url::Host::parse`, which percent-decodes,
  maps compatibility forms and reads shorthand IPv4 before IDNA, so `\\n%61s\share` had `\\nas\share`'s volume while
  Windows reads `n%61s` as a name of its own: a replica page's `ngterm-page://<id>//n%2561s/share/...` (decoded once
  by `replica_path`) passed `page_serve::resolve`'s pre-check against a grant on `\\nas\share` and Windows then
  resolved `n%61s` (DNS, LLMNR, NBT-NS) and opened SMB to whoever answered; `policy::granted_file` had the same hole.
  `html::volume`'s server spelling (`volume::server`) now takes the parser's spelling only when it is the name in
  another case or the IDNA form of a real Unicode name (read back through `idna::domain_to_unicode`, `idna` the URL
  parser's own crate, now a direct dependency of `ngterm-core`), else keeps the name as written, lowercased; every
  check goes through it. Tests: `a_server_is_spelled_as_its_address_only_when_windows_reads_the_same_host` (every
  OS), `volume`, `strictly_inside`, `grant_error`/`contains`, `page_serve`'s
  `a_request_on_a_server_spelled_to_read_as_the_grants_is_refused` (no file system answers either side, so only the
  volume check refuses it) and `policy`'s `only_a_request_on_a_volume_the_grant_names_reaches_the_file_system` (the
  pre-check alone, `GrantRoot::names_volume_of`); each fails without the fix
- ➕ [major, pre-existing, revmux round 5] a grant on `\\localhost\share\site` passed `grant_error`, but its
  `WebView2` address read back as `file://localhost/share/...`, which the URL parser shortens to `file:///share/...`:
  the navigation was cancelled and the grant protocol found no Windows path. A share on `localhost` now reads back on
  `127.0.0.1` (`scheme::granted_file_uri`), and `volume` spells the server `localhost` as `127.0.0.1`, the same host
  to Windows, so the grant's checks take the page's address and its grant as one volume; the NTLM pre-check is
  unchanged. Tests: `a_share_on_localhost_reads_back_on_its_address` (address, read-back, `decide`), and
  `the_grant_serves_a_drives_and_a_shares_files_only_inside_it` grants through `\\localhost\<drive>$` again beside
  `127.0.0.1`, and now runs `decide` on each route's own document before the grant serves it
