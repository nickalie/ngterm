# Windows 4: the per-user MSI

## Overview

- **Goal**: ngterm installs on Windows from a per-user MSI, upgrades in place without ending live sessions, and
  uninstalls cleanly. Last of the four Windows plans; `20261025-windows-1-control-plane.md` holds the series' goal,
  done bar, targets, decisions and what is out of it.
- **Decisions** (user, 2026-10-25):
  - a per-user MSI built with WiX, not MSIX (whose virtualized file system and registry fight the session server's
    detached spawn, its private binary copy and the agent hook installs) and not just a staged directory;
  - one MSI per architecture, x64 and arm64;
  - no signing in this plan: an unsigned MSI meets SmartScreen; signing needs a certificate and secrets the user sets
    up, as a change of its own.

## Context (from discovery and the plan review)

- `scripts/stage.sh` writes the layout the suite, the dev launch and the installed identity read; plan 1 gave it its
  Windows form (`bin\ngterm.exe`, `ngtermctl.exe`, `ngterm-session.exe`, `share\…`, `share\ngterm\app-BUILD`).
- `version` reports the installed build beside the running one (`result.installed`), from the layout's `app-BUILD`
  (`crates/ngterm-core/src/identity.rs`).
- The workspace version is `0.1.0`.
- Live sessions on Windows run from a private copy of `ngterm-session.exe` in the namespace directory, so a new build
  can be staged over the running one; the app hands the sessions to the new server (the update handoff) when the
  session build ids differ.
- `ngtermctl.exe` runs from the install's `bin` inside sessions and hooks (`events`, `live attach`, `session wait`,
  the agent-status payloads), so it can be in use during an upgrade.
- Plan 1: the app quits without a prompt on `WM_QUERYENDSESSION`/`WM_ENDSESSION` with `ENDSESSION_CLOSEAPP`; the agent
  payloads are refreshed when the running build is newer than their copy; "Install Command Line Tool…" edits the
  user's `PATH` on Windows.
- Plan 2: the AppUserModelID derives from the app id and is set through GPUI's `set_app_identity` (which calls
  `SetCurrentProcessExplicitAppUserModelID`); GPUI writes `HKCU\Software\Classes\AppUserModelId\<id>` on the first
  toast. The private state dir and app id reach the app through `NGTERM_STATE_DIR` and `NGTERM_APP_ID`.

## Development Approach

- **testing approach**: Regular (code first, then its tests in the same task).
- complete each task fully before moving to the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests**, success and error cases.
- **CRITICAL: all tests must pass before starting the next task**, on both Windows runners.
- **CRITICAL: update this plan file when scope changes during implementation.**
- **Local Windows work in the VM** (user, 2026-10-25): a task that needs Windows by hand (a red test or id to
  debug, a probe, an interactive check) starts the Win11-admin VMware VM, works there and stops it at the end of
  the task (memory: windows-macos-debug-in-vmware). The VM is off by default and never left running between tasks;
  CI on the Windows runners stays the gate.
- Scripts stay bash (Git Bash on Windows).
- An install test never touches the runner's or the VM user's default state dir or app id: every launch, relaunch
  included, runs on a private state dir and app id.

## Testing Strategy

- **unit tests**: per task (the restart registration and its arguments, the build script's refusals).
- **install tests**: a pytest module run by the `windows-msi` job on both arches: install, check, launch on a private
  state dir and app id, upgrade with a live session and a running `ngtermctl events`, uninstall; the harness's pipe
  client.
- **conformance suite**: once at the end (Task 8), plus `run-linux.sh` once on Linux.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- keep the plan in sync with the work done

## Solution Overview

- **WiX v6** (the `wix` .NET tool, pinned in a dotnet tool manifest, its EULA accepted with `-acceptEula wix6`;
  `cargo-wix` targets WiX 3), from a checked-in `packaging/windows/ngterm.wxs`. Its payload is the staged layout,
  harvested from the stage directory, so the installed tree is the staged tree and the installed identity works.
- **Per user, no elevation**: `Scope="perUser"` into `%LOCALAPPDATA%\Programs\ngterm`; a Start Menu shortcut whose
  `System.AppUserModel.ID` is the default app id's AppUserModelID; that HKCU key written by the MSI too, so uninstall
  removes it even though the app writes it at runtime; `bin` on the user's `PATH`.
- **Upgrades**: a fixed `UpgradeCode` per architecture, `ProductVersion` from the workspace version, `MajorUpgrade`
  with `AllowSameVersionUpgrades="yes"` (a rebuild of one version installs over itself, as a local update does).
  The Restart Manager may close `ngterm.exe`, which quits through plan 1's session-end path and is relaunched after
  (`RegisterApplicationRestart`), and `ngtermctl.exe` processes, whose long-running streams end as they do when the
  app quits (the server they talk to goes away with it). `ngterm-session.exe` runs from its private copy and is never
  a file in use, so live sessions outlive the upgrade and the relaunched app hands them to the new server.
- **Uninstall** removes the program files, the shortcut, the `PATH` entry and the default AppUserModelID key; it leaves
  the state and config dirs and the live sessions, as Windows apps do.
- **References to study in their tasks**: WiX v6's docs for per-user packages and `Files` harvesting; Windows
  Terminal's and PowerToys' (MIT) WiX sources for shortcuts with an AppUserModelID and the Restart Manager.

## Technical Details

- **Restart registration**: `RegisterApplicationRestart` with the instance's state dir and app id as explicit
  arguments (`--state-dir`, `--app-id`, which the app takes as it takes `NGTERM_STATE_DIR` and `NGTERM_APP_ID`), and
  `RESTART_NO_CRASH | RESTART_NO_HANG`, so only an update or a restart relaunches it, and on the instance it was.
- **The upgrade test's two packages**: the "old" package is built from the merge base with `main` (a worktree, cached
  by commit), the current one from the tree under test, both at the workspace version, so their session build ids
  differ and the handoff runs.
- **Artifacts**: `ngterm-<version>-x64.msi`, `ngterm-<version>-arm64.msi`.

## What Goes Where

- **Implementation Steps**: the WiX source, the build script, the app's restart registration, the install tests, CI,
  docs.
- **Post-Completion**: a hand install in the Win11 VM; signing, later and the user's to set up.

## Implementation Steps

### Task 1: The WiX source

**Files:**
- Create: `packaging/windows/ngterm.wxs`
- Create: `.config/dotnet-tools.json` (the pinned `wix` tool)

- [ ] study WiX v6's per-user packages and Windows Terminal's or PowerToys' shortcut with an AppUserModelID
- [ ] the package: per user, the install dir, the harvested stage, the Start Menu shortcut with its AppUserModelID,
      the AppUserModelID key, the `PATH` entry, `MajorUpgrade` with same-version upgrades, an `UpgradeCode` per arch
- [ ] the Restart Manager allowed to close `ngterm.exe` and `ngtermctl.exe` only
- [ ] write a test that the shortcut's AppUserModelID in the WiX source equals the one the app derives for the default
      app id (plan 2's function)
- [ ] a `wix build` of a sample stage succeeds on both runners - must pass before next task

### Task 2: The MSI build script

**Files:**
- Create: `scripts/build-msi.sh`
- Create: `scripts/tests/` cases for it (or the repo's existing script-test location)

- [ ] release build for the arch, `scripts/stage.sh` into a fresh stage, `wix build` with the version and arch
- [ ] refuses a stage missing a file the package lists, or a version that does not parse
- [ ] write tests for both refusals and for a good build's file name and version
- [ ] run it for x64 and arm64 on their runners - must pass before next task

### Task 3: The restart registration

**Files:**
- Modify: `crates/ngterm-gpui/src/main.rs` (the arguments), `crates/ngterm-gpui/src/signals/windows.rs` (the
  registration beside the session-end handling)

- [ ] `--state-dir` and `--app-id` taken as their environment variables are, the argument winning over the variable
- [ ] `RegisterApplicationRestart` at start with the instance's own state dir and app id (Technical Details)
- [ ] write tests: the registered command line read back (`GetApplicationRestartSettings`) names the private state
      dir and app id; the arguments override the environment
- [ ] run tests on both Windows runners - must pass before next task

### Task 4: Install tests

**Files:**
- Create: `packaging/windows/tests/test_msi.py`

- [ ] install with `msiexec /i … /qn`; the files, the shortcut's AppUserModelID and the `PATH` entry checked
- [ ] the installed `ngterm.exe` launched on a private state dir and app id answers `version` with `installed:` naming
      the MSI's build; quit through the Restart Manager
- [ ] an upgrade from the old package with a live session open and an `ngtermctl events` running: the session's
      program still runs after; the relaunched app came back on the private state dir and app id, not the default,
      and shows the session; the `events` stream ended and a new one connects; no reboot pending
- [ ] uninstall: nothing left in the install dir, no shortcut, no `PATH` entry, no default AppUserModelID key, the
      private app id's key removed by the test; the state dir and the live session untouched (the session then reaped
      through its namespace's server)
- [ ] run on both Windows runners - must pass before next task

### Task 5: The windows-msi CI job

**Files:**
- Create: `.github/workflows/windows-msi.yml`

- [ ] on both arches: build the old and the current package, run `test_msi.py`, upload the current MSIs as artifacts
- [ ] on pull requests that touch the packaging, the stage or the app's Windows code, and on pushes to `main` and
      `ci/**`
- [ ] measure the job once and set its timeout from it
- [ ] run it on a `ci/**` push - must pass before next task

### Task 6: README and CLAUDE.md

**Files:**
- Modify: `README.md`, `CLAUDE.md`

- [ ] README: installing on Windows (the MSI, where it installs, what an upgrade keeps and what uninstall keeps),
      building the MSI, updating a local install in the VM by building and installing the MSI there
- [ ] CLAUDE.md: only if a rule changed (the packaging is not a crate)
- [ ] run the gates - must pass before next task

### Task 7: The series' done bar

- [ ] every protocol, cli and behaviors id in the Linux ratchet is in the Windows ratchet too, or recorded here with
      its reason (a behavior Windows legitimately lacks, named in rust-build.md)

### Task 8: Verify acceptance criteria

- [ ] push to a `ci/**` branch; wait (in the background) for `conformance-windows` and `windows-msi` on both arches
- [ ] add the newly green ids with `check_ratchet.py --list conformance/rust-passing-windows.txt --add`
- [ ] `conformance/tools/run-linux.sh` once in the background; no Linux ratchet id went red
- [ ] Task 7 holds after this run

### Task 9: [Final] Update documentation

- [ ] README and CLAUDE.md match what was built
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

**Manual verification** (in the Win11 VM, started for it and stopped after):
- install from the artifact by double-click, SmartScreen's "Run anyway", the Start Menu entry, a toast from the
  installed app, `ngtermctl` in a fresh PowerShell

**External system updates**:
- signing the MSI (for example Azure Trusted Signing): the certificate and CI secrets are the user's to set up, then a
  change of its own adds the signing step to `windows-msi.yml`
