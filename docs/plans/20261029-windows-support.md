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
  - `ngtermctl terminfo install` dumps the entry with `/usr/bin/infocmp`, which Windows lacks (two
    `test_cli_only.py` ids): a Windows form (Task 2) would read the staged `share/terminfo` instead
  - a file page's title (`htmlOverlays[].title`) never comes, on the CI runner and in the VM, so eight of
    `test_overlay.py`'s page tests wait out their read-back; and a chromeless URL page asks another origin for a
    frame (`test_chromeless_url_page_keeps_frames_on_its_origin`) (Task 6)
  - a file page without a grant reloads its text through `NavigateToString` again, which adds a history entry, so
    `canGoBack` turns true once `WebView2`'s `HistoryChanged` lands: `navigate back` after a reload is refused or not
    by that race (`test_cli_output.py::test_overlay_page_blocks_until_submitted`, green in three runs, red in the
    fourth, taken out of the list); upstream's reload of such a page adds none (Task 6)

### Task 9: Docs

- [ ] README: Windows build, run, install and test; CI list; prerequisites (WebView2, WiX)
- [ ] CLAUDE.md's crate table: each crate's platform column as it now stands
- [ ] `conformance/docs/rust-build.md`: the Windows profile, divergences and renamed strings, if any
