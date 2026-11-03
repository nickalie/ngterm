# Android app: a remote-only ngterm

## Overview

A native Android app that views and drives the live sessions of remote ngterm machines: `ngterm-headless` on a server,
or a desktop instance. From a phone or tablet you add a machine, list its sessions, create or end one, and attach it.
You get its panes, split layout, agent status, context, HUD and notifications; asks show as native dialogs or as
notification actions. The app runs no local sessions.

It is a new viewer of the protocol desktop's File ▸ Attach Remote already speaks: `ngtermctl live tree`, `ngtermctl
live present SESSION`, `ngterm-session attach` per pane, and the `control_command` create/end lines, all over ssh. Like
desktop's New Session/End Session on HOST, it is an extension beyond upstream, which has no mobile app, so it claims no
conformance ids.

The Rust part is a new crate, `ngterm-mobile`, exposed to Kotlin through UniFFI. The UI is Kotlin + Jetpack Compose.
It must be clean, intuitive and appealing, follow UI/UX best practice, and be built with the `frontend-design` skill.
Tablets are first-class, not a stretched phone layout. The Android project is strictly linted from its first commit.

An iOS app over the same crate may come some day. It is only a consideration: the crate avoids Android-only Rust where
that costs nothing, and the plan has no iOS targets, checks or tasks.

## Prepared and proven before this plan (2026-11-02)

Two investigations ran on the dev box. Their working diffs are saved next to this plan. Both apply cleanly on
`be924cc0`; prototyped on `5edec0d8`.

- **`20261102-android-app/task3-libghostty-vt-android.patch`:** libghostty-vt builds for Android at the pinned revision
  with **no Ghostty patch**.
  - `cargo ndk -t arm64-v8a -P 24` and `-t x86_64 -P 24` both build `ngterm-ghostty-vt --tests`, and all 103 x86_64
    test binaries passed on an x86_64 API 36.1 emulator.
  - Ghostty's `GhosttyLibVt.zig` handles Android itself (`pkg/android-ndk` writes Zig's libc file against the NDK's
    bionic sysroot), and simdutf and highway build without libc++.
  - **The API floor is 24:** Zig 0.16's std imports `preadv64`, which bionic has only from API 24. The patch refuses
    anything lower.
  - cargo-ndk doesn't export `ANDROID_NDK_HOME`, so the build script derives it from `CARGO_NDK_SYSROOT_PATH`.
  - The prefix needs no new Zig packages, so its key stays unchanged.
  - NDK 27.1.12297006 is verified (27.0 also builds). Arm64 binaries were built, not run.
- **`20261102-android-app/task2-ngterm-term-stream-backend.patch`:** `ngterm-term` gets a default-on `local` feature
  (`ngterm-pty`, `ngterm-session`, `ngterm-host` and their modules: `launch`, `pty`, `session_io`, `painted`, `bench`)
  and a fourth `Pane` backend, `Pane::stream`.
  - The gate is needed: `ngterm-session` does not compile for Android (`job/mod.rs` has no `sys` for android, and
    `libc::CLOSE_RANGE_CLOEXEC` is missing).
  - Without `local`, `ngterm-term` checks for `aarch64-linux-android` (together with the Task 3 patch), and its tree
    holds no `ngterm-session` or `portable-pty`.
  - Measured on the prototype: 200 default-feature tests and 53 `--no-default-features --lib` tests passed (6 of them
    new stream tests); clippy was clean in both feature sets, and `cargo check -p ngterm-gpui` passed.
  - The patch also adds the CI steps for the no-`local` build to `cross-platform.yml` and `linux.yml`.
- **russh 0.65** builds and links a cdylib under `cargo ndk -t arm64-v8a -P 24`, with either crypto backend. The plan
  uses `ring` (`default-features = false`): it has no C build-system dependency and is lighter on Windows CI.
- A plan review checked the plan against the code, and its verified findings are folded in below: e2e isolation,
  leadership, the `PresentationClient` threading model, Core-bound helpers, the companion filter, 1–2 pane layouts,
  and CI feature unification.

The patches are inputs for Tasks 2 and 3, not finished work. Apply one, review it against the task, finish what the
task adds, and run its checks.

## Context (from discovery)

- `crates/ngterm-core/src/remote/`:
  - `client.rs`: `PresentationClient`, sans-IO and Core-free.
    - The owner splits lines, passes launch numbers and `now`, runs `tick` about once a second (viewer.rs's `TICK`),
      and applies the `ClientEffect`s.
    - Its backoff is `retry_delay`, held per stream (`fail`/`retry_at`/`retry_now`).
    - It is `!Send`, because it holds `Box<dyn PresentationLink>`.
    - Related items: `viewer_kinds(pages)`, `STALE_AFTER`, `PAGES_UNSUPPORTED`.
  - `viewer.rs::apply_client_effect` (Core-bound) holds the semantics mobile must re-derive:
    - a `SnapshotStatus` is no change on the origin, so it doesn't notify;
    - requests it can't show get `AskRejected`, `OverlayRejected` or `PageRejected(PAGES_UNSUPPORTED)`;
    - answers echo `owner`;
    - an ask whose target isn't shown is refused (lines 599–606).
  - `layout.rs` is Core-bound and not reusable; `PresentationLayout::is_valid` allows 1–2 panes, plus a hidden split.
  - `tree.rs`: `RemoteCommandRunner`, `ssh_arguments`, `remote_cli_chain`, `REMOTE_CLI`, `validate`, `is_plain`.
  - `ssh.rs`: `present_command`, `attach_command(…, lead)`, `control_command(host, args)`, and the unused-here
    `run_job_command`, `page_serve_command` and `tunnel_command`.
  - The create/end helpers are private or crate-bound, and mobile needs them public:
    - `not_live`, `too_old`, `end_failure`, `no_answer` (private, `origin_sessions.rs`);
    - `origin_created`, `unattached`, `reply_of`, `NOT_ATTACHABLE`, `PAUSE` (`pub(super)`, `companions.rs`);
    - `read_remote_tree` (`pub(crate)`, `attach.rs`).
  - Create is three stages: check that `restore mode` is live, run `session new --no-select`, then poll the tree every
    `PAUSE` within `READY_BUDGET`.
  - The picker filters out companions with content (`content.is_none()`) and sorts parents first (`attach.rs:391-393`).
  - `respawn.rs`: `RemoteRespawnBook`, which reattaches attach-only after a parent's respawn.
  - `GONE_GUARD`: a session that is gone prints `ngterm: remote session is gone` and exits 1.
- `crates/ngterm-core/src/live/lead.rs`: `LeadAttachment` (nonce, claim, `assignments()`), `LeadNotice::parse`,
  `LeadState`. Only the leader's grid sets the pty size, and the role is reported as a reserved-prefix title.
- `crates/ngterm-term` with the Task 2 patch: `Pane::stream(&PaneConfig) -> (Pane, StreamFeed, StreamOut)`.
  - `StreamFeed::deliver` blocks above the mailbox high mark, which gives backpressure; it also has `end_of_output`
    and `exited(code, signal)`.
  - `StreamOut::next(timeout)` returns `Outgoing::Bytes` (input and replies, in order) or `Outgoing::Resize`.
  - Rendering: `frame()` returns a `Frame`, using `Frame.seq`, `Row.changed` and `row_changed_since`; `PaneConfig.wake`
    signals new output.
  - Input: `key(&KeyInput)` (`PhysicalKey` and `Mods` from `ngterm_core` binds), `mouse`, `wheel`, `paste`, `focus`.
    Also `gesture`, `select*`, `search`, and `take_events()`, which returns `PaneEvent`s.
  - A `ClipboardRead` must be answered through `answer_clipboard_read`, or output freezes.
- `crates/ngterm-ctl/src/socket.rs`: `ngtermctl` resolves its socket as `--socket`, then `NGTERM_STATE_DIR`, then the
  default.
- `scripts/stage.sh DEST [--profile]`: stages `bin/{ngterm-headless,ngtermctl,ngterm-session}` plus Ghostty's
  resources.
- The workspace lints deny `clippy::pedantic` and `allow_attributes`, so the escape hatch is an item-level
  `#[expect(…, reason)]`. `members = ["crates/*"]`, so workspace-wide `cargo test` includes new crates.

## Development Approach

- **Testing approach:** TDD for the Rust work: tests first, seen failing, then the code. Code first for the Compose UI:
  design with `frontend-design`, build, then cover with UI and screenshot tests in the same task.
- Complete each task fully before the next, in small, focused changes.
- **CRITICAL: every task includes new or updated tests** for its code, covering success and error paths.
- **CRITICAL: all tests pass before the next task starts.**
- **CRITICAL: update this plan when scope changes during implementation.**
- Desktop behavior stays unchanged: the Rust refactors are pinned by desktop's existing tests and by new ones.
- From Task 11 on, every task that touches `mobile/android` leaves `./gradlew check` green. A lint finding is fixed in
  the code, never in a baseline file or a blanket disable.
- Checks are sized to the change (CLAUDE.md > Choosing checks): `cargo test -p` on the touched crates, the Gradle tasks
  for `mobile/android`, and CI for the other OSes.
- **Never touch the user's running ngterm.** Strip `NGTERM_*` from everything launched. The e2e test forces a private
  `NGTERM_STATE_DIR`, `HOME` and `PATH` on the remote side (Task 10). Reap sessions through that state dir's own
  server.
- **Machine-independent:** the plan runs on a different machine from where it was prepared. Task 1 makes the toolchain
  reproducible, and no task assumes this dev box's paths, mise shims or installed SDK.

## UI/UX standard (applies to every UI task)

- **Before writing any Compose UI, invoke the `frontend-design` skill.** Task 12 sets the aesthetic direction once and
  records it in `mobile/android/DESIGN.md` (direction, tokens, type scale, spacing, motion, component inventory). Every
  later UI task reads `DESIGN.md`, invokes `frontend-design` again for its screen, and stays inside the system, or
  extends `DESIGN.md` on purpose.
- Material 3 as the base, given a distinctive identity of its own rather than stock defaults. ngterm's palette is the
  default, and dynamic color is optional.
- **Best practice, checked in review:**
  - touch targets at least 48dp;
  - WCAG AA contrast for text and status colors in light and dark;
  - every screen has empty, loading, error and offline states, with a clear next action;
  - destructive actions confirm (End Session), and safe ones don't nag (Detach);
  - visible feedback within 100 ms of a tap;
  - TalkBack labels, and status never conveyed by color alone;
  - respect font scale and reduced motion;
  - edge-to-edge with insets handled, and predictive back.
- **Adaptive by window size class:**
  - compact (phone): stacked navigation, one pane full screen;
  - medium: a collapsible rail;
  - expanded (tablet, landscape, unfolded): a permanent sidebar and the real split layout.

  Multi-window, freeform resizing, folding and rotation never lose state.
- The terminal is the hero: chrome stays minimal and gets out of the way while typing.

## Linting (from the first commit of `mobile/android`)

The tools are the standard ones. There are no baseline files, and no suppression without a reason.

- **Kotlin compiler:** `allWarningsAsErrors = true`, with `progressiveMode` on.
- **Formatting:** ktlint through Spotless, with the `ktlint_official` code style and `.editorconfig` as the single
  source of truth. `spotlessCheck` is part of `check`.
- **Static analysis:** detekt with its default ruleset plus the Compose rules (`io.nlopez.compose.rules:detekt`),
  `maxIssues: 0`, and its formatting rules off (Spotless owns formatting). A suppression is `@Suppress("Rule")` on the
  item, with a comment saying why.
- **Android Lint:** `warningsAsErrors = true`, `abortOnError = true`, `checkDependencies = true`, no
  `lint-baseline.xml`, and the accessibility and Compose checks on.
- **Dependencies:** every version lives in `gradle/libs.versions.toml`, with no dynamic versions.
- **Rust:** the workspace's strict clippy with the `clippy.toml` limits, plus `cargo fmt`. UniFFI's owned-argument
  exports trigger `needless_pass_by_value`, so they stay in one thin `api` facade that uses item-level
  `#[expect(…, reason)]`.

## Testing Strategy

- **Rust unit tests** (`cargo test -p ngterm-mobile`, run on Linux, macOS and Windows hosts in CI):
  - the remote-command builders;
  - transport over an in-process fake (scripted `live tree`, present output and pane channels);
  - one test per `ClientEffect` variant for the effect → event mapping;
  - reconnect on a fake clock, and suspend/resume;
  - the never-claim-pages rule;
  - leadership;
  - the channel-end cases;
  - clipboard reads;
  - the packed row buffer.
- **Rust e2e** (Linux; `#[ignore]`, so plain workspace runs skip it, run by `scripts/test-mobile-e2e.sh` and by CI): a
  staged `ngterm-headless` behind an unprivileged `sshd` whose `ForceCommand` forces the private environment. It covers
  isolation first, then create → attach → read → leadership with two attaches → drop → reattach → end. On CI a missing
  `sshd` fails the run instead of skipping.
- **Android:**
  - JVM unit tests for the view models;
  - Roborazzi screenshot tests (Robolectric, as Now in Android uses) for each screen at phone, medium and expanded
    widths, light and dark, and at font scale 1.0 and 2.0;
  - Compose UI tests on an emulator against a fake core binding;
  - `./gradlew check` for all lint.
- No conformance ids: upstream has no mobile app.

## Progress Tracking

- Mark completed items `[x]` immediately.
- Add newly found tasks with ➕, and blockers with ⚠️.
- Keep the plan in sync with the work done.

## Solution Overview

```
Android app (Kotlin, Compose)                 ngterm-mobile (Rust, UniFFI)              server
┌───────────────────────────────┐   calls/   ┌──────────────────────────────┐ russh ┌───────────────────┐
│ UI: machines, sessions,       │   events   │ actor thread (tokio, LocalSet)│──────▶│ ngtermctl live    │
│ adaptive layout, TerminalView │◀──────────▶│  ├ Machine: one ssh conn     │ exec  │  tree / present   │
│ LinkService (foreground)      │            │  ├ PresentationClient/session│ chans │ ngterm-session    │
│ Notifications, Keystore       │            │  └ Pane: Pane::stream + feed/│       │  attach (per pane)│
│ Keyboard + extra-keys row     │            │      out threads, row buffer │       │ ngterm-headless   │
└───────────────────────────────┘            └──────────────────────────────┘       └───────────────────┘
```

- **One russh connection per machine**, with exec channels:
  - tree and control (`restore mode`, `session new --no-select`, `session close --target`);
  - one `live present` per attached session, with no pty;
  - one `ngterm-session attach` per shown pane, with a pty sized by the view, through `attach_command`'s words with a
    `LeadAttachment`.
- **Threading model:**
  - One actor thread per `MobileCore` runs a current-thread tokio runtime and a `LocalSet`. It owns the russh
    connections, the `!Send` `PresentationClient`s and a 1 s tick.
  - UniFFI objects are thin `Send + Sync` handles that post commands to the actor.
  - Each pane has a feeder thread (channel bytes → `StreamFeed::deliver`, which may block for backpressure) and a
    writer thread (`StreamOut::next` → the channel, including resizes).
  - Events go to Kotlin through the `EventSink` callback, off the UI thread.
- **Reuse:**
  - Core builders produce the remote command strings (Task 4), and core's reply parsers and error strings become public
    and Core-free (Task 5).
  - `PresentationClient`, `frames`, `retry_delay`, `LeadAttachment`/`LeadNotice`/`LeadState` and
    `RemoteRespawnBook`'s rule are used as they are.
  - The terminal is `ngterm-term`'s `Pane::stream` (Task 2).
  - Input uses `ngterm-term`'s `KeyInput`, `PhysicalKey`, `MouseInput` and `WheelInput`, with no new key type.
- **Presentation:**
  - The hello uses `viewer_kinds(false)`. Overlays and pages are refused with `OverlayRejected` and
    `PageRejected(PAGES_UNSUPPORTED)`.
  - The presenter role follows the hub's rules, and answers echo `owner`.
  - The ask rule is a deliberate extension: an ask for any attached session is accepted. It shows as a dialog when that
    session is on screen, and otherwise as a notification (button asks as actions, input asks as inline reply, choice
    asks open the app). A backgrounded app is a valid place to answer. Asks for a session that isn't attached are
    refused, as desktop refuses them.
- **Reconnect:**
  - The machine connection retries on `retry_delay`, and `network_changed()` retries at once.
  - While the connection is down, the presentation adapter's `open` returns a link that closes immediately, so each
    `PresentationClient` keeps its own backoff. On reconnect, every client gets `retry_now`.
  - Panes reattach into a fresh `Pane::stream` with `LeadAttachment::new(false)`. A fresh attach's snapshot isn't
    preceded by a reset, so a stale pane is never reused.
- **Channel end:**
  - *Link lost* (transport error, no exit status): cover the pane and reattach on reconnect.
  - *Program exited* (exit status): `end_of_output` and then `exited(code, signal)`. A respawn counted from the origin's
    `Respawns` reattaches attach-only.
  - *Session gone* (`GONE_GUARD`'s text, confirmed through the tree): final, with no retry, and the row leaves.
- **Leadership:** `PaneEvent::Title` is filtered through `LeadNotice::parse` and `LeadState`, so reserved titles never
  reach the UI. A follower pane gets a cover with **Take control**, which reattaches with `LeadAttachment::new(true)`.
- **Clipboard reads (OSC 52):** each read gets an Allow / Deny prompt, Ghostty's default `clipboard-read = ask`, with
  "always for this machine" stored per machine. Detach, drop and exit deny any read still open.
- **Kitty images:** `kitty_image_limit = 0` in v1. The attach client's probe then reports no support, and nothing
  hangs.
- **Auth:**
  - Rust generates ed25519 keys, returns the OpenSSH private bytes, and parses imported keys with their passphrase.
  - Kotlin wraps the key bytes with an Android Keystore AES-GCM key and stores them.
  - `KeyVault.load(key_id)` hands the bytes to Rust for a connection; Rust zeroizes them after use.
  - Host keys are pinned on first use (TOFU) through `HostKeyStore`. A mismatch stops the connection with both
    fingerprints shown.
- **Platform seams** (UniFFI callback interfaces): only `KeyVault`, `HostKeyStore` and `EventSink`. Notifications,
  asks and clipboard are events Kotlin acts on; clipboard answers and ask answers go back through the API.
- **Android floor:** `minSdk 26`, `targetSdk 36`, `compileSdk 37` (the current AndroidX libraries need 37 to compile against; Task 11), and cargo-ndk `-P 26`, which is ≥ the libghostty-vt
  floor of 24.
- **Foreground service:** type `specialUse`, with a `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` explanation ("keeps the ssh
  sessions of attached remote terminals connected"). `dataSync` is time-limited on Android 15.

## Technical Details

- **Remote commands** (Task 4):
  - `remote::ssh` gains `remote_tail(...) -> Result<String, InvocationError>` for tree, present, attach and control:
    the exact string desktop places last in its argv.
  - Desktop's argv functions become `ssh_arguments(host, …)` + that tail, byte-identical to today.
  - `run_job_command`, `page_serve_command` and `tunnel_command` are untouched.
- **Public origin replies** (Task 5): a new Core-free `remote::origin_replies` module holds the create/end/tree reply
  parsing, the error strings and the create stages' constants (`PAUSE`, `READY_BUDGET`). The `Core` flows call it.
- **Mobile transport** (`ngterm-mobile`, internal): `Transport::exec(command, pty: Option<GridSize>) -> Channel`,
  where a `Channel` reads, writes, resizes, sends EOF, closes, and reports how it ended (`ExitStatus(code, signal)` or
  `Lost(reason)`). The russh implementation keeps one connection per machine, with keepalive and a connect timeout.
  The fake implementation scripts the channels for tests.
- **Machine record** (stored by the app, passed in): `id`, `label`, `host`, `port`, `user`, `key_id`, and an optional
  `pinned_host_key`.
- **Events to Kotlin** (`EventSink`):
  - `MachineState(connected | reconnecting{next_in_ms} | unreachable{reason})`
  - `Tree(machine, sessions)`
  - `SessionConnection(session, state)`
  - `SessionStatus`, `SessionContext`, `SessionLayout`, `SessionMode(presenter | mirror)`
  - `Hud(Option)`, `Notify`, `Sound`
  - `Ask`, `AskDismiss`
  - `PaneDirty(pane)`, `PaneTitle`, `PaneLead(role)`, `PaneEnded(kind)`
  - `Bell`, `ClipboardWrite`, `ClipboardReadRequest(pane, id)`
  - `HostKeyUnknown(fingerprint)`, `HostKeyChanged(old, new)`
- **Row buffer:** `pane.frame(since_seq) -> Vec<u8>`, a versioned, little-endian packed record of the changed rows (by
  `row_changed_since`).
  - Per row: runs of UTF-8 text, fg/bg RGBA, and attribute bits.
  - Then the cursor (position, shape, visibility), the selection ranges, the grid size and the new `seq`.
  - A golden fixture keeps the Rust writer and the Kotlin reader in step.
- **Build:**
  - `scripts/build-android.sh` builds libghostty-vt (the prefix), then runs `cargo ndk -t arm64-v8a -t x86_64 -P 26
    build -p ngterm-mobile --release`.
  - UniFFI's bindgen is an in-crate `[[bin]] uniffi-bindgen` (`uniffi/cli`), run in library mode on the built `.so`.
    It writes Kotlin into `mobile/android/core/build/generated/`, which is never committed.
  - Gradle packs the `.so` files into `jniLibs`. The app depends on JNA (`net.java.dev.jna:jna@aar`).

## What Goes Where

- **Implementation Steps:** code, tests, scripts, CI and docs in this repo.
- **Post-Completion:** manual checks on real devices, and store publishing.

## Implementation Steps

### Task 1: Reproducible Android toolchain

**Files:**
- Create: `scripts/setup-android.sh`
- Modify: `build/pins.env` (`ANDROID_NDK_VERSION`, `ANDROID_PLATFORM_VERSION`, `ANDROID_BUILD_TOOLS_VERSION`,
  `ANDROID_EMULATOR_IMAGE`, `CARGO_NDK_VERSION`; no `LIBGHOSTTY_VT_` prefix, so the vt prefix key doesn't move)
- Modify: `README.md` (a short "Android toolchain" pointer)

- [x] write `scripts/setup-android.sh`, idempotent, that reports what it did and what it couldn't do:
  - check for JDK 21 (`JAVA_HOME` or PATH) and print install advice if it's missing;
  - find or create an SDK root (`ANDROID_HOME`, else `~/Android/Sdk`); fetch the command-line tools if absent; accept
    the licenses;
  - install the pinned platform, build-tools, NDK, platform-tools, emulator and system image through `sdkmanager`;
  - create an AVD for a phone and one for a tablet, if absent;
  - `rustup target add aarch64-linux-android x86_64-linux-android`;
  - `cargo install --locked cargo-ndk --version $CARGO_NDK_VERSION` when the pinned version is missing;
  - check for `sshd` (for the e2e test) and print the package name when it's missing;
  - print the `ANDROID_HOME` and `ANDROID_NDK_HOME` lines to export
- [x] pin NDK 27.1.12297006 (the version the investigation verified), platform 36, build-tools 36.0.0, an x86_64
      google_apis system image for API 36, and cargo-ndk 4.1.2
- [x] check it with shellcheck, run it twice on the machine (the second run must change nothing), and record the
      result here
  - result (2026-11-03, CachyOS x86_64): shellcheck clean; the first run (cmdline-tools 16111833, build-tools 36.0.0
    and platform 36 already present from an interrupted earlier run) installed the NDK, platform-tools, emulator and
    system image, created the `ngterm-phone` and `ngterm-tablet` AVDs, added both Rust targets and installed cargo-ndk
    4.1.2; the second run reported "nothing to do" and exited 0. JDK 21 (`jdk21-openjdk`) was already installed; sshd
    is present. sdkmanager prints a deprecation notice (Android CLI) on stderr: harmless, it still installs

### Task 2: ngterm-term's stream backend and the `local` feature

**Files:**
- Apply: `docs/plans/20261102-android-app/task2-ngterm-term-stream-backend.patch`
- Modify (by the patch): `crates/ngterm-term/{Cargo.toml,src/lib.rs,src/pane.rs,src/mailbox.rs,src/effects.rs,
  src/input.rs,src/pty.rs,tests/*.rs}`, `crates/ngterm-pty/src/…` (`From<Arc<AtomicBool>> for ReadPause`),
  `.github/workflows/{cross-platform.yml,linux.yml}`
- Create (by the patch): `crates/ngterm-term/src/{outlet.rs,stream.rs,stream/tests.rs}`

- [x] apply the patch with `git apply --3way`, and resolve any drift against main
- [x] read the stream tests and add the missing ones first:
  - a reply flood is capped as the actor's `REPLY_CAP` caps it, and the cap is implemented;
  - `exited` while output is still queued;
  - deliver after drop returns `false`
- [x] fix the rustdoc links that break without `local` (cfg-gate them or reword), and keep the
      `#[cfg(test)]` and `#[cfg(feature = "local")]` attributes separate (the clippy `allow-unwrap-in-tests` caveat)
- [x] update `ngterm-term`'s root docs: the four backends and the `local` feature
- [x] run `cargo test -p ngterm-term`, `cargo test -p ngterm-term --no-default-features --lib`, clippy in both feature
      sets, and `cargo check -p ngterm-gpui -p ngterm-headless`; all must pass before Task 3

### Task 3: libghostty-vt for Android

**Files:**
- Apply: `docs/plans/20261102-android-app/task3-libghostty-vt-android.patch`
- Modify (by the patch): `crates/ngterm-ghostty-vt-sys/{zig_build.rs,build.rs,tests/zig_build.rs}`,
  `scripts/build-ghostty-vt.sh`

- [x] apply the patch with `git apply --3way`, and resolve any drift
- [x] finish what the investigation left:
  - wrap `build-ghostty-vt.sh`'s over-long line;
  - update the doc comment above `RERUN_ENV` and the module docs, which still say the map has no Android;
  - run the script end to end on a fresh prefix directory, to prove the configure step with the empty
    `ANDROID_NDK_HOME`
- [x] tests (the patch has them, so run and extend): `cargo test -p ngterm-ghostty-vt-sys --test zig_build`, which
      includes the API-floor refusal and the NDK derivation from `CARGO_NDK_SYSROOT_PATH`
- [x] run `cargo ndk -t arm64-v8a -t x86_64 -P 26 build -p ngterm-ghostty-vt --tests` and
      `cargo ndk -t x86_64 -P 26 check -p ngterm-term --no-default-features`, then clippy on the sys crate; all must
      pass before Task 4
  - result (2026-11-03): the patch applied cleanly; the script prepared a fresh prefix under a private `NGTERM_VENDOR`
    (key `7fb8d1040daf5f9b`, unchanged); the 14 `zig_build` tests passed against it; both cargo-ndk builds passed, the
    `ngterm-term` check with `ANDROID_NDK_HOME` unset (the NDK derived from `CARGO_NDK_SYSROOT_PATH`); clippy and
    shellcheck clean

### Task 4: Remote command tails in core

**Files:**
- Modify: `crates/ngterm-core/src/remote/ssh.rs`, `crates/ngterm-core/src/remote/tree.rs`

- [x] write tests first: `remote_tail` for tree, present, attach with and without a `LeadAttachment`, and control
      (`restore mode`, `session new --no-select`, `session close --target ID`). They cover quoting, the PATH chain and
      the attach contract's cleared variables. Each desktop argv must equal `ssh_arguments` + its tail,
      byte-identical to today's (a snapshot taken before the change).
- [x] add the public tail builders and rebuild `present_command`, `attach_command`, `control_command` and the tree line
      on them; leave `run_job_command`, `page_serve_command` and `tunnel_command` unchanged
- [x] run `cargo test -p ngterm-core remote` and clippy; both must pass before Task 5

### Task 5: Core-free origin replies

**Files:**
- Create: `crates/ngterm-core/src/remote/origin_replies.rs`
- Modify: `crates/ngterm-core/src/remote/{origin_sessions.rs,companions.rs,attach.rs,mod.rs}`

- [x] write tests first, ported from those that cover these helpers today and extended:
  - `not_live`, `too_old`, `end_failure`, `no_answer`;
  - `origin_created`, `unattached`, `reply_of`, `NOT_ATTACHABLE`;
  - tree parsing (`read_remote_tree`'s core);
  - the picker filter (`content.is_none()`, parents first)
- [x] move them into the public `origin_replies` module with `PAUSE` and `READY_BUDGET`; the `Core` flows call it, with
      behavior and strings unchanged
- [x] add one pure function per create stage (restore-mode check, `session new` reply, poll until listed), so mobile
      and `Core` share the decisions
- [x] run `cargo test -p ngterm-core remote` and clippy; both must pass before Task 6

### Task 6: ngterm-mobile scaffold: crate, facade, transport traits and fake

**Files:**
- Create: `crates/ngterm-mobile/{Cargo.toml,src/lib.rs,src/api.rs,src/actor.rs,src/transport/mod.rs,
  src/transport/fake.rs,src/bin/uniffi-bindgen.rs}`
- Modify: `Cargo.toml` (workspace dependencies `uniffi`, `russh` with `default-features = false` and features
  `ring`/`flate2`/`rsa`, `tokio`, `zeroize`)

- [x] write tests first: the actor answers a command and delivers an event through a test `EventSink` off the caller's
      thread; dropping `MobileCore` joins the actor
- [x] create the crate with `ngterm-term` using `default-features = false`. Its dependencies must build on Linux, macOS
      and Windows hosts (it is an `any` crate) and for Android.
- [x] write the UniFFI facade in `api.rs` (proc-macro form): `MobileCore`, `Machine`, `Pane`, the event enum, and the
      callback interfaces `KeyVault`, `HostKeyStore` and `EventSink`, with item-level `#[expect]`s for the
      FFI-shaped signatures
- [x] write the actor (current-thread runtime + `LocalSet`), the `Transport` and `Channel` traits, and the scripted
      fake transport
- [x] run `cargo test -p ngterm-mobile`, clippy, and `cargo ndk -t x86_64 -P 26 build -p ngterm-mobile`; all must pass
      before Task 7
  - Notes: `Channel` is one concrete pair of queues (`ChannelReader`, `ChannelWriter`, the transport's `ChannelPeer`)
    rather than a trait; `Transport::connect` gives a per-machine `Connection` whose `exec` opens it. `MobileCore` has
    only the Rust `with_transport` constructor until Task 7 adds the UniFFI one over russh. `MobileEvent` holds the
    machine-level variants; later tasks add theirs. `uniffi-bindgen` needs the crate's `bindgen` feature, and the fake
    is behind `fakes`. `ngterm-term` is a path dependency (a member can't drop the workspace entry's defaults).

### Task 7: russh transport, keys and host-key pinning

**Files:**
- Create: `crates/ngterm-mobile/src/transport/russh.rs`, `src/keys.rs`

- [x] write tests first, against an in-process russh server inside the test:
  - connect with a key loaded through a test `KeyVault`;
  - an unknown host emits `HostKeyUnknown`, the connection proceeds once `HostKeyStore` pins the key, a later
    connection with the same key passes, and a changed key fails with `HostKeyChanged` carrying both fingerprints;
  - exec without a pty delivers bytes in order and then EOF;
  - exec with a pty: the size arrives and a resize is delivered;
  - exit status vs a lost link;
  - a dropped connection ends every channel as `Lost`
- [x] `keys.rs`: generate an ed25519 key (returns the OpenSSH private bytes plus the `authorized_keys` line), parse an
      imported key with its passphrase, zeroize after use
- [x] implement the russh `Transport`: one connection per machine, keepalive, connect timeout
- [x] run the tests and clippy; both must pass before Task 8
  - Notes: an unknown host key keeps the connect waiting (up to `Timing::confirm`, 120 s) for the new
    `MobileCore::answer_host_key(machine_id, fingerprint, trust)`, posted through the actor to
    `Transport::answer_host_key` (a no-op on the fake); trusting the offered fingerprint calls `HostKeyStore::pin` and
    the handshake goes on, while a refusal, another fingerprint, the timeout or a dropped connect ends it.
    `HostKeyStore::pinned` is the only authority on the pin (`Machine.pinned_host_key` is the app's display copy), so
    Task 13's Remove pinned key can't be undone by a stale record. The UniFFI constructor is `MobileCore::new(sink,
    vault, hosts)`; `generate_key(comment)` and `import_key(bytes, passphrase)` are exported functions returning a
    `KeyPair` (unencrypted OpenSSH private bytes, `authorized_keys` line, fingerprint). A channel the core closes ends
    as `Lost(CHANNEL_CLOSED)` (russh drops the server's answering close), one whose connection the core closed as
    `Lost` with `TransportError::Closed`'s text, and one whose link dropped as `Lost(CONNECTION_LOST)`; a channel with
    no reader and no writer left is closed. The actor's runtime now enables time and IO.

### Task 8: Sessions: tree, create and end

**Files:**
- Create: `crates/ngterm-mobile/src/sessions.rs`

- [x] write tests first, on the fake transport:
  - the tree parses, with companions with content filtered out and parents first;
  - errors from the remote and a missing `ngtermctl` surface through Task 5's strings;
  - create covers all three stages (not live, then `session new`, then poll until listed or the budget runs out), with
    and without a name;
  - end succeeds, and end failures map to their errors
- [x] decide which reused strings carry desktop CLI advice (for example "`ngtermctl live attach HOST ID`"); map those to
      phone wording in the event, and record the choice here
- [x] implement them over Task 4's tails and Task 5's replies
- [x] run the tests; they must pass before Task 9
  - Notes: the CLI-advice choice: only two reused strings carry it, both "…; `ngtermctl live attach HOST ID` attaches
    it" (`origin_created`'s failed-after-receipt and `unattached`, i.e. created but never attachable in time).
    `sessions::phone_wording` replaces that advice with "attach it from the session list once it shows there"
    (`ATTACH_FROM_LIST`); every other reused string names no CLI and keeps desktop's words. Messages name the machine by
    its label (its host when the label is blank). The API is `MobileCore::list_sessions(machine_id)`,
    `create_session(machine_id, name)` and `end_session(machine_id, session_id)`, each needing a connection
    (`MobileError::NotConnected`) and failing with `MobileError::Remote { reason }`; a listing (list, create) emits
    `MobileEvent::Tree` with the picker's sessions (`RemoteSession`) unless stale by then (its connection closed or
    replaced, or a later listing or an end landed: the actor orders them by tickets), and an end emits the kept listing
    without the session when one is kept; the actor keeps the whole last listing per machine for attaching. A
    name goes as `--name=NAME`. A control line runs within desktop's 30 s `COMPANION_DEADLINE`, a listing within
    `TREE_DEADLINE`; a command that can't start fails with the transport's text, and a create whose listing can't start
    (the connection closed) gives up at once. The fake gained `script_sequence` for successive answers.

### Task 9a: Presentation stream and effects

**Files:**
- Create: `crates/ngterm-mobile/src/presentation.rs`

- [x] write tests first, one per `ClientEffect` variant, mapped to its event:
  - `SnapshotStatus` emits no notification;
  - an ask is accepted under the ask rule, and its answer echoes `owner`;
  - an overlay is refused with `OverlayRejected`, and a page with `PageRejected(PAGES_UNSUPPORTED)`;
  - the hello lists `viewer_kinds(false)`;
  - pings are answered, and a quiet stream reconnects after `STALE_AFTER`
- [x] implement the `PresentationTransport`/`PresentationLink` adapter over the mobile transport on the actor: line
      splitting, launch numbers, the 1 s tick
- [x] run the tests; they must pass before Task 9b
  - Notes: `MobileCore::attach_session(machine_id, session_id)` (needs the connection and a kept listing holding the
    session, `MobileError::NotListed` otherwise; the listing's `presentation` version decides, `None` reporting
    `StreamState::Unsupported`), `detach_session` (ends the stream, reports nothing) and `answer_ask(machine_id,
    session_id, ask_id, AskReply)`: the core keeps each open ask's `owner` and echoes it, and drops an answer to an ask
    no longer open. Events: `SessionConnection(StreamState)`, `SessionStatus { status, changed }` (`changed` false for
    a snapshot's status and for the clearing below, so the app notifies nothing), `SessionContext`, `SessionLayout`,
    `SessionMode(PresentationRole)`, `Hud`, `Notify(RemoteNotice)`, `Sound(name)`, `Ask(RemoteAsk)`, `AskDismiss`;
    wire enumerations travel as their wire names and panes as their identities. As desktop's row does, leaving
    `connected` dismisses the open asks and clears context, status and HUD. Respawn counts and warnings make no event
    yet (9c takes the respawns). The adapter (`ChannelLinks`) runs each launch as a local task on the machine's
    connection as it was at the attach: lines sent before the channel opens wait for it, a line over
    `MAX_FRAME_BYTES` ends the stream, a trailing partial line is dropped, and an end reads as desktop's bridge words
    it (`exit N`, `signal NAME`, or the lost link's reason). Closing a machine's connection (disconnect, or a connect
    as edited) ends its sessions' streams for now; 9b keeps them across a lost connection. `MobileEvent` is no longer
    `Eq` (records carry `f64`s).


### Task 9b: Reconnect and lifecycle

**Files:**
- Create: `crates/ngterm-mobile/src/reconnect.rs`
- Modify: `crates/ngterm-mobile/src/actor.rs`

- [x] write tests first, on a fake clock:
  - the machine connection retries on `retry_delay`;
  - `network_changed` retries at once;
  - while the connection is down, opened links close at once and each client keeps its own backoff;
  - on reconnect, every client gets `retry_now`;
  - `suspend` closes the links with no error state, and `resume` reattaches everything
- [x] implement them
- [x] run the tests; they must pass before Task 9c
  - Notes: `Connection` gained `lost()` (resolves once the connection is gone, closed or lost; russh's resolves when
    its session's `Client` handler drops, the fake's on `close` or the new `FakeTransport::drop_connections`). The
    actor watches each connection it holds; one lost while still the machine's becomes a reconnect: a connect attempt
    with a `reconnect::Retry` that waits `retry_delay(failures)` first (1 s after the loss, doubling), each wait
    reported as `MachineState::Reconnecting { next_in_ms }`, a failure scheduling the next. A connect the app asks for
    still fails for good with `Unreachable`; only a machine that was connected keeps retrying, on any error but a
    refusal: the transport fails a changed host key, an unknown key not trusted and a client key the machine does not
    accept as `TransportError::Refused`, which ends a reconnect as `Unreachable` too. The machine's listing and attached
    streams stay across the loss: `ChannelLinks` holds the connection as an `Option`, and a launch while it is `None`
    reports `CONNECTION_DOWN` at once, so each client fails on its own backoff; `Presentations::machine_up` starts
    each client again if stopped, gives it `retry_now` and ticks it at once. New API: `MobileCore::network_changed()`
    (every reconnect waiting out its delay retries at once, its backoff over; a `connect` of a machine waiting to
    reconnect does the same and joins), `suspend()` (closes every connection, cancels connects in flight with
    `Cancelled`, stops the streams without a word, and sets each machine aside) and `resume()` (reconnects each one that
    had been connected at once on the reconnect schedule, and connects one whose first connect it cancelled as that
    connect again, failing for good; streams start again once their machine is up; a connect of a set-aside machine
    does the same). A disconnect or a connect as edited still ends the machine's streams, retries and suspension.

### Task 9c: Remote pane: stream, leadership, channel end, clipboard

**Files:**
- Create: `crates/ngterm-mobile/src/pane.rs`

- [x] write tests first, on the fake transport:
  - bytes reach the `Pane::stream`, and replies and input go back in order;
  - resize reaches the channel;
  - the `LeadAttachment` words are in the attach tail; reserved titles become `PaneLead` and never `PaneTitle`;
    `take_lead` reattaches with claim;
  - each channel end: lost → covered → reattached into a fresh pane after reconnect; exited → `PaneEnded(exit)`;
    respawn → reattach attach-only; gone → final;
  - a clipboard read raises `ClipboardReadRequest`, and both Allow and Deny unfreeze the output; detach denies any read
    still open;
  - `kitty_image_limit` is 0
- [x] implement `Pane` with its feeder and writer threads, and its key, mouse, wheel, paste and selection API over
      `ngterm-term`'s types
- [x] run the tests; they must pass before Task 9d
  - Notes: `MobileCore::show_pane(machine_id, session_id, pane_id, PaneSize) -> Pane` (the pane's identity as the
    layout names it; needs the connection and a kept listing whose session lists the pane's daemon, not an attached
    stream), `hide_pane`, `take_lead` and `answer_clipboard_read(…, read_id, ClipboardReply)`; `detach_session` and
    `disconnect` hide the session's or machine's panes. The first attach and every automatic one claim nothing
    (`LeadAttachment::new(false)`); only Take control claims. The `Pane` handle (`api::pane`) calls the terminal
    directly on the caller's thread (key, type, mouse, wheel, paste, focus, resize, select*, selection text) over
    `PaneKey`/`PaneMouse`/`PaneWheel` records that become `ngterm-term`'s `KeyInput`/`MouseInput`/`WheelInput` (the key
    named as Ghostty names the physical key, modifiers as `Mods` bits); every reattach swaps a fresh terminal into the
    same handle and reports `PaneAttached` (redraw from scratch). Pane events carry a `PaneRef` (machine, session,
    pane): `PaneAttached`, `PaneDirty` (per wake), `PaneTitle`, `PaneLead { role, covered }`, `PaneEnded(PaneEnd::
    Lost | Exited { code, signal } | Gone)`, `Bell`, `ClipboardWrite`, `ClipboardReadRequest { read_id, list_only }`
    and `ClipboardReadDismiss`. Read ids are the core's own (a fresh terminal restarts the terminal's), and a read
    still open when its pane is hidden, loses its link or exits is denied and dismissed. A link lost while the
    connection stays up attaches again on `retry_delay` (reset by a role report); one lost with its connection waits
    for `machine_up`. A gone-guard exit (core's new `ssh::SESSION_GONE` line, then exit 1) reads a fresh listing,
    kept and reported as any other: no longer listed is `Gone`, otherwise (or with no listing) a plain exit 1.
    Respawn counts reach the panes from `Presentations::take_respawns` through `RemoteRespawnBook`. "Always allow
    for this machine" is the app's to store: it answers a request at once without asking. A stopped writer sends what
    is still queued (a denied read's reply) and closes the channel once the queue is quiet for `WRITER_POLL`.

### Task 9d: Packed row buffer

**Files:**
- Create: `crates/ngterm-mobile/src/rows.rs`, `crates/ngterm-mobile/tests/fixtures/rows-golden.bin`

- [x] write tests first: feed known VT bytes and decode the buffer (text, colors, attributes, wide characters, cursor
      shapes, selection); only changed rows are sent since a `seq`; the golden fixture's layout is checked
- [x] implement the writer, and document the layout in the module docs (the Kotlin reader in Task 15 follows it)
- [x] run the tests; they must pass before Task 10
  - Notes: `Pane::frame(since_seq) -> Result<Vec<u8>, PaneInputError>` (`Closed` with no terminal) packs with
    `rows::pack`; the layout (v1, magic `NGRB`) is in `rows`' module docs. The grid, seq, default colors and the cursor
    are in a fixed 40-byte header; rows follow, then the selection of every row it crosses. Colors are `0xAARRGGBB`;
    a run's default color is `0`, resolved from the header of the frame being drawn (OSC 10/11 resends no rows); runs
    split at a width change, a multi-code-point cluster is a run of its own, and the run kind is flag bits
    (`RUN_WIDE | RUN_CLUSTER` is a wide cluster);
    blank cells after a row's last drawn cell are left out. `since` 0 or past the frame's seq sends every row
    (`FRAME_FULL`); after `PaneAttached` the reader passes 0. `UPDATE_GOLDEN=1` rewrites the golden fixture.

### Task 10: End-to-end test against a real ngterm-headless

**Files:**
- Create: `crates/ngterm-mobile/tests/headless_e2e.rs`, `crates/ngterm-mobile/tests/support/sshd.rs`,
  `scripts/test-mobile-e2e.sh` (➕ `tests/support/{machine,phone,steps}.rs`)

- [x] write `scripts/test-mobile-e2e.sh`:
  - build `ngterm-headless`, `ngtermctl` and `ngterm-session`;
  - `scripts/stage.sh` them into a temporary stage;
  - run the test with `--ignored`, passing the stage path, with `NGTERM_*` stripped
- [x] in the test, start `ngterm-headless` from the stage in a private state dir (`NGTERM_STATE_DIR` and `HOME` under
      a temp dir, with the session server through that dir)
- [x] start an unprivileged `sshd` on a random port with its own host key, `authorized_keys` and config. Its
      `ForceCommand` wrapper exports the private `NGTERM_STATE_DIR` and `HOME` and `PATH=<stage>/bin:/usr/bin:/bin`,
      then runs `exec /bin/sh -c "$SSH_ORIGINAL_COMMAND"`.
- [x] **assert isolation first:** create a uniquely named marker session through the private instance's own socket
      (`ngtermctl --socket <private>`). `live tree` over the sshd must list exactly that marker, before any create
      or end runs through ssh.
- [x] then run, through `MobileCore`:
  - create a session, attach its pane, run `echo` and read it in the rows;
  - attach a second client and check that the roles are leader and follower, then take the lead;
  - kill the connection, reattach and find the same screen;
  - end the session, and the tree no longer lists it
- [x] reap through the private state dir's own server on every exit path; with `CI` set a missing `sshd` fails, and
      locally it skips with a reason
- [x] run the script; it must pass before Task 11
  - Notes: the script (Linux, `--profile debug|release`, debug by default) strips `NGTERM_*`, builds and stages
    into `<target>/me2e.XXXXXX` (`MOBILE_E2E_WORK_ROOT` overrides), and passes `MOBILE_E2E_STAGE` and
    `MOBILE_E2E_WORK` (not `NGTERM_*`, which it strips); the scratch directory goes after a pass and stays after a
    failure with `headless.log` and `sshd/sshd.log`. The private control socket lives under it, so the test refuses a
    path over 103 bytes. Every process of the instance runs with a cleared environment (the private
    `NGTERM_STATE_DIR` and `HOME`, the stage first on `PATH`, `SHELL=/bin/bash`); sessions' panes still run the
    account's login shell (fish on the dev box), so the line typed is `printf 'e2e-%s\n' 42`, typed again until it
    prints (a starting fish drops early input). The instance starts with a session of its own, so the isolation step
    closes it through the private socket after making the marker, then requires `live tree` over ssh to list exactly
    the marker. The machine's own headless terminal of a pane attaches first and leads, so a phone's pane starts as a
    follower whose typing the session server drops: the test takes the lead before typing, then checks the second
    phone follows, and that its Take control turns the roles round. The connection is killed by a TCP proxy in front of
    `sshd` (`Proxy::sever`), so `sshd` and new connections carry on. Host keys are pinned up front (`HostKeyStore`
    answering the `sshd` key's fingerprint). Reaping: SIGTERM to the instance's own child, then `ngterm-session kill
    --force` of every session its namespace's server lists, waiting for that server's `server.sock` to go (polling
    `list` would keep restarting its idle grace) and removing the namespace's directory. CI wiring is Task 21's.

### Task 11: Android project, build pipeline and lint gate

**Files:**
- Create: `mobile/android/` (`settings.gradle.kts`, `build.gradle.kts`, `gradle/libs.versions.toml`, the Gradle
  wrapper, `.editorconfig`, `config/detekt/detekt.yml`, and the modules `app` and `core`)
- Create: `scripts/build-android.sh`, `crates/ngterm-mobile/uniffi.toml` (the bindings' package
  `com.nickalie.ngterm.core.ffi`, and `android = true`: Lint refuses UniFFI's default `java.lang.ref.Cleaner` below API
  33)
- Modify: `scripts/setup-android.sh` (`--check`, which prints the exports), `build/pins.env` (platform `37.0`),
  `crates/ngterm-mobile/src/api.rs` (`core_version()`, which the smoke test reads)

- [x] write `scripts/build-android.sh`:
  - check the toolchain (`scripts/setup-android.sh --check`) and build the libghostty-vt prefix;
  - `cargo ndk -t arm64-v8a -t x86_64 -P 26 build -p ngterm-mobile` (release by default);
  - generate the Kotlin through the in-crate `uniffi-bindgen` in library mode;
  - run Gradle
- [x] set up `core` (generated bindings, `jniLibs`, JNA aar) and `app`:
  - Compose, Material 3, Navigation, DataStore, the adaptive and window-size-class libraries;
  - a fake `MobileCore` implementation for tests;
  - `minSdk 26`, `targetSdk 36`, current stable versions in the catalog;
  - the application id from ngterm's identity (`com.nickalie.ngterm.android`)
- [x] set up the whole lint gate from "Linting" before any feature code, and wire it into `./gradlew check`
- [x] prove the gate bites: add one violation per tool (an unused variable, misformatted code, a composable missing its
      `modifier` parameter, a hard-coded string), check that each fails `check`, then remove them
- [x] configure Roborazzi and the instrumented runner; add a smoke test where the app launches and the binding loads
      and reports its version
- [x] build the APK and run `./gradlew check` plus the tests; all must pass before Task 12
  - result (2026-11-03): AGP 9.5.0 (built-in Kotlin), Kotlin 2.4.21, Gradle 9.8.2, compose-bom 2026.09.00, Roborazzi
    1.77.0, Robolectric 4.17, Spotless 8.11.0 with ktlint 1.8.0, detekt 1.23.8 with compose-rules 0.4.28 (detekt
    2.0 is still alpha; compose-rules 0.5+ need it). The current AndroidX libraries require compileSdk 37, so the app
    compiles against `android-37.0` and keeps `targetSdk 36`. `scripts/build-android.sh assembleDebug check` is
    green; the instrumented smoke test passed on the headless `ngterm-phone` AVD (API 36, x86_64). Gate probes: an
    unused variable and a composable without `modifier` failed detekt (`UnusedPrivateProperty`, `ModifierMissing`),
    misformatted code failed `spotlessKotlinCheck`, a hard-coded layout string failed `lintDebug` (`HardcodedText`),
    and a deprecated call failed `compileDebugKotlin` under `-Werror` (Kotlin 2.4 reports no unused-variable
    warning, so detekt is what catches that one). Lint disables only the calendar-dependent checks (`OldTargetApi`,
    `NewerVersionAvailable`, `GradleDependency`, `AndroidGradlePluginVersion`), with the reason in the build files

### Task 12: Design system and adaptive scaffold (frontend-design)

**Files:**
- Create: `mobile/android/DESIGN.md`, `app/src/main/kotlin/.../ui/theme/`, `ui/components/`,
  `ui/AdaptiveScaffold.kt`

- [x] invoke `frontend-design`, choose the aesthetic direction, and record it in `DESIGN.md`:
  - tokens for light and dark, the type scale, spacing, shapes, motion;
  - the built-in terminal palettes;
  - status colors with paired icons;
  - the component inventory
- [x] build the theme and the components: status badge, machine card, session row, section header, empty, error and
      offline states, banner, confirm dialog, cover
- [x] build `AdaptiveScaffold` on window size classes (compact stack, medium rail, expanded sidebar), with insets,
      edge-to-edge and predictive back
- [x] write Roborazzi screenshots of the components and the scaffold at three widths, in light and dark and at font
      scale 2.0, plus a contrast test on the status colors
- [x] run `./gradlew check` and the tests; all must pass before Task 13
  - result (2026-11-03): the direction is "copper and patina" (DESIGN.md): graphite / drafting-grey grounds, the
    brand's copper as primary and verdigris as the healthy state, flat tonal surfaces, and one loud element, the
    copper bus bar marking where you are. Type is Atkinson Hyperlegible Next with its Mono sibling (variable fonts,
    OFL, `licenses/`, NOTICE). Icons are the app's own vector drawables (no icon library); the status glyphs reuse
    upstream's six shapes. Window size classes come from `currentWindowAdaptiveInfoV2` (the V1 call is deprecated).
    Destinations for now: Machines, Sessions, Settings (Settings shows the core version, which the smoke test now
    opens). `./gradlew check assembleDebug` is green (30 design goldens plus 2
    launch goldens, `ContrastTest`, `NavLayoutTest`, `AdaptiveScaffoldTest`), and the instrumented smoke test passed
    on the headless `ngterm-phone` AVD

### Task 13: Keys, machines and first connect

**Files:**
- Create: `ui/onboarding/`, `ui/machines/`, `data/MachineStore.kt`, `platform/KeystoreKeyVault.kt`,
  `platform/PinnedHostKeys.kt`

- [x] invoke `frontend-design` for these screens, inside `DESIGN.md`
- [x] first run: generate a key, then show the public-key screen (the `authorized_keys` line, Copy, Share, a short
      how-to); offer importing a key with its passphrase
- [x] Machines home: one card per machine with its state and aggregate agent status; empty and offline states; Add and
      Edit Machine with inline validation; delete with undo
- [x] first connect: a fingerprint confirmation sheet; a changed host key gives a hard-stop screen explaining the risk,
      with Remove pinned key as a deliberate action
- [x] implement `KeyVault` (Keystore AES-GCM wrap) and `HostKeyStore` over DataStore
- [x] write view-model unit tests, screenshots of each screen and state, and a UI test of add machine → confirm
      fingerprint → connected (fake core)
- [x] run `./gradlew check` and the tests; all must pass before Task 14
  - result (2026-11-03): `data/` holds `MachineStore` (DataStore, JSON), `AppKeys` (the keys' public halves in
    DataStore; `KeyMaker` wraps the core's `generate_key`/`import_key`) and `Connections` (the core's `EventSink`: each
    machine's `Link` and last listing); `platform/` holds `KeystoreKeyVault` (AES-256-GCM under an Android Keystore key,
    one sealed file per key in the no-backup dir, the handed bytes wiped), `PinnedHostKeys` (the core's
    `HostKeyStore`) and `NetworkMonitor` (offline banner, `network_changed`). `NgtermApplication` makes the `AppGraph`
    lazily, so the JVM tests never load the native library; the manifest gains INTERNET and ACCESS_NETWORK_STATE. The
    first run (welcome, import, public key with Copy/Share and the append command) gates the scaffold; the Machines
    destination stacks Add/Edit machine, a key's public half and the changed-key stop, with Undo for a removal; the
    host-key question is a sheet over the whole app. A card's tap edits the machine until Task 14 points it at the
    machine's sessions; its More menu has Connect/Disconnect, Edit and Remove. `FakeMobileCore` gained host keys
    (`hostKeys` + a `HostKeyStore`: unknown waits for `answerHostKey`, changed fails as the core does), the
    `Unreachable` event before a failed connect, and an edited machine connecting anew. MachineCard's name no longer
    truncates beside a long label: the badge wraps under it (`rows_medium_font2` re-recorded). DESIGN.md gained the
    screens, `KeyPlate`/`CopyButton`/`ScreenFrame` and the Back and More icons. `./gradlew check assembleDebug` is
    green (50 new goldens in `screenshots/screens`, at phone light/dark/2.0, medium light and expanded light/dark;
    the empty, offline, error and loading states at phone light and dark); the add → confirm fingerprint → connected
    flow, a declined key, the changed-key stop, Undo and Edit run as Robolectric UI tests on the fake core, as Task
    12's did. On the headless `ngterm-phone` AVD the smoke test now walks the first run with the real core (key made,
    sealed by the Keystore) and a Keystore vault test passes


### Task 14: A machine's sessions

**Files:**
- Create: `ui/sessions/`

- [x] invoke `frontend-design` for the sessions list
- [x] the list from `Tree`, with status badges, companions under their parent, an Attached group and pull to refresh;
      New Session with an optional name; End Session confirms
- [x] states: loading, empty (with New Session as the main action), unreachable with retry, and `ngtermctl` missing on
      the server with a pointer to the README
- [x] write view-model tests, screenshots, and a UI test of create → appears → end → disappears (fake core)
- [x] run `./gradlew check` and the tests; all must pass before Task 15
  - result (2026-11-03): `ui/sessions/` holds `SessionsRoute` (stacked on the Machines destination: tapping a machine
    card opens it, Edit moved to the card's More menu), `SessionsScreen`, `SessionEntry`, `NewSessionDialog`,
    `SessionsBody` (the body by link and listing, pure) and `SessionsViewModel`. `Connections` gained `treeFailures`
    (why a machine's last listing failed, cleared by the next), `attached` (the sessions this app attached),
    `listSessions`, `createSession` (blank name = none), `endSession`, `attach` and `detach`; an attached session
    that ends or leaves the listing is detached, and a disconnect forgets the machine's. A missing `ngtermctl` is
    recognized in the app from the far shell's words (`isMissingCli`: "ngtermctl" with "not found" or "No such
    file"), since the core passes them on as `Remote`; it points to the README's "A server without a display"
    (Open the README, with Retry). Tapping a row attaches it (Task 18 opens the session view from there); its More
    menu has Attach/Detach and End session. The Sessions destination stays the cross-machine view of what is
    attached, filled in with the session view (Task 18); a machine's own list lives under Machines. DESIGN.md gained
    the screen, `SessionRow`'s menu slot and `MessagePanel`'s secondary action. Tests: `SessionsBodyTest`,
    `SessionsViewModelTest`, three new `ConnectionsTest` cases, `SessionsFlowTest` (create → appears → end → gone,
    the empty state and Back, the missing CLI and Retry, attach and detach) and 16 goldens (the list at the six
    windows, nine states on a phone, the New session dialog). Robolectric never settles a text field in a dialog
    under the phone qualifiers (a layout loop in its window transforms), so the flow test and the dialog's golden use
    its default window

### Task 15: TerminalView rendering

**Files:**
- Create: `ui/terminal/TerminalView.kt`, `ui/terminal/RowBufferReader.kt`, `ui/terminal/GlyphCache.kt`, a bundled
  font in `res/font/`

- [x] write a JVM test first: `RowBufferReader` decodes Task 9d's golden fixture
- [x] draw the changed rows on a Canvas with a bundled monospace font, the platform fallback for CJK and emoji, and a
      cache of shaped runs per row
- [x] draw the cursor shapes and the selection; measure cells and report the grid size, which sends a pty resize
- [x] keep frames within budget: profile a full-screen `cat` of a large file and record the frame times here
  - measured (2026-11-03), app side on the headless x86_64 `ngterm-phone` AVD (API 36, 1080 × 2400, SwiftShader:
    the emulator has no host GPU), debug build, `TerminalFrameTimeTest`: a fake pane whose every frame scrolls the
    whole 54 × 52 grid by 7 lines of a long file, read as fast as the display takes them. 596 frames in 10 s (60 fps,
    one pane frame read per display frame), none with over 16.7 ms of app work. UI thread (measure, layout, draw):
    p50 3.1 ms, p90 4.6 ms, p99 7.8 ms; render thread (sync, issue): p50 2.8 ms, p90 4.5 ms, p99 5.8 ms; GPU
    (SwiftShader, the CPU): p50 8.9 ms, p99 24.3 ms; total pipeline latency p50 16.8 ms, p99 33.5 ms, the GPU's.
    Core side on the dev box (x86_64, release), `rows/tests.rs`'s ignored `packing_a_full_screen_cat`: 2 MiB of
    text in 64 KiB reads into 52 × 50, every row changed: `frame()` p50 148 µs, p99 192 µs; `pack()` p50 36 µs, p99
    47 µs, 3.6 KB per frame. A real core on the emulator was not profiled: a pane needs an ssh attach, and the
    emulator test has no server to reach; the two halves meet only at the packed buffer, measured on both sides
- [x] write screenshots of known buffers (colors, attributes, wide characters, cursor shapes, selection), and a UI test
      of resizing through rotation
- [x] run `./gradlew check` and the tests; all must pass before Task 16
  - result (2026-11-03): `ui/terminal/` holds `RowBufferReader` (rows.rs's layout v1), `TerminalScreen` (the rows
    held, replaced per frame; a full frame or a new grid starts blank), `GlyphCache` (`TerminalFont`: four paints and
    the cell; a row's runs measured once per row record: a run of the font's own glyphs is one string on the grid, a
    drifting run, a wide run and a cluster go glyph by glyph, centered or squeezed into their cells),
    `TerminalPainter` and `TerminalDecorations` (grounds, selection, text, lines, cursor shapes; defaults resolved
    from the header drawn) and `TerminalView` with `TerminalState` (one frame read per display frame on
    `Dispatchers.Default`, since the held seq, 0 after `attached()`; stale frames asked again; a closed pane keeps
    the last screen; the grid that fits goes to `Pane.resize`). The font is JetBrains Mono 2.304's four static faces,
    the release Ghostty and desktop embed (OFL, `licenses/`, NOTICE). ➕ The palette reaches the terminal: the core
    gained `Pane.set_colors(PaneColors)` (kept in the pane's slot and applied to every terminal a reattach installs;
    Rust test in `pane/tests.rs`), which `TerminalView` calls with the theme's palette. The cursor never blinks and
    the selection is the palette's tonal fill (DESIGN.md > The terminal). `core` test fixtures gained `FakePane` and
    `RowBufferWriter` (it packs the golden screen byte for byte as Rust does). Tests: `RowBufferReaderTest`,
    `TerminalScreenTest`, `TerminalViewTest` (turning the phone's size gives the grid that fits and draws it, frames
    asked since/whole, closed, stale, palette), 12 goldens in `screenshots/terminal`, and the emulator's frame
    profile. Rotation is the view's size turning in Robolectric: its `setQualifiers("+land")` does not resize a
    running window. `scripts/build-android.sh --profile debug check connectedDebugAndroidTest` is green, as are
    `cargo test`/`clippy --all-targets --all-features -D warnings`/`fmt` for `ngterm-mobile`

### Task 16: Touch, mouse and selection

**Files:**
- Create: `ui/terminal/TerminalGestures.kt`, `ui/terminal/SelectionHandles.kt`

- [x] touch:
  - drag scrolls, using mouse reporting when the program asked for it;
  - long-press starts a selection with handles and a Copy/Paste toolbar;
  - pinch changes font size;
  - a tap sends a click under mouse tracking
- [x] mouse and trackpad: click, drag, wheel, and a right-click menu
- [x] the clipboard-read prompt (Allow, Deny, Always for this machine) and the paste confirmation for unsafe text
- [x] write UI tests for each gesture against the fake core, which records the input
- [x] run `./gradlew check` and the tests; all must pass before Task 17
  - result (2026-11-03): `ui/terminal/` gained `TerminalInput` (positions as the pane's cells and pixels; mouse,
    wheel, scroll, drag in whole rows, a spline fling, pinch zoom in whole sp within `TerminalTextSizes` 8-32) and
    `TerminalSelection` (the touch selection, the right-click menu, copy, paste with its unsafe hold) in
    `TerminalInput.kt`; `TerminalGestures.kt` (a finger: drag scrolls through `Pane.wheel`, which the pane reports to
    a program tracking the mouse; long press selects the word and drags on; a handle's drag moves its end; pinch; a
    tap clicks under tracking), `TerminalMouse.kt` (buttons, drags and moves to a tracking program, shift overriding;
    otherwise click/double/triple and drag select, the right button the menu; the wheel and hover motion),
    `SelectionHandles.kt` (copper stems and teardrop knobs, flipped at the view's edge; the toolbar; the menu),
    `TerminalOverlays.kt` and `TerminalPrompts.kt` (`PasteConfirmDialog`, the clipboard-read sheet). The text size
    moved into `TerminalState.textSize` (Task 17's shortcuts and Task 20's setting set it there). The mouse-tracking
    state the app needs was already exposed (`Pane.mouse_tracking`), so no Rust change. Clipboard reads:
    `data/ClipboardReads.kt` takes the core's `ClipboardReadRequest`/`ClipboardReadDismiss` through a new
    `Connections.listen` hook, queues them for the sheet (shown from `NgtermApp` over any destination), answers with
    the device's clipboard (`platform/AndroidClipboard`) or `Deny`, and keeps "Always allow on build-01" per machine
    in `ClipboardTrust` (its own DataStore; Settings lists it in Task 20), answering that machine's later reads at
    once. `FakePane` records every input call (`FakeInput`) and answers as the terminal would (tracking, unsafe
    paste). Tests: `TerminalGesturesTest` (drag both ways, fling, long press + Copy, long press + drag, handle drag,
    unsafe paste confirmed, pinch up and clamped down, tap with and without tracking, mouse to a tracking program,
    mouse drag and double click, wheel and the right-click menu), `ClipboardReadsTest`, `ClipboardReadPromptTest`
    (the app on the fake core: allow, deny, always, dismissed), goldens `terminal/touch_selection_*`, `menu_*`,
    `paste_unsafe_*` and `screens/clipboard_read_sheet_*`. `./gradlew check` is green (spotless, detekt, lint, unit
    and screenshot tests); the instrumented tests compile, and were not rerun on the emulator: Robolectric drives
    every gesture here
  - ➕ A program's clipboard write (`ClipboardWrite`, OSC 52) is not applied yet: added to Task 18, which routes the
    shown panes' events

### Task 17: Keyboard input

**Files:**
- Create: `ui/terminal/TerminalInputConnection.kt`, `ui/terminal/ExtraKeysRow.kt`, `ui/terminal/KeyMapping.kt`

- [x] write a JVM test first: `KeyMapping` turns Android `KeyEvent`s into `ngterm-term`'s `KeyInput` (`PhysicalKey`,
      `Mods`, text) through the binding
- [x] an `InputConnection` with suggestions and autocorrect off, committing text and drawing IME composition at the
      cursor
- [x] invoke `frontend-design` for the extra-keys row: Esc, Tab, sticky Ctrl and Alt, arrows, Home/End, PgUp/PgDn,
      `|`, `~`, `/`, with clear sticky states and haptics; hide it while a hardware keyboard is attached
- [x] hardware keyboard shortcuts: next and previous pane and session, and font size
- [x] write UI tests: type through the soft keyboard and the extra keys, and press a hardware chord
- [x] run `./gradlew check` and the tests; all must pass before Task 18
  - result (2026-11-03): `ui/terminal/` gained `KeyMapping` (a key code as Ghostty's physical key name, the meta
    state as the pane's bits, the text before Ctrl/Alt/Meta, the unshifted character, press/repeat/release, dead keys
    as composing; a character as the US key that types it, for sticky modifiers; the app's chords),
    `TerminalKeyboard` (the sticky Ctrl and Alt, off → once → locked → off; hardware and IME keys, commits with line
    breaks as Enter, compositions; chords: Ctrl+=/Ctrl++/Ctrl+- step the text size, Ctrl+0 resets it,
    Ctrl+Shift+arrows run `TerminalShortcuts`' pane and session callbacks, and a chord with no callback is the
    program's),
    `TerminalInputConnection` (a `BaseInputConnection` whose editable holds only the composition; the editor asks for
    no suggestions, autocorrect, capitals, learning or fullscreen; the Compose node opens a platform text input
    session while the terminal has focus and takes hardware keys; `CompositionPainter` draws the composition at the
    cursor, underlined in the cursor's copper) and `ExtraKeysRow` (two lines of eight, or one of sixteen from 768dp;
    keys act on press with a tick, the arrows and page keys repeat while held; sticky states by outline and fill and
    TalkBack's state words; hidden while a hardware keyboard is attached). A tap on the terminal gives it the focus and
    opens the soft keyboard; a mouse button gives only the focus. Every key the row and the chords need was already
    expressible through `PaneKey`, so no Rust change. DESIGN.md gained a Keyboard section. Tests: `KeyMappingTest`
    (Robolectric's key character map), `TerminalKeyboardTest` (the input method's connection and editor flags,
    composition, extra keys, sticky Ctrl once and Alt locked, a held arrow repeating, hardware typing and chords, the
    row standing aside), goldens `terminal/keys_typing_dark|light`, `keys_font2`, `keys_landscape`. `./gradlew check`
    is green; the instrumented tests compile and were not rerun on the emulator (Robolectric drives every input here)
  - ➕ Left for Task 18: the session view passes its `TerminalKeyboard` (one for all panes) to every `TerminalView`
    and the `ExtraKeysRow`, sets `TerminalShortcuts` (next/previous pane and session), places the row over the IME
    (`imePadding`), and makes the terminal's text readable by TalkBack

### Task 18: Session view, adaptive

**Files:**
- Create: `ui/session/SessionScreen.kt`, `ui/session/PaneSwitcher.kt`, `ui/session/SplitLayout.kt`,
  `ui/session/HudBanner.kt`, `ui/session/LeadCover.kt`

- [x] invoke `frontend-design` for the session view at all three widths
- [x] compact: the active pane fills the screen; a top bar with name, status, context and pane switcher; swipe between
      the panes; the HUD as a banner
- [x] medium: a collapsible rail of attached sessions
- [x] expanded: a sidebar of machines and their attached sessions with status badges, beside the server's split layout
      (1 or 2 panes on either axis, or a hidden split), where a tap focuses a pane
- [x] the follower cover with **Take control**; a reconnecting cover that keeps the last screen visible; "session
      ended" and "session gone" states; Detach and End in the session's menu
- [x] ➕ route the shown panes' events to their views (`PaneDirty`, `PaneAttached` to `TerminalState`), and apply a
      program's clipboard write (`ClipboardWrite`, OSC 52) to the device's clipboard
- [x] ➕ the keyboard (from Task 17): one `TerminalKeyboard` for every pane's `TerminalView` and the `ExtraKeysRow`
      (over the IME), its `TerminalShortcuts` moving between panes and sessions, and the terminal's text read by
      TalkBack
- [x] write screenshots at three widths for one pane, two panes on each axis, a hidden split, a follower and a
      reconnecting pane, plus a UI test where pane switching and a size-class change keep the focused pane
- [x] run `./gradlew check` and the tests; all must pass before Task 19
  - result (2026-11-03): `data/LiveSessions.kt` keeps each attached session's stream as its events leave it (state,
    status, context, layout, HUD; each pane's role, cover, end and title), routes `PaneDirty`/`PaneAttached` to the
    pane's `TerminalState` (now a `PaneFrames`), lets a detached session's state go, and puts a program's OSC 52 write
    to the standard clipboard on the device's (`ClipboardSink`, `AndroidClipboard.write`; selection and primary are
    dropped: Android has neither). `ui/session/` holds `SessionPanes` (a pane is shown the first time its slot has a
    size and stays shown, with its terminal, lead and text size, until its session is detached or its layout drops
    it; why one can't be shown, with Retry), `SessionViewModel` (the attached sessions by machine, the open session,
    live or gone, the focused pane per session, Detach, End), `SessionsDestination` (the list, the session view, the
    gone state; Back closes), `SessionScreen` and `SessionBar` (name, status or stream state, context, `PaneSwitcher`,
    More: Detach, End session confirmed), `HudBanner`, `SplitLayout` with `PaneSlot` (side by side or stacked, the bus
    bar in the focused pane's gutter, a hidden split's focused pane alone), `PaneCover`/`LeadCover` (follower with
    Take control, attaching, reconnecting as a light scrim and a strip over the last screen, program exited with
    Detach, can't show with Retry) and `AttachedList` (the list, the sidebar's machines and sessions, the rail's cells
    or rows). Tapping a session on a machine's list attaches it and opens its view under Sessions; a phone's open
    session hides the bottom bar (`AdaptiveScaffold.navigationVisible`), and the rail gained a content slot. One
    `TerminalKeyboard` serves the view's panes and `ExtraKeysRow`, follows the focused pane, and its chords move
    between panes and sessions; the pane on screen is read by TalkBack (`TerminalView.readAloud`, refreshed at most
    once a second); a sideways swipe on a phone's terminal moves between panes (`TerminalGestures`' swipe), and the
    focus follows a pane switch while the soft keyboard is up. `FakeMobileCore` now hands back a shown pane's handle
    and hides panes on `hidePane`, `detachSession` and `disconnect` (`shownPanes`). DESIGN.md gained the session view.
    Tests: `LiveSessionsTest`, `SessionViewModelTest`, `SessionViewTest` (attach from the list opens the view; pane
    switching, a size-class change and back keep the focused pane, where the extra keys type; a swipe; Take control;
    reconnecting; a clipboard write; Detach), a `FakeMobileCoreTest` case, `SessionsFlowTest`'s tap now opening the
    view, and 25 goldens in `screenshots/session` (one pane with the HUD, two panes on each axis, a hidden split, a
    follower and a reconnecting pane at compact dark and light, medium light and expanded dark, plus the medium rail
    open). `./gradlew check` is green; the instrumented tests compile and were not rerun on the emulator (Robolectric
    drives the view here)

### Task 19: Foreground service, notifications and asks

**Files:**
- Create: `service/LinkService.kt`, `platform/Notifications.kt`, `ui/asks/`

- [ ] `LinkService`: a `specialUse` foreground service with its subtype property, holding the core while any session
      is attached; its notification shows the attached count and Detach All; the `POST_NOTIFICATIONS` request
      explains why it's needed
- [ ] a `ConnectivityManager` callback calls `network_changed`
- [ ] server notifications and agent-status changes (waiting, done, error, each toggleable) go to a channel per
      machine, and a tap opens the session and pane
- [ ] invoke `frontend-design` for the ask dialogs: button, input and choice asks, the free-form row and `cancel_id`
      for choice asks; in the background, notification actions, inline reply, or opening the app, per the ask rule
- [ ] write unit tests mapping events to notifications, screenshots of the ask dialogs, and a UI test answering an ask
      in-app and from a notification (fake core)
- [ ] run `./gradlew check` and the tests; all must pass before Task 20

### Task 20: Settings

**Files:**
- Create: `ui/settings/`, `data/SettingsStore.kt`

- [ ] invoke `frontend-design` for Settings
- [ ] the settings:
  - font and size;
  - the built-in terminal palette, with a live preview;
  - follow the system's light/dark;
  - the extra-keys layout;
  - one toggle per notification kind;
  - keep the screen on while attached;
  - clipboard-read permissions per machine;
  - key management (show, copy, rotate, import)
- [ ] write view-model tests, and screenshots at three widths
- [ ] run `./gradlew check` and the tests; all must pass before Task 21

### Task 21: CI

**Files:**
- Create: `.github/workflows/android.yml`
- Modify: `.github/workflows/cross-platform.yml`

- [ ] add `ngterm-mobile` to the `crates` job's packages on all three OSes; the e2e test stays `#[ignore]` there
- [ ] `android.yml`, path-filtered to:
  - `crates/ngterm-mobile`, `ngterm-term`, `ngterm-core`, `ngterm-proto`, `ngterm-ghostty-vt`(-sys);
  - `mobile/`;
  - `scripts/{build-android,setup-android,build-ghostty-vt,test-mobile-e2e}.sh`;
  - `build/pins.env`
- [ ] in `android.yml`, run on Ubuntu:
  - `scripts/setup-android.sh`;
  - `scripts/build-android.sh`;
  - `./gradlew check` and the unit and screenshot tests;
  - emulator UI tests on phone and tablet profiles (`reactivecircus/android-emulator-runner`, KVM);
  - `scripts/test-mobile-e2e.sh` with `openssh-server` installed;
  - upload a debug APK
- [ ] push to a CI branch and confirm both workflows are green

### Task 22: Verify acceptance criteria

- [ ] on the emulator against Task 10's staged server, every Overview requirement works:
  - add a machine and pin its key;
  - list, create, attach, type;
  - see status, context, HUD and notifications;
  - answer an ask in-app and from a notification;
  - take the lead from a second client;
  - detach, end
- [ ] the phone, medium and tablet layouts, multi-window and rotation behave as Task 18 specifies
- [ ] a UI/UX review against the standard above (targets, contrast, states, TalkBack, font scale, motion), with its
      fixes made
- [ ] the checks: `cargo test -p ngterm-core remote`; `cargo test -p ngterm-term`, with default features and with
      `--no-default-features --lib`; `cargo test -p ngterm-mobile`; `scripts/test-mobile-e2e.sh`;
      `scripts/build-android.sh` plus `./gradlew check`
- [ ] a revmux round over the whole change, with every verified finding fixed

### Task 23: [Final] Update documentation

- [ ] README: an "Android" section (what it does, setup and build through the two scripts, adding the device key to
      the server, the v1 limits), listed among the extensions beyond upstream
- [ ] CLAUDE.md: an `ngterm-mobile` row in the architecture table (platform `any`), and the rule that UI work goes
      through `frontend-design` and `mobile/android/DESIGN.md`
- [ ] the crate root docs of `ngterm-mobile`: its module map, the threading model, the seams
- [ ] move this plan and its `20261102-android-app/` directory to `docs/plans/completed/`

## Post-Completion

**Manual verification:**
- Real devices: a phone, a tablet, and a foldable if one is available.
- Run the arm64 build on a real device; only x86_64 test binaries ran on an emulator during preparation.
- Check a hardware keyboard and a trackpad, and IME for a non-Latin language.
- Check battery with the foreground service on for an hour, and screen-off reconnect across Wi-Fi ↔ mobile data
  switches.
- Attach a desktop ngterm and the phone to the same session, and check the presenter and leadership rules.

**Out of v1:**
- HTML pages, program overlays, jump hosts, passwords, Kitty images, ligatures, a GPU renderer, editing the layout
  from the phone, and Play Store publishing.
- Future only: an iOS app over `ngterm-mobile` (it would need iOS Zig targets, and APNs for background notifications).
