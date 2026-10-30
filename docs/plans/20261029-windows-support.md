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

### Task 3: App services on Windows (`ngterm-app`)

- [ ] the hook runner's Windows form (no `poll`; pipe readers on threads, the same timeout and output caps)
- [ ] the log file's Windows form (open without following a reparse point, shared for appending, the same rotation
  under the running instance's lock)
- [ ] the remote presentation process's Windows form (its lifetime, stop and reap)
- [ ] the agent integrations' installers without file modes or symlinks on Windows (what each one installs and where,
  per agent, as each agent documents for Windows)
- [ ] `ngterm-app` joins PACKAGES; its tests pass on all three
- [ ] Windows VM: clippy and tests green; Linux gate green

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
  status 0 and the relaunch restores the split, keys typed on the desktop (`SendKeys`) run in the pane, the quick
  terminal shows, runs PowerShell, zooms and hides, a second launch forwards over the instance pipe and exits 0, a flip
  of `AppsUseLightTheme` reloads a two-slot theme, `WM_QUERYENDSESSION`/`WM_ENDSESSION` to the hidden window quits with
  status 0. It needs a staged tree (`bin\ngterm.exe` beside `share\ghostty` with its themes, shell integration and
  terminfo, scripts/stage.sh's layout), staged by hand until Task 8's Windows staging script
- ➕ the smoke's desktop typing found Windows keys synthesized with no scan code (`SendInput`/`keybd_event`, as
  on-screen keyboards and automation tools send) dropped when they type no text (Enter): such a key is now placed by
  its virtual key's scan code (`MapVirtualKeyW`)
- ➕ a Windows test run found the pane reporting a config's own color change as a program's at its next output batch
  (any platform: a prompt redraw after a background change raced the deck's OSC 11 handling); fixed in
  `ngterm-term`'s `Pane::set_colors`, with a test
- ➕ on Windows the unit tests whose panes run POSIX `sh` scripts (the terminal, input, Kitty and overlay-program
  tests), measure DejaVu through fontconfig, or have `cat` repeat an OSC sequence are ignored there with their reason
  (82 tests); the deck's and quick terminal's tests run on Windows over an idle `findstr` pane
- ➕ checked on the desktop: the quick terminal's window host (shown, zoomed, hidden), the appearance flip, the
  session's end, Ctrl+C. Checked by build and tests only, not on the desktop: the primary's window rising on an
  activation, a console window's close (`CTRL_CLOSE_EVENT`), the motion setting (`SPI_GETCLIENTAREAANIMATION` on
  `WM_SETTINGCHANGE`), the folder chooser, monitors and window frames, the live reset relauncher (`--relaunch-after`)
- ➕ the HTML clipboard is plain text on Windows (GPUI's Windows clipboard writes no `CF_HTML`)
- ➕ the folder chooser on Windows has no title and no starting folder (GPUI's path prompt takes the accept label only)

### Task 5: Desktop integrations on Windows (depends on Task 4)

- [ ] notifications: toasts under the AUMID with the slot identities and replace rules of `notifications.rs`, click to
  reveal and raise, withdrawals
- [ ] global hotkey: the chord registered system-wide by physical key position, the press posted to the main loop
- [ ] remote links: the network's reachability and resume from sleep as `LinkEvent`s
- [ ] status sounds and the keyboard-layout lock re-checked end to end on Windows
- [ ] tests on the seams; Windows VM checks of each (a toast shown and clicked, the hotkey toggling the quick terminal)

### Task 6: HTML overlays on Windows (WebView2, depends on Task 4)

- [ ] a Windows page host behind `ngterm_core::html::HtmlHost` on `wry` (Decisions > HTML overlays): pages over the
  pane and the session, their bounds, visibility, focus and input, the theme, navigation policy and the system
  browser hand-off, the page bridge, saved logins (a persistent profile under the state dir), devtools
- [ ] the `html` feature builds on Windows; what `ngterm-wpe` holds that has no WebView2 meaning is named
- [ ] tests of the host's frontend-neutral parts on Windows; Windows VM: `ngtermctl session overlay page` shows, its
  bridge round-trips, a login survives a restart
- [ ] the WebView2 runtime's presence is checked at start, with a message when it is missing

### Task 7: The Windows installer (depends on Tasks 4–6)

- [ ] a WiX MSI (Decisions > Installer) built by a script under `scripts/` and kept in `packaging/windows/`
- [ ] the installed app finds its resources, shell integration and `ngtermctl`; the AUMID shortcut makes toasts work
- [ ] a CI job builds the MSI on the Windows runner and keeps it as an artifact
- [ ] Windows VM: install, run, upgrade, uninstall

### Task 8: The conformance suite on Windows (depends on Tasks 2 and 4)

- [ ] the harness's transport over the named pipe and its launch of the staged Windows build (private state dir,
  stripped env, the app's process tree reaped at the end)
- [ ] a staging script for Windows beside `run-linux.sh`
- [ ] a `windows` CI job: protocol, cli and behaviors on the Windows runner, the ids green there held by
  `--must-pass` (a Windows list beside `rust-passing.txt`, kept by `check_ratchet.py`)
- [ ] the selftests cover the transport and the launch
- [ ] Windows VM: the suite runs there; the failures are fixed in the build or recorded as platform divergences
  (conformance/README.md > Reference discipline)

### Task 9: Docs

- [ ] README: Windows build, run, install and test; CI list; prerequisites (WebView2, WiX)
- [ ] CLAUDE.md's crate table: each crate's platform column as it now stands
- [ ] `conformance/docs/rust-build.md`: the Windows profile, divergences and renamed strings, if any
