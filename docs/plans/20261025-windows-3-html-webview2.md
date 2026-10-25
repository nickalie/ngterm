# Windows 3: HTML overlays on WebView2

## Overview

- **Goal**: the HTML overlays (session and pane pages, URLs, dev servers, saved logins, interactive pages and their
  channel) on Windows, on WebView2, with the behavior they have on Linux. Third of the four Windows plans;
  `20261025-windows-1-control-plane.md` holds the series' goal, done bar, targets and what is out of it.
- **Decision** (user, 2026-10-25): WebView2 renders offscreen and its frames are drawn by GPUI, as WPE's are on Linux:
  `CoreWebView2CompositionController` into a composition visual, `Windows.Graphics.Capture` into D3D11 textures,
  plan 2's external surfaces into the window. GPUI draws everything, so the HUD, menus, popovers and clipping over a
  page work unchanged and the `html_overlay` layer stays shared. Rejected: WebView2's visual inside GPUI's
  DirectComposition tree (anything GPUI draws over a page would need a second GPUI layer) and CEF (about 200 MB of
  Chromium shipped with the app, not the platform's engine).
- **Risk, taken first**: capture latency and the hand-built input path. Task 1 is a spike with named fallbacks.

## Context (from discovery)

- `ngterm-wpe` (Linux): `Host::start` with an engine thread of its own, commands in by method, `Event`s out on one
  channel (frames, load progress, questions held open by token), `SessionSpec` (in memory or on disk), `Settings`,
  user scripts and script messages, URI schemes, policy refusals, `engine::interaction` (clipboard, `<select>`,
  context menu, input methods), `frames.rs` (the buffer protocol and acknowledgements), `worker` (an engine process).
- `ngterm-gpui/src/html_overlay/`: `registry`, `page`, `view`, `panel`, `menus`, `policy` (engine-neutral by what they
  do) and `engine`, `input`, `sandbox` (WPE-specific); all behind `cfg(all(feature = "html", target_os = "linux"))`.
  The module docs already name WebView2 for Windows and its Esc rule (`AcceleratorKeyPressed` must not re-dispatch
  an unhandled Esc).
- `crates/ngterm-gpui/tests/html_overlay/` (cases, the fps measure: 24 frames/s at 1080p on dmabufs).
- Plan 2's external surfaces on DirectX: a shared D3D11 texture by NT handle with a fence, import capabilities by
  adapter LUID.

## Development Approach

- **testing approach**: Regular (code first, then its tests in the same task).
- complete each task fully before moving to the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests**, success and error cases.
- **CRITICAL: all tests must pass before starting the next task**, on both Windows runners with the real WebView2
  runtime.
- **CRITICAL: update this plan file when scope changes during implementation**, the spike's outcome first of all.
- **Local Windows work in the VM** (user, 2026-10-25): a task that needs Windows by hand (a red test or id to
  debug, a probe, an interactive check) starts the Win11-admin VMware VM, works there and stops it at the end of
  the task (memory: windows-macos-debug-in-vmware). The VM is off by default and never left running between tasks;
  CI on the Windows runners stays the gate.
- Linux's HTML overlays stay as they are; their tests and ratchet ids stay green through the refactor (Task 8).

## Testing Strategy

- **unit tests**: per task; `ngterm-webview2`'s own tests on the real runtime on both Windows runners.
- **frontend tests**: `tests/html_overlay`'s Windows form, the fps measure included.
- **conformance suite**: once at the end (Task 11) on `conformance-windows`, plus `run-linux.sh` once on Linux.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- keep the plan in sync with the work done

## Solution Overview

- **`ngterm-webview2`** (Windows), the twin of `ngterm-wpe`: the same `Host` and `Event` shape, so the frontend's
  adapter is thin. Bindings: `webview2-com` (the crate wry and Tauri use) over `windows`. It runs on the Evergreen
  runtime Windows 11 ships; where none is found, the host refuses every page with the reason, as WPE's host does when
  its sandbox cannot start.
- **Frames**: the controller's root visual target is a `Windows.UI.Composition` visual; a `GraphicsCaptureItem` over it
  feeds a free-threaded `Direct3D11CaptureFramePool` on GPUI's adapter (the LUID from plan 2's import capabilities);
  each frame is copied into the shared texture the external surface presents, with its damage, and acknowledged as
  WPE's frames are.
- **Placement and input**: the controller's `Bounds` follow the page's rect inside the GPUI window (its
  `ParentWindow`), so pointer coordinates and IME candidate windows line up with where GPUI draws the page. Mouse and
  wheel go through `SendMouseInput`/`SendPointerInput`. The keyboard gets real focus (`MoveFocus`), so IME and TSF
  work natively; the app's chords are taken in `AcceleratorKeyPressed`, and an unhandled Esc is consumed, never
  re-dispatched.
- **Policy, mapped one to one** (Technical Details).
- **`html_overlay` shared**: `registry`, `page`, `view`, `panel`, `menus` and `policy` are compiled on both OSes; each
  OS has one adapter module set (`engine`, `input`, `sandbox`) with the same function signatures, picked by `cfg`.
  No trait: one engine per OS at compile time.
- **References to study in their tasks**: wry (Apache-2.0/MIT) for `webview2-com` hosting and its event wiring;
  Microsoft's WebView2 samples for visual hosting (`WebView2APISample`'s `ViewComponent`); the
  `Windows.Graphics.Capture` samples for visual capture.

## Technical Details

| Linux (WPE) | Windows (WebView2) |
|---|---|
| in-memory store | `IsInPrivateModeEnabled` profile |
| saved logins on disk | a profile directory under the state dir |
| URI schemes | `CoreWebView2CustomSchemeRegistration` + `WebResourceRequested`, answered by token |
| user scripts | `AddScriptToExecuteOnDocumentCreated` |
| script messages | `WebMessageReceived` (`chrome.webview.postMessage`), the page channel's handlers |
| navigation and response policy | `NavigationStarting`, `WebResourceResponseReceived` |
| new windows, script dialogs, file choosers, permissions, full screen, downloads refused | `NewWindowRequested`, `ScriptDialogOpening`, the file input refused, `PermissionRequested`, `ContainsFullScreenElementChanged`, `DownloadStarting` |
| context menu reported, drawn by the app | `ContextMenuRequested` with its items reported, `Handled` set |
| clipboard: only the text the app hands in before a paste | the same rule (Task 7 settles how against the spike) |
| engine process | WebView2's own browser process; no worker of ngterm's |

## What Goes Where

- **Implementation Steps**: code, tests, spike notes, CI and docs in this repo.
- **Post-Completion**: manual checks in the Win11 VM.

## Implementation Steps

### Task 1: Spike: WebView2 offscreen into GPUI

**Files:**
- Create: `docs/spikes/20261025-webview2-capture.md`
- Create: a throwaway probe under the scratchpad or a `ci/**` branch (not merged)

- [ ] a composition controller rendering into a visual attached to no window: does it keep rendering, and does
      `GraphicsCaptureItem.CreateFromVisual` capture it without a border into GPUI's device (fallback: a cloaked owner
      window whose visual tree holds it)
- [ ] the controller on an engine thread of its own vs needing the GPUI thread for its `ParentWindow` (fallback: the
      GPUI thread, with the engine's work posted to it)
- [ ] where `<select>` popups and Chromium's own popups go (fallback: reported and drawn by the app, as on Linux)
- [ ] latency and frames/s at 1080p against WPE's numbers (fallback threshold recorded before the run)
- [ ] the clipboard rule: can a page's copy be reported and a paste be limited to the text the app hands in (the
      `navigator.clipboard` and `execCommand` paths, the keyboard paste), and with which WebView2 settings
- [ ] the WebView2 runtime and `Windows.Graphics.Capture` present on both runners (`windows-2025` is Server 2025); if
      a runner lacks the runtime, the workflow installs the Evergreen runtime first
- [ ] the spike note records each answer and the chosen path; this plan is updated from it
- [ ] run on both Windows runners (and in the VM for the interactive parts) before next task

### Task 2: ngterm-webview2: crate, host and engine thread

**Files:**
- Create: `crates/ngterm-webview2/Cargo.toml`, `src/lib.rs`, `src/host.rs`, `src/event.rs`, `src/spec.rs`
- Modify: `Cargo.toml` (workspace dependencies `webview2-com`, `windows`; members are the `crates/*` glob)
- Modify: `scripts/lib/native.sh` (the crate kept out of Linux's workspace runs, as the WPE crates are out of
  Windows')

- [ ] `Host::start` on the thread the spike chose, with its message loop; the environment created once per process
- [ ] the runtime's presence and version read; missing or too old, every page is refused with the reason
- [ ] the ids, events and specs in `ngterm-wpe`'s shape (same names where the meaning is the same)
- [ ] write tests for start, shutdown, the missing-runtime refusal (by a forced runtime path)
- [ ] run tests on both Windows runners - must pass before next task

### Task 3: Pages, stores and policy

**Files:**
- Create: `crates/ngterm-webview2/src/page.rs`, `src/policy.rs`

- [ ] a page per spec: InPrivate or a profile dir under the state dir for saved logins; its settings
- [ ] the refusals of the policy table; navigation and response questions held open by token
- [ ] load progress and what a page shows (title, URL, the loading state) reported
- [ ] write tests for each refusal, a navigation answered both ways, a saved login surviving a host restart, an
      InPrivate page keeping nothing
- [ ] run tests on both Windows runners - must pass before next task

### Task 4: URI schemes, user scripts and script messages

**Files:**
- Create: `crates/ngterm-webview2/src/schemes.rs`, `src/scripts.rs`

- [ ] the consumer's schemes registered at environment creation, each read answered by token (body, type, status)
- [ ] user scripts at document creation; script messages to the consumer, replies back into the page
- [ ] write tests for a scheme read answered, refused and never answered (the page's timeout), a script message
      round trip, a script on every frame vs the main frame
- [ ] run tests on both Windows runners - must pass before next task

### Task 5: Frames out

**Files:**
- Create: `crates/ngterm-webview2/src/frames.rs`

- [ ] the capture pool on the consumer's adapter by LUID; each frame copied into the shared texture with its damage
- [ ] a frame counts as rendered once the consumer presented it; a lost frame or device reported; a resize
      recreates the pool
- [ ] the frame stats the frontend reads (`FrameStats`)
- [ ] write tests for frames presented and acknowledged, a resize, a mismatched adapter refused, a page hidden and shown
- [ ] run tests on both Windows runners - must pass before next task

### Task 6: Input and focus

**Files:**
- Create: `crates/ngterm-webview2/src/input.rs`

- [ ] the controller's bounds kept on the page's rect; mouse, wheel and pointer input through the composition
      controller; the cursor the page asks for reported
- [ ] keyboard focus moved in and out with the page's focus; the app's chords taken in `AcceleratorKeyPressed`; an
      unhandled Esc consumed, never re-dispatched
- [ ] IME composition in place, its candidate window at the caret
- [ ] write tests for clicks and wheel at the right coordinates, a chord reaching the app, Esc consumed, focus leaving
      the page with the app's chord
- [ ] run tests on both Windows runners - must pass before next task

### Task 7: Clipboard, context menu and popups

**Files:**
- Create: `crates/ngterm-webview2/src/interaction.rs`

- [ ] the clipboard rule as the spike settled it: a page's copy reported, a paste carrying only the text the app hands
      in, never files
- [ ] the context menu reported with its items and answered by command; `<select>` and other popups as the spike
      decided
- [ ] write tests for copy, paste, a context-menu command, a `<select>` choice
- [ ] run tests on both Windows runners - must pass before next task

### Task 8: html_overlay shared across the two engines

**Files:**
- Modify: `crates/ngterm-gpui/src/html_overlay.rs`
- Move: `crates/ngterm-gpui/src/html_overlay/{engine,input,sandbox}.rs` → `html_overlay/linux/`
- Create: `crates/ngterm-gpui/src/html_overlay/windows/{engine,input,sandbox}.rs`
- Modify: `crates/ngterm-gpui/Cargo.toml` (`html` feature: `ngterm-webview2` on Windows)
- Modify: the other `cfg(all(feature = "html", target_os = "linux"))` sites: `crates/ngterm-gpui/src/{state,main}.rs`,
  `src/html_overlay/registry.rs`, `src/deck/tests/overlay.rs`, `src/deck/tests/overlay_visibility.rs`,
  `src/window/tests/overlay_visibility.rs`

- [ ] the shared modules compile on both OSes; the per-OS adapters have the same signatures
- [ ] the Windows adapter over `ngterm-webview2`, with the frames into `external_texture`
- [ ] Linux's HTML overlay tests and ratchet ids stay green, unchanged
- [ ] write unit tests for the Windows adapter against a stand-in engine, as `html_overlay::testing` does for WPE
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 9: The frontend's HTML overlay tests on Windows

**Files:**
- Modify: `crates/ngterm-gpui/tests/html_overlay/{main,cases}.rs`
- Modify: `.github/workflows/rust-windows.yml`

- [ ] the cases' Windows form (page shown, interactive page and its channel, saved login, URL and dev server, menus
      over a page)
- [ ] the fps measure's Windows mode with the threshold the spike set
- [ ] run in `rust-windows.yml` on both arches - must pass before next task

### Task 10: CLAUDE.md and README

**Files:**
- Modify: `CLAUDE.md`, `README.md`

- [ ] the crate map: `ngterm-webview2` (Windows); `ngterm-gpui`'s HTML overlays on Linux and Windows
- [ ] README: WebView2 (the runtime, what a page may not do, where saved logins live on Windows)
- [ ] run the gates - must pass before next task

### Task 11: Verify acceptance criteria

- [ ] push to a `ci/**` branch; wait (in the background) for `conformance-windows` on both arches
- [ ] add the newly green ids (`test_overlay`'s page ids among them) with `check_ratchet.py --list conformance/rust-passing-windows.txt --add`;
      a page id green on Linux and red on Windows is fixed, or recorded here with ⚠️
- [ ] `conformance/tools/run-linux.sh` once in the background; no Linux ratchet id went red
- [ ] every gate on Linux and `rust-windows.yml` green

### Task 12: [Final] Update documentation

- [ ] README and CLAUDE.md match what was built; the spike note says what was chosen
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

**Manual verification** (in the Win11 VM, started for it and stopped after):
- a page at 150 % scale on a second monitor: sharp, input aligned, IME candidates at the caret
- a dev server page with hot reload; a login kept across an app restart

**External system updates**: none.
