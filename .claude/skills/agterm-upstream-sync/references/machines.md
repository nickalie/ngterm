# Test machines

The Linux host builds and checks Linux only. Work for another platform runs on that platform's own machine: building,
clippy, tests, conformance runs, debugging and GUI runs of code that is Windows-, macOS-, iOS- or Android-specific
(a `cfg` branch, an OS mechanism, a mobile screen), and the checks of a change that reaches that platform. A
cross-build on the host (README's x86_64 macOS build path, say) checks nothing for that platform: never run one in
place of a check there, and don't leave such a check to CI alone. The PR's CI stays the final gate on every platform.
GUI runs follow AGENTS.md > Never touch the user's running ngterm on every machine.

| platform | machine | reached by |
|---|---|---|
| Windows | a Windows VM on the host | ssh (PowerShell) |
| macOS, iOS | a Mac with Metal, Xcode and the iOS simulators, the operator's own | ssh |
| Android | the `ngterm-phone` and `ngterm-tablet` emulators on the host (README > Android toolchain) | `adb` |

## The machine values are private

This repository is public, so it never names a machine's address, account, ssh alias, VM name, hypervisor, clone
path or anything else of the operator's setup: not in this file, a plan, a commit or a doc. Below they appear as
placeholders: `<win-ssh>` and `<mac-ssh>` (the ssh destinations), `<win-clone>` and `<mac-clone>` (the ngterm clone
on each), and the VM's state, start and stop commands. Their values are in
`.claude/skills/agterm-upstream-sync/references/machines.local.md`, a git-ignored file of the operator's (with the
machines' own quirks). Read it before using a machine; if it is missing or lacks a value, ask the user for it instead
of guessing, and write it there.

## Start on demand, stop when done

The Windows VM and the emulators are off by default and cost the host CPU and memory while up.

1. Windows: check the VM's state first. Already running means someone else (the user, another session) is using it:
   use it, and leave it running. Otherwise start it and remember that you did, then wait until `<win-ssh>` answers.
2. Android: every session starts an emulator of its own and never uses one it didn't start (another session's
   installs and its shutdown would collide with yours). `-read-only` lets several instances of one AVD run at once:
   `SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}`, then
   `"$SDK/emulator/emulator" -avd ngterm-phone -read-only -port <even port 5554-5682 no serial in adb devices uses>
   -no-window -no-audio -no-boot-anim` in the background (`ngterm-tablet` for a tablet layout), then, in the
   foreground, `timeout 300 adb -s <serial> wait-for-device shell 'while [ "$(getprop sys.boot_completed)" != 1 ];
   do sleep 2; done'`. A timeout there is a failed boot: kill your emulator, then report it or retry once.
   Its serial is `emulator-<port>`. Export `ANDROID_SERIAL=<serial>` for every `adb`, Gradle and
   `scripts/build-android.sh` command: Gradle's install and instrumented-test tasks act on every attached device
   unless it is set, the user's phone or emulator included.
3. Stop what you started as soon as the piece of work that needed it ends (the step, the plan task), failures
   included, and before you report: the VM with its stop command (a forced stop only when that hangs for minutes),
   your emulator with `adb -s <your serial> emu kill`. Never leave one you started running, and never stop one you
   didn't. A VM you started still waits while another session's worktree is there (`git worktree list` in
   `<win-clone>`, Getting the code there): it's using the VM; stop it once that worktree is gone.

The Mac is the operator's own and always on: never shut it down. It may be asleep at first contact ("No route to
host"): retry for a minute. A brief to a subagent that may need any of these machines carries this file's path.

## Getting the code there

The sync's commits stay local, so the remote clones get the code from the host, not from GitHub.

- Each run works in a worktree of its own, added to the machine's existing clone (`<win-clone>`, `<mac-clone>`) and
  removed when done; never make another clone, and never change the clone's own checkout (its branch, its files).
  Name the worktree `<clone>-wt-<name>` beside the clone, with a `<name>` no other session would pick (the branch
  plus a few random characters); its own worktree is how concurrent sessions stay apart.
- Committed work: `git bundle create <file> <base>..<branch>`, where `<base>` is a commit the clone already has;
  `scp` it over, then in the clone `git fetch <file> +<branch>:refs/agent/<name>` and
  `git worktree add --detach <worktree> refs/agent/<name>`.
- Uncommitted work: a worktree at the host's `HEAD` that way first, then `git add -N <new files>` and
  `git diff --binary HEAD > <file>` on the host (it holds staged and unstaged changes and the new files), and
  `git apply <file>` in the worktree, never in the clone's own checkout.
- Builds go to the worktree's own `target`, removed with it; never point `CARGO_TARGET_DIR` at the clone's or
  another worktree's. To skip a cold build of every dependency, seed it from the clone's `target` with timestamps
  kept, so only the worktree's own crates rebuild: `cp -cRp` on the Mac (an APFS clone, instant),
  `robocopy <clone>\target <worktree>\target /E /NFL /NDL` on Windows. libghostty-vt's prefix is shared already, in
  the vendor cache under the user's home.
- A conformance suite runs one per machine (`conformance/tools/run-windows.sh`'s header): before starting one, check
  that no other session's suite runs there, and wait if one does.
- Nothing stays behind in a clone: no branch or commit of yours, no applied patch or other change in its own
  checkout, no worktree or `refs/agent/*` ref of yours. When done, from the clone: `git worktree remove --force
  <worktree>` and `git update-ref -d refs/agent/<name>`, then check `git branch --list`, `git worktree list`,
  `git for-each-ref refs/agent` and `git status` there and clear what is yours, failures included, before you report.

## Waiting on a remote run

A run that outlasts one ssh call (a build, a test or conformance run) is started, then waited on in bounded steps: a
run that dies has to show up as a failure, never as a wait that goes on.

- A script in the run's worktree starts it and writes its log and an exit marker (the exit code) inside that
  worktree, never in the remote user's home or a shared directory, where other sessions and cleanups reach them. It
  writes the marker on every exit path, failures included (a `trap` in a shell, `try`/`finally` in PowerShell).
- Note the run's process id when it starts, and set an overall deadline from its expected duration, two to three times
  that. Past the deadline the run counts as hung: stop it and report it.
- Wait in foreground calls of under 10 minutes each, each bounded by its own `timeout` on the host and repeated until
  the marker appears or the deadline passes. Each call checks both the marker and that the run's process is still
  alive on the remote.
- A missing log, or a dead process with no marker, is a failed run: read what log there is, then report it or re-run
  it once; never keep waiting.
- Never wait in an unbounded `until`/`while … sleep` loop, in the foreground or the background: one that cannot see
  the run end spins silently for hours.

## Quirks

- Windows: OpenSSH keeps an ssh session open until every process started in it exits, so a background watcher (a
  `sleep` timer, a server) holds the call open. A long run starts outside the session through WMI:
  `Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine='powershell -NoProfile
  -ExecutionPolicy Bypass -File <worktree>\run.ps1'; CurrentDirectory='<worktree>'}` returns at once, its
  `ReturnValue` is 0 when the process started and its `ProcessId` is the run's process id, and the process outlives
  the ssh session (`Get-Process -Id <pid>` checks it from a later call). Without `-ExecutionPolicy Bypass` the script
  is refused and the process exits at once, with no log or marker. The run is then waited on as
  [Waiting on a remote run](#waiting-on-a-remote-run) says. One cargo build at a time in the VM. Its GUI runs need a
  console logon; ask the user if you lack it.
- Mac: no `timeout` (`perl -e 'alarm N; exec @ARGV' cmd` does it); wrap long work in `caffeinate -is`. Over ssh there
  is no Screen Recording grant, so no screenshots, and a locked screen gives GPUI no frames (`session not realized`):
  check the lock before reading anything into that. A dev run there follows AGENTS.md > Never touch the user's
  running ngterm, as on the host.
- Android: builds and JVM tests run on the host (`scripts/build-android.sh`); the emulator is for installing the app,
  instrumented tests and looking at a screen.
- iOS: `scripts/setup-ios.sh` and `scripts/build-ios.sh` on the Mac (README > iOS toolchain, > iOS); its header has
  the notes for a Mac reached over ssh.
