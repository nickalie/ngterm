# Windows 2: the GPUI frontend on Windows

## Overview

- **Goal**: every feature of the GPUI frontend works on Windows except the HTML overlays (plan 3) and the installer
  (plan 4). Second of the four Windows plans; `20261025-windows-1-control-plane.md` holds the series' goal, done bar,
  targets, decisions and what is out of it.
- **Starts from plan 1**: the app builds and opens a window on the stock GPUI Windows backend, its Linux-only modules
  log "not available on Windows yet", it quits on the session-end protocol, and the suite runs on
  `conformance-windows` with a Windows ratchet. This plan replaces each stub and grows the ratchet.
- **Why it is a plan of its own**: GPUI on Windows (`gpui-pre-windows`) renders through DirectX 11 and DirectWrite, not
  wgpu, so each GPUI patch this build carries for the terminal needs a DirectX or Win32 form, and its external-surface
  drawing (`draw_surfaces` in `directx_renderer.rs`) is an empty stub.

## Context (from discovery and the plan review)

- GPUI's Windows backend: `gpui-pre-windows` 0.3.9 (`directx_renderer.rs` with DirectComposition and an empty
  `draw_surfaces`, `directx_devices.rs`, `direct_write.rs`, `events.rs`, `keyboard.rs`, `clipboard.rs`,
  `system_notifications.rs` with toasts, writing `HKCU\Software\Classes\AppUserModelId\<id>` on the first one;
  `platform.rs` `set_app_identity` calls `SetCurrentProcessExplicitAppUserModelID`; `WM_INPUTLANGCHANGE` reported as
  a keyboard layout change).
- This build's GPUI patches (`build/patches/`, applied by `scripts/vendor-gpui.sh`):
  - `gpui-pre`: 0001 external surfaces (a Linux payload in `PaintSurface`), 0002 font metrics, 0003 alpha mask,
    0004 strikeout;
  - `gpui-pre-wgpu`: 0001 dmabuf external surfaces (device setup, import, its own draw pipeline within the content
    mask, CPU uploads, a completion thread; with `gpui-pre/0001` about 2200 lines), 0002 strikeout;
  - `gpui-pre-linux`: native key events and the `text/html` clipboard, xdg-output geometry, X11 window state, keymap
    change.
- Frontend modules whose Windows form plan 1 stubbed: `instance.rs` (D-Bus single instance keyed on the app id,
  `NGTERM_APP_ID`), `notifications.rs` (`org.freedesktop.Notifications`), `quick/host.rs` (layer-shell),
  `global_hotkey.rs`, `remote.rs` (`dbus`, `routes`; the `LinkEvent` seam), `monitors.rs`, `folder_chooser.rs`
  (the portal, for its title and start folder), `appearance.rs` (zbus: the desktop's dark/light), `status_glyph.rs`
  (ashpd: `enable-animations`), `external_texture`, `terminal/surface.rs` (the owner-only screen file).
  Already-existing fallbacks: `remote.rs:84`, `monitors.rs:101`, `quick/host.rs:85`. Windows forms already exist for
  `input_source`, `status_sound.rs`, `terminal/selection_clipboard`.
- The HTML clipboard call site: `crates/ngterm-gpui/src/terminal.rs` (`with_clipboard_html`, about line 992).
- Keymap defaults: `crates/ngterm-core/src/keymap/linux.rs` (`linux_defaults()`, already used on every OS:
  `commands/keymap.rs`, `app_actions.rs`).
- Plain panes already run the PowerShell login shell on Windows (`ngterm-term/src/launch.rs` `Host::shell`); live
  sessions inject `resources/shell-integration/pwsh/ngterm.ps1`.
- Frontend process tests: `crates/ngterm-gpui/tests/` (private weston, Xvfb and bus on Linux, lavapipe).

## Development Approach

- **testing approach**: Regular (code first, then its tests in the same task).
- complete each task fully before moving to the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests**, success and error cases; a Linux process test gets its
  Windows form beside it.
- **CRITICAL: all tests must pass before starting the next task**, on both Windows runners (`rust-windows.yml` on a
  `ci/**` push), not only through `cargo check --target`.
- **CRITICAL: update this plan file when scope changes during implementation.**
- **Local Windows work in the VM** (user, 2026-10-25): a task that needs Windows by hand (a red test or id to
  debug, a probe, an interactive check) starts the Win11-admin VMware VM, works there and stops it at the end of
  the task (memory: windows-macos-debug-in-vmware). The VM is off by default and never left running between tasks;
  CI on the Windows runners stays the gate.
- Linux behavior stays as it is; every Linux test and ratchet id stays green.
- A vendored crate is its crates.io source plus its patches; an edit goes into the patch (CLAUDE.md > Gates).
- Every test instance, here and in the harness, runs under an app id of its own, so no toast, AppUserModelID key,
  mutex or pipe of the installed app's is touched; a test removes the HKCU keys its app id left at teardown.

## Testing Strategy

- **unit tests**: per task, on Linux and on both Windows runners.
- **process tests**: `crates/ngterm-gpui/tests` get a Windows support module first (Task 2): a private state dir and
  app id, WARP (the runners' only adapter), the runner's interactive session, no Xvfb.
- **conformance suite**: targeted runs (`--ratchet-only --also <files>`) where a task claims a capability; the full
  run once at the end (Task 19) on `conformance-windows`, plus `run-linux.sh` once on Linux.

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- keep the plan in sync with the work done

## Solution Overview

- **Vendor `gpui-pre-windows`** as `gpui-pre-linux` was: the verified crates.io archive plus
  `build/patches/gpui-pre-windows/`, applied and checked by `scripts/vendor-gpui.sh`; a workspace dependency and a
  `cfg(windows)` dependency of `ngterm-gpui`.
- **External surfaces on DirectX**: `PaintSurface`'s payload gets a Windows form, a D3D11 texture shared by NT handle
  with a shared fence, or CPU pixels with damaged rectangles. The DirectX renderer gets an HLSL pipeline that samples a
  surface's texture within the content mask, as the wgpu patch's draw pipeline does; it opens a shared texture on its
  device, waits on the fence, and reports `Presented` once the GPU finished with it, `Lost` when it cannot know. The
  import capabilities name the adapter's LUID. Plan 3's WebView2 frames are the producer; this plan's tests produce
  frames themselves.
- **Text**: strikeout from `DWRITE_FONT_METRICS`; font metrics and alpha masks are core patches, verified on DirectX.
- **Native key events**: each key carries its scan code, virtual key, the layout's level-0 character (`ToUnicodeEx`
  over an empty key state, with the flag that keeps dead-key state), consumed modifiers, Caps and Num Lock, and AltGr
  told apart from Ctrl+Alt. The clipboard offers CF_HTML for `copy_to_clipboard:html` and `:mixed`.
- **Each stubbed module's Windows form** (Technical Details).
- **References to study in their tasks**: Windows Terminal (MIT) for its quake mode, key handling and single-instance
  forwarding; WezTerm (MIT, `window/src/os/windows`) for the key-event data and AltGr; the wgpu patch for the surface
  draw pipeline. Zed's code beyond GPUI is GPL: ideas only.

## Technical Details

- **Single instance**, keyed on the app id as on Linux (the D-Bus name there): a named mutex
  `Local\<app id>` decides the primary; the primary serves an owner-only pipe named after the app id
  (`ngterm_core::owner_pipe`); a second launch writes its arguments and the foreground grant
  (`AllowSetForegroundWindow` for the primary's pid) and exits before GPUI starts and before any model read or write.
- **Quick terminal**: a borderless, topmost tool window (`WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, no taskbar button),
  placed by `ngterm-app::quick_placement` on the monitor under the cursor (its work area), with the Linux form's slide;
  hidden, not destroyed, on dismiss; the global hotkey through `RegisterHotKey` on a thread with a message loop,
  posting to the main loop.
- **Toasts**: the AppUserModelID derives from the app id (the default id gives the installed app's, which plan 4's
  shortcut carries), set through GPUI's `cx.set_app_identity`; GPUI's `SystemNotification` with the slot identity as
  tag and group, so a repeat in a slot replaces the last; a click reveals the pane and raises the window; withdrawals
  remove by tag.
- **Remote-link watchers**: the Network List Manager's `ConnectivityChanged` as reachability,
  `WM_POWERBROADCAST`/`PBT_APMRESUMEAUTOMATIC` as the resume, both into the `LinkEvent` seam.
- **Monitors and saved frames**: `EnumDisplayMonitors` with work areas in logical pixels;
  `GetWindowPlacement`/`SetWindowPlacement` for the restored frame and the minimized and maximized state.
- **Appearance and animations**: GPUI's `window_appearance` and its change callback (`WM_SETTINGCHANGE`
  `ImmersiveColorSet`) for dark/light; `SPI_GETCLIENTAREAANIMATION` for `enable-animations`.
- **Folder chooser**: `IFileOpenDialog` with `FOS_PICKFOLDERS`, `SetTitle` and `SetFolder`, since GPUI's
  `prompt_for_paths` takes neither a title nor a start folder.

## What Goes Where

- **Implementation Steps**: code, patches, tests, CI and docs in this repo.
- **Post-Completion**: manual checks in the Win11 VM.

## Implementation Steps

### Task 1: Vendor gpui-pre-windows

**Files:**
- Modify: `scripts/vendor-gpui.sh`
- Create: `vendor/gpui-pre-windows/` (generated by the script), `build/patches/gpui-pre-windows/` (empty at first)
- Modify: `Cargo.toml` (`[patch.crates-io]`, the workspace dependency), `crates/ngterm-gpui/Cargo.toml`
  (`cfg(windows)` dependency)

- [ ] the script fetches and verifies `gpui-pre-windows` 0.3.9 as it does the others, applies its patch directory,
      and `--check` covers it
- [ ] the workspace builds against the vendored copy on Windows; nothing changes on Linux
- [ ] a selftest of the script's `--check` refusing an edit made in `vendor/gpui-pre-windows` outside the patches
- [ ] run `scripts/vendor-gpui.sh --check` on Linux and both Windows runners, and the gates - must pass before next
      task

### Task 2: The frontend's process-test support on Windows

**Files:**
- Create: `crates/ngterm-gpui/tests/support/windows.rs`
- Modify: `crates/ngterm-gpui/tests/support/mod.rs`, `.github/workflows/rust-windows.yml`

- [ ] the support module's Windows form: a private state dir and app id, the scrubbed environment, the runner's
      session, teardown removing the app id's HKCU keys
- [ ] one existing process test (`tests/quit.rs` or `build_identity.rs`) running through it on Windows as the proof
- [ ] `rust-windows.yml` runs every process test that has a Windows form, on both arches
- [ ] run the job - must pass before next task

### Task 3: External surfaces on DirectX: the payload, the CPU path and the draw pipeline

**Files:**
- Modify: `build/patches/gpui-pre/0001-external-surfaces.patch` (the Windows payload in the scene and window half)
- Create: `build/patches/gpui-pre-windows/0001-external-surfaces.patch`

- [ ] the Windows payload in `PaintSurface`: a shared texture by NT handle with its fence, or CPU pixels with damage
- [ ] the renderer's surface pipeline (HLSL): a surface's texture sampled within its content mask, as the wgpu
      patch's; `draw_surfaces` no longer a stub
- [ ] the CPU path: pixels uploaded by damaged rectangle into a texture of the renderer's, `Presented` after the GPU
      used it
- [ ] write tests in the patch's crate for the CPU path (full frame, damaged update, content-mask clipping)
- [ ] run tests on Linux (unchanged) and both Windows runners - must pass before next task

### Task 4: External surfaces on DirectX: shared textures, fences and completion

**Files:**
- Modify: `build/patches/gpui-pre-windows/0001-external-surfaces.patch`

- [ ] a shared texture opened on GPUI's device (`OpenSharedResource1`), the shared fence waited on before the draw,
      `Presented` after the GPU finished (a completion thread on the renderer's fence), `Lost` on a device removal
- [ ] the import capabilities: the adapter's LUID and the formats it draws; the import stats
- [ ] write tests: a frame from a texture made on the same device presented and released; a LUID mismatch refused
      (the LUID faked, since the runners have WARP only); a device lost reported
- [ ] run tests on both Windows runners - must pass before next task

### Task 5: The frontend's external textures on Windows

**Files:**
- Modify: `crates/ngterm-gpui/src/external_texture.rs` (and its `external_texture/` modules)
- Modify: `crates/ngterm-gpui/tests/external_texture/`

- [ ] `external_texture`'s Windows form, so a producer on the same adapter hands frames in, with the CPU fallback
- [ ] the external-texture tests' Windows form: a test producer's frames presented and released, the CPU fallback's
      damaged upload, the stats read back
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 6: Strikeout, font metrics and alpha masks on DirectWrite

**Files:**
- Create: `build/patches/gpui-pre-windows/0002-font-strikeout.patch`
- Modify: `crates/ngterm-gpui/src/terminal/` (cell metrics, only where DirectWrite's numbers need it)
- Modify: `crates/ngterm-gpui/tests/sprite_golden.rs`, `crates/ngterm-gpui/tests/golden/`

- [ ] `font_strikeout` from `DWRITE_FONT_METRICS`
- [ ] the terminal's cell size, baseline, underline and strikethrough from DirectWrite's metrics follow Ghostty's
      formulas; the bundled fonts (`scripts/fetch-fonts.sh`) are what the tests measure
- [ ] the sprite goldens (procedural alpha masks) pass on DirectX unchanged; glyph goldens get Windows references only
      where the rasterizer legitimately differs, each difference recorded
- [ ] write tests for the strikeout and metrics read on Windows
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 7: Native key events on Windows

**Files:**
- Create: `build/patches/gpui-pre-windows/0003-native-key-events.patch`
- Modify: `crates/ngterm-gpui/src/terminal/input.rs`

- [ ] study WezTerm's and Windows Terminal's key handling; `NativeInput::Key` on Windows carries the scan code, the
      virtual key, the level-0 character, consumed modifiers, Caps/Num Lock, repeat and composition flags
- [ ] AltGr told apart from Ctrl+Alt (the synthetic LCtrl before RAlt); dead keys and IME composition reach the
      terminal as text, not as keys
- [ ] `terminal/input.rs` maps it into libghostty-vt's key encoder as the XKB path does (physical keys, Kitty flags)
- [ ] write tests for the encoder input on Windows: US, a layout with AltGr (German), a dead key, a non-Latin layout's
      level-0 for binds, autorepeat
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 8: HTML clipboard and the keymap-change signal on Windows

**Files:**
- Modify: `build/patches/gpui-pre-windows/0003-native-key-events.patch` (CF_HTML beside the key data, as on Linux)
- Create: `build/patches/gpui-pre-windows/0004-keymap-change.patch` (only if the check below needs it)
- Modify: `crates/ngterm-gpui/src/terminal.rs` (the HTML clipboard call site)
- Modify: `crates/ngterm-gpui/tests/clipboard_html.rs`, `crates/ngterm-gpui/tests/keyboard_layout.rs`

- [ ] `with_clipboard_html` writes CF_HTML with its header offsets beside CF_UNICODETEXT, or alone; the call site
      compiled on Windows
- [ ] check whether adding or removing a keyboard layout reaches `on_keyboard_layout_change`; if not, the patch
      reports it (the enabled list re-read on `WM_INPUTLANGCHANGE` and on a settings change)
- [ ] the keyboard-layout lock re-reads its layouts on that callback, as on Linux
- [ ] write the clipboard and keyboard-layout tests' Windows forms
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 9: Keymap defaults named for what they are, checked against Windows' reserved chords

**Files:**
- Move: `crates/ngterm-core/src/keymap/linux.rs` → `crates/ngterm-core/src/keymap/ctrl_shift.rs` (`linux_defaults()`
  renamed to match, its callers in `commands/keymap.rs` and `app_actions.rs`)

- [ ] the rename only: every OS keeps reading the same defaults
- [ ] a check of the defaults against chords Windows reserves (Win+…, Alt+Tab, Ctrl+Alt+Del, Alt+F4 as window close,
      Ctrl+Esc); a collision changed for both OSes or recorded with its reason
- [ ] write tests that the defaults load with no diagnostics and that the reserved check holds
- [ ] run tests - must pass before next task

### Task 10: Single instance on Windows

**Files:**
- Create: `crates/ngterm-gpui/src/instance/windows.rs`
- Modify: `crates/ngterm-gpui/src/instance.rs`, `crates/ngterm-gpui/src/activation.rs`
- Modify: `crates/ngterm-gpui/tests/instance.rs`

- [ ] the named mutex and the activation pipe keyed on the app id (Technical Details); the forwarded launch exits
      before GPUI starts and before any model read or write, its log lines appended only
- [ ] the primary raises its window with the foreground grant the launcher gave
- [ ] write the instance tests' Windows form: a second launch forwards and exits, two app ids keep apart, a mutex
      abandoned by a crash does not block the next launch
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 11: Window state, monitors, saved frames and the titlebar on Windows

**Files:**
- Modify: `crates/ngterm-gpui/src/{window,window_frames,monitors,titlebar,chrome}.rs`
- Modify: the window-state process tests (`crates/ngterm-gpui/tests/`, Windows forms beside `x11_window.rs`)
- Modify: `conformance/docs/rust-build.md`, `crates/ngterm-gpui/src/app.rs` (capability claims per OS)

- [ ] minimized and maximized state read and set through Win32; the frame saved at quit is the restored frame
- [ ] monitors with their work areas in logical pixels; a restored window kept on a monitor that still exists
- [ ] the titlebar on Windows: GPUI's caption buttons and snap layouts, the app's own titlebar content as on Linux
- [ ] write tests for the saved frame's round trip, a missing monitor, minimized at quit
- [ ] claim `window-geometry` and `window-minimize` on Windows after a targeted run
      (`run-windows.sh --ratchet-only --also tests/protocol/test_window.py`) passes them; `--must-pass` on both in
      `conformance-windows.yml`
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 12: Quick terminal and the global hotkey on Windows

**Files:**
- Create: `crates/ngterm-gpui/src/quick/windows.rs`
- Modify: `crates/ngterm-gpui/src/quick/host.rs`, `crates/ngterm-gpui/src/global_hotkey.rs`
- Modify: `crates/ngterm-gpui/tests/quick_terminal.rs`, `crates/ngterm-gpui/tests/global_hotkey.rs`

- [ ] study Windows Terminal's quake mode; the quick terminal as a topmost tool window (Technical Details) with
      `quick_placement`'s position and size, its slide, its dismiss on focus loss as configured
- [ ] the global hotkey through `RegisterHotKey`; a chord another app holds is reported with its reason
- [ ] write the quick-terminal and hotkey tests' Windows forms (keys sent with `SendInput` in the runner's session)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 13: Toast notifications on Windows

**Files:**
- Create: `crates/ngterm-gpui/src/notifications/windows.rs`
- Modify: `crates/ngterm-gpui/src/notifications.rs`, `crates/ngterm-gpui/src/main.rs` (`set_app_identity`)
- Modify: `crates/ngterm-gpui/tests/notifications.rs`

- [ ] the AppUserModelID from the app id through `cx.set_app_identity` (Technical Details); one function shared with
      plan 4's package
- [ ] posts, replacements by slot and withdrawals through GPUI's `SystemNotification`; a click reveals the pane that
      fired and raises its window
- [ ] write the notification tests' Windows form under a per-test app id (the toast history read back through
      `ToastNotificationManager.History`, the HKCU key removed at teardown)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 14: Remote-link watchers on Windows

**Files:**
- Create: `crates/ngterm-gpui/src/remote/windows.rs`
- Modify: `crates/ngterm-gpui/src/remote.rs`, `crates/ngterm-gpui/tests/remote_links.rs`

- [ ] the Network List Manager's connectivity events and the power resume into `LinkEvent`; where the Network List
      Manager cannot be reached, the network reads as always reachable (logged), as on Linux
- [ ] write tests for the Windows source starting, reporting and stopping cleanly (the folding into retries is
      already tested with a fake source)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 15: Appearance, animations, the screen file and the folder chooser on Windows

**Files:**
- Modify: `crates/ngterm-gpui/src/{appearance,status_glyph,folder_chooser}.rs`, `crates/ngterm-gpui/src/terminal/surface.rs`
- Modify: `crates/ngterm-gpui/tests/appearance.rs`

- [ ] the system's dark/light followed through GPUI's `window_appearance` and its change callback
- [ ] `enable-animations` from `SPI_GETCLIENTAREAANIMATION`, re-read on its settings change
- [ ] the screen file (`write_screen_file`) owner-only through `ngterm_core::owner_only`
- [ ] the folder chooser through `IFileOpenDialog` with its title and start folder
- [ ] write tests for each (an appearance change followed, the animation flag read, the screen file's ACL, the chooser's
      options set)
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 16: The PowerShell integration in plain panes, and the stubs left

**Files:**
- Modify: `crates/ngterm-term/src/launch.rs` (only if plain panes do not yet inject `ngterm.ps1`)
- Modify: `crates/ngterm-gpui/src/{input_source,status_sound}.rs`, `crates/ngterm-gpui/src/terminal/selection_clipboard.rs`
  (only where a real window shows a gap)

- [ ] check what plain panes' PowerShell login shell is missing against live sessions' (the integration injected, its
      cwd, prompt marks and foreground-job reports reaching the core); add only that
- [ ] the existing Windows forms of the keyboard-layout lock, the status sounds and the selection clipboard checked in
      a real window; gaps fixed
- [ ] no plan-1 stub left except the HTML overlays: the "not available on Windows yet" log and the older fallbacks
      (`remote.rs`, `monitors.rs`, `quick/host.rs`) are gone or replaced
- [ ] write tests for each fix
- [ ] run tests on Linux and both Windows runners - must pass before next task

### Task 17: CLAUDE.md and README

**Files:**
- Modify: `CLAUDE.md`, `README.md`

- [ ] the crate map: `ngterm-gpui` on Linux and Windows (HTML overlays on Windows in plan 3)
- [ ] README: the Windows frontend's forms (single instance, quick terminal, toasts and the AppUserModelID, the
      keymap, appearance), the vendored `gpui-pre-windows`
- [ ] run the gates - must pass before next task

### Task 18: The capability claims at their Windows level

**Files:**
- Modify: `conformance/docs/rust-build.md`, `.github/workflows/conformance-windows.yml`

- [ ] every capability Windows now claims is in rust-build.md's profile with its OS and `--must-pass` in the Windows job
- [ ] run the selftests - must pass before next task

### Task 19: Verify acceptance criteria

- [ ] push to a `ci/**` branch; wait (in the background) for `conformance-windows` on both arches
- [ ] add the newly green ids with `check_ratchet.py --list conformance/rust-passing-windows.txt --add`
- [ ] the expected set: the Linux ratchet's protocol, cli and behaviors ids minus those whose tests drive HTML pages or
      the MSI; each id of the expected set still red is fixed or recorded here with ⚠️ and its reason
- [ ] `conformance/tools/run-linux.sh` once in the background; no Linux ratchet id went red, newly green ids added
- [ ] every gate on Linux and `rust-windows.yml` green

### Task 20: [Final] Update documentation

- [ ] README and CLAUDE.md match what was built
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

**Manual verification** (in the Win11 VM, started for it and stopped after):
- typing in a German and a Russian layout, AltGr chords, a dead key; IME (Japanese) composition in a pane
- the quick terminal on two monitors at different scales; a toast's click while the window is minimized
- a resume from sleep retries a remote link; switching Windows to dark mode

**External system updates**: none.
