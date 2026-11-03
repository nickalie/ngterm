# E2E UI tests and measured coverage

## Overview

ngterm gets end-to-end UI tests in the spirit of Playwright: tests find controls by id, press, click, drag, scroll,
type, wait on read-back, and leave a trace when they fail. Two coverage figures are measured: the Rust code the e2e run
exercises, and the catalog's features and ui ids a passing test covers.

No new framework is needed. The conformance suite already has the Playwright-shaped layer, `conformance/tests/ui`:
tests find elements by upstream's accessibility ids (`ui-ids.yaml`), press them, post key chords, set and read text,
open menus and tabs. It runs on upstream's macOS release through the `ax` driver. On the Rust build it skips as
unsupported: `drivers/base.py` reserves `atspi` (Linux) and `uia` (Windows), and neither is written. The runners do not
collect it either (`run-linux.sh` and `run-windows.sh` take protocol, cli and behaviors). This plan:

- writes the two drivers on the accessibility tree GPUI already publishes (AccessKit), and puts the ui layer into both
  runners and their ratchets;
- adds pointer input (move, mouse click, double-click, drag, scroll) and text typing to the driver API;
- adds failure traces;
- builds an instrumented stage whose suite run yields Rust code coverage, held by a per-crate floor ratchet;
- reports which ui ids a passing Rust test exercised, beside the existing catalog coverage gate.

The tests stay valid on upstream's release: a ui test is accepted only once it is green on reference-macos, as the
suite's reference discipline already requires.

## Context (from discovery and the plan review)

- `conformance/agterm_conformance/drivers/base.py`: the driver API (`find`/`all`/`absent` with `name=`/`enabled=`
  waits and an inventory on timeout, `click` = the accessibility press, `press` chords of lowercase printable keys,
  `text`, `set_text`, `focus`, `hover`, `menu`, `menu_item`, `tab`), the abstract per-OS primitives
  (`_press_element`, `_post_chord`, `_set_text`, `_focus`, `_hover`, `_menu_command`, `_menu_item`, `_tab`, ...),
  `PLATFORM_DRIVERS`, `IMPLEMENTED`, `NoDriver`.
- `conformance/agterm_conformance/drivers/ax.py`: the macOS driver, imported lazily (`_bindings`); already posts
  CGEvent mouse moves for `hover`.
- `conformance/tests/ui/`: dialogs, html_overlay, live, settings, sidebar_rows, titlebar, welcome. Several instances
  live at once in one pytest process (module-scoped `shared_app`, `seeded`/`live` fixtures beside `launch()` inside
  tests).
- `conformance/agterm_conformance/plugin.py`: `ui_skip_reason`, `ui`/`ui_for` (a driver made from a pid), the instance
  fixtures (`shared_app`, `launch`, `relaunch`), `CliProcess` (`ngtermctl` subprocesses, SIGTERM then kill on close),
  `--nodeid-report` (one line per test, no markers; refused together with `--coverage-gate`).
- `conformance/agterm_conformance/impls/base.py`, `impls/rust.py`: each instance on its own headless weston and private
  session bus; `isolated_runtime_env` forces `GDK_BACKEND=wayland`; `check_launch_env` refuses `DISPLAY` and
  `AT_SPI_BUS_ADDRESS` for a headless instance; SIGTERM quit; the Windows quit window and job object reap.
- `conformance/agterm_conformance/wait.py`: `wait_until`, `wait_text` (polls `session.text`, reports the last text).
- `conformance/agterm_conformance/coverage.py`: the catalog coverage gate (`covers()` against `catalog.yaml` and
  `ui-ids.yaml`), `--coverage-gate` in `conformance-gate.yml`.
- `conformance/tools/run-linux.sh`, `run-windows.sh` (they write `report.md`), `check_ratchet.py` (a listed id that
  was not collected is only a warning); `conformance/rust-passing.txt`, `rust-passing-windows.txt`.
- `conformance/selftests/`: host-free harness tests; `test_drivers.py` already has a `FakeDriver`.
- `vendor/gpui-pre-linux`: the X11 backend wires `accesskit_unix` (feature `x11`, `a11y_init` in `x11/window.rs`), but
  nothing calls `a11y_update_window_bounds` (defined in `gpui-pre/src/platform.rs` and both Linux backends), so AT-SPI
  extents are window-local. `accesskit_windows` maps to the screen itself (`ClientToScreen`).
- AccessKit's author id is AT-SPI's `AccessibleId` (`Atspi.Accessible.get_accessible_id`) and UIA's `AutomationId`.
  `accesskit_unix` takes its bus from `AT_SPI_BUS_ADDRESS`, else `org.a11y.Bus.GetAddress`, registers with the
  registry (`at-spi2-registryd` must run) and publishes only while `org.a11y.Status.IsEnabled` is on.
- libatspi's Python API (`gi.repository.Atspi`) has `init()` with no bus argument: one a11y bus per process, read from
  the environment at `init`.
- `crates/ngterm-gpui/tests/support/mod.rs`: the private-Xvfb precedent (`Xvfb -displayfd 1 -nolisten tcp`, no window
  manager).
- `crates/ngterm-gpui/src/main.rs` `exit_now`: `_exit` on Unix, `TerminateProcess` of itself on Windows, after the
  orderly shutdown. Both skip the LLVM profile runtime's exit-time write. `process::exit` paths run it and need nothing.
- `ui-ids.yaml` is generated from upstream: there is no id for a pane's surface, a split divider or the sidebar's scroll
  area. GPUI's menus are in-window popups under a title-bar button (`app_menu.rs`); Settings is an in-window modal.
- CI: `.github/workflows/linux.yml` (`conformance` job: Arch container, a bash/zsh/fish matrix, `run-linux.sh`),
  `cross-platform.yml` (`windows` job, `run-windows.sh -- --must-pass live-any-shell`), `reference-macos.yml` (ui with
  `--reruns`), `conformance-gate.yml`. `rust-toolchain.toml` lists clippy and rustfmt only.

## Decisions

- **Real accessibility, real input, inside the existing suite.** Elements come from the OS accessibility API over the
  AccessKit tree; input goes through the OS (XTest, `SendInput`, CGEvent). No test-only command surface or code path in
  the app. An accessibility gap a driver exposes is fixed in `ngterm-gpui` (or a recorded GPUI patch) as real
  accessibility work.
- **Linux ui instances run on X11, each on its own Xvfb, all on one run-wide private a11y bus.** Wayland gives a client
  neither global coordinates nor input injection. libatspi binds one a11y bus per process, so the pytest process
  starts one private session bus with `at-spi-bus-launcher` for the run, sets its own `AT_SPI_BUS_ADDRESS` to that bus
  before `Atspi.init()`, and passes the same address to each ui instance. Each instance keeps its own session bus and
  gets its own Xvfb (`-displayfd`, no TCP, no fixed display number), so windows never overlap and pointer input lands
  on the right app; the driver finds its app on the shared a11y bus by pid and sends XTest input to that instance's
  display. The protocol, cli and behaviors layers stay on weston. Rendering stays on lavapipe.
- **The launch mode is chosen at launch, by layer.** Every instance a `tests/ui` module launches (its module fixtures,
  `launch()`, `relaunch` keeping the mode) is an X11 instance; elsewhere weston. The `ui` fixture takes the instance,
  not a pid, so the driver knows the display.
- **Safety refusals.** The harness refuses an a11y bus address it did not start, a `DISPLAY` that is not the instance's
  own Xvfb, a ui run in a non-headless display mode, and an `Atspi.init()` in a process whose a11y bus is not the run's.
  `GSETTINGS_BACKEND=memory` keeps the a11y toggle off dconf.
- **AT-SPI screen extents come from GPUI.** A `gpui-pre-linux` patch (through `build/patches/gpui-pre-linux` and
  `scripts/vendor-gpui.sh`) calls `a11y_update_window_bounds` when the window is configured, so `Component.get_extents(SCREEN)`
  is in screen coordinates for any AT client, not only the suite.
- **`click` stays the accessibility press.** Pointer actions get their own names (`mouse_click`, `mouse_double_click`,
  `move`, `drag`, `drag_by`, `scroll`), so the tests green on the reference keep their meaning.
- **Snapshot elements.** Tests keep `Element` snapshots. A pointer action re-reads bounds at action time; a stale
  element is found again once by its id and name (role and name for an element with no id), else the action fails with
  `ElementNotFound`. Nothing retries after input was sent.
- **Waits reuse what the suite has.** `find(..., name=, enabled=)` and `wait_text` already wait with read-back; only the
  missing waits are added to `wait.py`/`base.py`: a count, a regex name, the cursor. No separate `expect` module.
- **No reruns on the Rust ui runs.** They wait on read-back like the other layers; reference-macos keeps its existing
  `--reruns` for the ax races it documents.
- **The ui layer joins both runners' default layers** once its tests are green there, so the ratchet holds ui ids in
  every full run (all three Linux shell entries).
- **Standard Python dependencies, imported lazily** (as `ax.py` does), so the host-free self-tests need none: Linux
  PyGObject with the distribution's `Atspi-2.0` typelib, python-xlib for XTest; Windows comtypes over
  `UIAutomationCore`, ctypes for `SendInput`; Pillow's `ImageGrab` for trace screenshots.
- **Missing setup fails the run, never skips**: no Xvfb, no `at-spi-bus-launcher` (searched in the distributions'
  libexec paths, not only `PATH`), no `at-spi2-registryd`, no typelib, no UIA → exit 2 naming the step.
- **Code coverage comes from cargo-llvm-cov** (`show-env` for the build, `report` for the merge, its standard install
  and the `llvm-tools` component on CI). Profiles are pooled per binary (`%Nm`, online merge) so thousands of
  `ngtermctl` calls do not leave thousands of files. A `#[cfg(coverage)]` call of `__llvm_profile_write_file()` runs in
  `exit_now` before `_exit`/`TerminateProcess`.
- **The coverage job judges floors only.** The ratchet of ids stays the normal, uninstrumented job's, so instrumented
  slowness never turns an id red.
- **Coverage is gated by a ratchet**: one floor file per OS, line % per workspace crate, a drop of more than 0.5 pt
  fails, floors are only ever raised (`--add`), the first run on `main` seeds them.
  ➕ Amended in Task 12: one run's line % of the same build varies by more than 0.5 pt (Linux ngterm-gpui 63.80-64.38%
  over three runs), so the floors are seeded from the per-crate minimum of three same-build CI runs and `--add` takes a
  batch of at least two summaries, raising each floor to the batch's lowest.
- **Not in scope**: visual snapshot assertions, video, an HTML trace viewer, tests whose target has no upstream id and
  no reachable stand-in (Task 7 decides each one from upstream).

## Development Approach

- **testing approach**: TDD. Each task writes its tests first: harness self-tests against fakes for harness code, ui
  tests for app behavior (green on reference-macos before they are expected green on Rust), Rust tests for Rust
  changes, then the code that makes them pass. A driver's body can only be tested on its host; its host-free parts
  (role, key and input-structure mappings) are tested first, the rest by the ui layer.
- complete each task fully before moving to the next; small, focused changes.
- **every task includes new or updated tests** for the code it changes, success and error paths, listed as their own
  checklist items; **all of a task's tests pass before the next task starts.**
- gates follow CLAUDE.md > Porting step 6 and Gates: clippy on the touched crates and their dependents, tests on the
  touched crates, the self-tests, a partial suite run over the ids the task touches. The whole workspace and the full
  suite are CI's. Newly green ids go into the ratchet with `check_ratchet.py --add`; a ratchet id never goes red.
- Windows code is proven on the Windows CI runner (a `ci/win-<task>` branch), not by `cargo check`; Windows GUI
  debugging goes in the throwaway VM only. The ax side is proven on `reference-macos.yml`.
- no dev instance, probe or `ngtermctl` call touches the user's running ngterm, display, session bus or a11y bus
  (CLAUDE.md > Never touch the user's running ngterm).
- update this plan when scope changes during implementation.

## Testing Strategy

- **harness self-tests** (`conformance/selftests`, host-free, `conformance-gate.yml`): the existing `FakeDriver`
  extended with bounds and recorded pointer/typing primitives; the X11 launch mode, its env and refusals, its reap,
  against fakes; the trace writer; the coverage env and `lost-profiles.txt`; `check_coverage.py`'s floor logic; the
  "ui ids exercised" count.
- **e2e** (`conformance/tests/ui`): the existing ui tests on Linux and Windows; new tests for the pointer interactions
  upstream has and for typing into a pane, each green on reference-macos first.
- **Rust tests**: for each `ngterm-gpui` accessibility fix (the element's role, name, value), and the GPUI bounds patch
  (an X11 window test in the existing `tests/x11_window.rs` style reading the bounds AccessKit receives).

## Progress Tracking

- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update the plan if implementation deviates from the original scope

## Solution Overview

```
pytest process (conformance)
  ├─ Linux ui run: one private session bus + at-spi-bus-launcher (+ registryd) → AT_SPI_BUS_ADDRESS, Atspi.init()
  ├─ ui fixture(instance) ─ driver_class(impl, platform)
  │     ├─ ax.py     macOS    AX tree, AXPress, CGEvent keys and pointer
  │     ├─ atspi.py  Linux    Atspi apps by pid on the run's a11y bus, XTest on the instance's own Xvfb
  │     └─ uia.py    Windows  UIA tree by process id, patterns, SendInput
  ├─ drivers/trace.py  per-test action log, socket and ngtermctl transcripts; on failure screenshot, tree, panes, log
  └─ adapter (impls/rust.py)
        tests/ui instances (Linux): own Xvfb (-displayfd) + own session bus + the run's AT_SPI_BUS_ADDRESS
        other instances: own weston + own session bus (unchanged)
        --coverage: LLVM_PROFILE_FILE (pooled per binary) for every launched process
run-linux.sh / run-windows.sh --coverage
  instrumented build (cargo llvm-cov show-env) → stage → suite → cargo llvm-cov report → coverage/ → check_coverage.py
```

## Technical Details

**Driver API additions** (`drivers/base.py`; tests see only these):

- `move(el, offset=(0.5, 0.5))`, `mouse_click(el, button="left"|"right", offset=...)`, `mouse_double_click(el, ...)`,
  `drag(src, dst, src_offset=..., dst_offset=..., steps=N)`, `drag_by(el, dx, dy, ...)`, `scroll(el, dy, dx=0)`,
  `type_text(text)` (any printable text, case and shifted symbols included, through the OS's text input).
- Per-OS primitives: `_bounds(element) -> (x, y, w, h)` in screen coordinates, `_pointer_move(x, y)`,
  `_pointer_button(button, down)`, `_wheel(dx, dy)`, `_type_text(text)`. The base class composes them (drag = move,
  down, stepped moves, up). `ax.py` implements them in the task that adds them, so the reference keeps instantiating.
- `_all_elements()` supplies snapshots including controls without ids for role/name relocation.
  `_double_click(button)` defaults to two down/up pairs; AX overrides it with explicit CGEvent click counts.
- Waits added beside the existing ones: `count(ui_id, n)` in the driver, a regex `name=` for `find`, `wait_cursor(ctl,
  target, column)` in `wait.py` over `surface.cursor`.

**Linux ui launch**:

- run-wide, started once by the plugin when a ui test is collected: a private session bus, `at-spi-bus-launcher` on
  it (which starts the a11y bus and activates `at-spi2-registryd`), `IsEnabled` set, its address from
  `org.a11y.Bus.GetAddress` exported to the pytest process before `Atspi.init()`; stopped and reaped (launcher, a11y
  dbus-daemon, registryd) at session end.
- per ui instance: `Xvfb -displayfd` with the screen size the weston instances use, the instance's own session bus,
  `DISPLAY` its Xvfb, `WAYLAND_DISPLAY` unset, `GDK_BACKEND=x11`, `AT_SPI_BUS_ADDRESS` the run's, `GSETTINGS_BACKEND=
  memory`; `check_launch_env` lets through exactly these and refuses any other `DISPLAY`/a11y address.
- the driver waits for an application on the a11y bus whose process id is the instance's, under a deadline
  (`DriverError` carrying the applications it saw), and opens python-xlib on the instance's display for XTest.
- Task 2 settles whether input focus on a bare Xvfb needs a window manager; one comes in only if a test needs it.

**Traces** (`reports/<run>/traces/<nodeid-slug>/`): `screenshot.png` (Pillow `ImageGrab`, `xdisplay=` the instance's
Xvfb on Linux; on macOS it needs the Screen Recording grant, else it lands in `trace-errors.txt`), `a11y-tree.txt`,
`actions.jsonl`, `socket.jsonl`, `cli.jsonl` (each `ngtermctl` argv, exit code, output), `panes/<sid>.txt`, `app.log`,
`trace-errors.txt` when collecting failed. `--trace-mode=off|on-failure|always`, default `on-failure`. Logs are kept in
memory per test and dropped on pass. Every layer gets socket, cli, panes and log; screenshot and tree only where a
driver exists.

**Coverage**:

- Build: `cargo llvm-cov show-env` exported, the release build in cargo-llvm-cov's target dir, staged by `stage.sh`
  into a stage dir of its own (`conformance-stage-cov`); `cfg(coverage)` declared in the workspace's `check-cfg`.
  `cargo llvm-cov clean --workspace` before the run.
- Run: `LLVM_PROFILE_FILE` for every process the adapter and `CliProcess` start, in the directory `report` reads,
  pooled per binary (`%Nm`). A process the harness had to SIGTERM-without-quit or kill (`CliProcess.close`, the
  Windows job close, the reap) is recorded in `lost-profiles.txt`.
- Report: `cargo llvm-cov report` into `coverage/lcov.info`, `coverage/html/`, `coverage/summary.md` (line, region and
  function % per workspace crate and total; `vendor/`, `-sys` and generated bindings excluded). The runners' `report.md`
  carries the summary and the catalog line.
- Floors: `conformance/coverage-floor-linux.txt`, `coverage-floor-windows.txt`, one `crate line%` per line.
  `tools/check_coverage.py --summary ... --floor ...` exits 1 naming each crate more than 0.5 pt below its floor with
  both numbers, 0 otherwise, 2 on a malformed file; `--add` raises floors and never lowers one; a crate new to the
  workspace is added, a removed one dropped.

**Catalog line**: `nodeids.jsonl` lines gain the test's `covers` targets; `coverage.py` counts the ui ids covered by at
least one passed test.

## What Goes Where

- **Implementation Steps**: harness code, drivers, adapter, runner scripts, the GPUI patch, app accessibility fixes,
  the coverage flush, CI workflows, docs.
- **Post-Completion**: what needs a human outside the repo.

## Implementation Steps

### Task 1: Pointer and typing API, with the ax primitives

**Files:**
- Modify: `conformance/agterm_conformance/drivers/base.py`
- Modify: `conformance/agterm_conformance/drivers/ax.py`
- Modify: `conformance/agterm_conformance/wait.py`
- Modify: `conformance/selftests/test_drivers.py`

- [x] write self-tests first on the extended `FakeDriver`: `move`, `mouse_click` (both buttons), `mouse_double_click`,
      `drag`, `drag_by`, `scroll`, `type_text`, offsets, bounds re-read at action time; a disabled element waits and
      then fails without sending input
- [x] write self-tests for the stale path: found again by id and name (a shared id like `session-row` picks the row with
      the snapshot's name), by role and name without an id, else `ElementNotFound`
- [x] write self-tests for `count`, the regex `name=`, `wait_cursor` (pass, and timeout with the last value)
- [x] add the API and the primitives to `base.py`, the waits to `base.py`/`wait.py`
- [x] implement the primitives in `ax.py` (CGEvent mouse down/up/drag/scroll, unicode text events)
- [x] run the self-tests; a `reference-macos.yml` run of the existing ui layer stays green

**Decision:** `surface.cursor` exposes only a zero-based column, so `wait_cursor` accepts
`(client, target, column, timeout=..., **args)` and returns the complete result. It preserves
`paneID` and other target arguments unchanged; no row is inferred or added to the protocol.

**Validation:** 894 harness self-tests passed (18 platform-specific skips), catalog coverage gate passed,
and [reference UI run 41478357613](https://github.com/nickalie/ngterm/actions/runs/41478357613) passed
all 44 tests with no skips or reruns. The CI-only branch `ci/mac-task1-input` removes the unrelated
protocol/CLI/behavior step from the reference workflow; the task code and UI command are identical.
The original full reference run was canceled before UI after spending over 16 minutes in the other layers.
The first scoped UI run exposed two pre-existing selectors that expected `palette-editor-hint`, while
SwiftUI exposes the same hint as `command-palette` text. A shared test-only selector now recognizes either
authored identifier without changing the semantic assertions or adding aliases to the AX driver.

### Task 2: Linux ui instances on their own Xvfb, on the run's a11y bus

**Files:**
- Modify: `conformance/agterm_conformance/impls/base.py`, `conformance/agterm_conformance/impls/rust.py`
- Modify: `conformance/agterm_conformance/plugin.py` (run-wide a11y bus, layer-chosen launch mode, `ui` takes the
  instance)
- Modify: `conformance/tools/run-linux.sh` (setup checks)
- Modify: `conformance/selftests/test_impls.py`, `test_isolation.py`, `test_run_linux.py`, `test_plugin_options.py`
- Create: `conformance/selftests/fake_xvfb.py`, `fake_a11y_bus.py` (if the fakes need them)

- [x] write self-tests first: an instance launched from a `tests/ui` module (module fixture, `launch()`, `relaunch`) is an
      X11 instance with its own Xvfb display and session bus and the run's a11y address; one launched elsewhere is a
      weston instance; the env carries exactly the harness-set `DISPLAY`/`AT_SPI_BUS_ADDRESS`/`GDK_BACKEND`/
      `GSETTINGS_BACKEND`, with every inherited `NGTERM_*`/`AGTERM_*`/`ZMX_*` gone
- [x] write self-tests for the refusals: an inherited `DISPLAY` or a11y address, a ui run in a non-headless display
      mode, an `Atspi.init()` before the run's bus is set; and for missing setup (Xvfb, launcher in its libexec paths,
      registryd, typelib) exiting 2 naming the step
- [x] write self-tests for the reap: Xvfb, the instance bus, and at session end the launcher, its dbus-daemon and
      registryd; nothing left in the instance root
- [x] implement the run-wide bus, the launch mode, the env guard change and the fixture wiring
- [x] check on the dev box (headless, lavapipe) that an instance comes up on its Xvfb and its AccessKit tree appears on
      the run's a11y bus; settle whether input focus needs a window manager and record the answer here
- [x] run the self-tests and a partial suite run over one protocol id (weston path unchanged)

**Decision:** AccessKit checks `org.a11y.Status.IsEnabled` on each app's session bus before reading
`AT_SPI_BUS_ADDRESS`. A small owned Gio discovery/status helper therefore serves `org.a11y.Bus.GetAddress`
and the status properties on each instance bus. It points to the run's real accessibility bus; it exposes
no app controls. It is retained through relaunch and reaped with the instance. Codex independently agreed.

**Focus:** no window manager is needed. On bare Xvfb, explicit X input focus plus XTest keyboard input
successfully toggled the sidebar with protocol read-back. Future AT-SPI driver input must focus its own window.

**Validation:** 926 harness self-tests passed (19 skips, including the opt-in real probe); subsequent focused
lifecycle checks include partial startup failure cleanup. The opt-in `test_linux_ui_integration.py` passed
against the existing Linux debug build: two separate displays and session buses, both app PIDs and tree
descendants on the shared AT-SPI desktop, a relaunched PID, real input, and complete helper/root cleanup.
`tests/protocol/test_version.py::test_version_reports_the_app_version` passed on the unchanged Weston path.
The real probe uses a system-Python venv with PyGObject and python-xlib; ordinary self-tests import neither.

### Task 3: Failure traces

**Files:**
- Create: `conformance/agterm_conformance/drivers/trace.py`
- Modify: `conformance/agterm_conformance/client.py` (socket transcript), `plugin.py` (`--trace-mode`,
  `runtest_makereport`, `CliProcess` transcript), `drivers/base.py` (action log, tree dump, screenshot)
- Create: `conformance/selftests/test_trace.py`
- Modify: `.github/workflows/linux.yml`, `cross-platform.yml`, `reference-macos.yml` (upload `traces/`)
- Modify: `conformance/pyproject.toml` (Pillow)

- [x] write self-tests first: a failing test writes the directory with each file; a passing test writes nothing under
      `on-failure`; `always` writes for a pass; `off` writes nothing; a protocol-layer failure gets socket, cli, panes
      and log but no screenshot or tree; a trace step that raises goes to `trace-errors.txt` and the test's own failure
      is what pytest reports
- [x] implement the per-test logs, the transcripts, the collector and the screenshot
- [x] add the option and the hook; upload `traces/` in the three workflows
- [x] run the self-tests and one deliberately failing local test (then drop it): the trace is complete

**Decision:** the artifact option is `--trace-mode=off|on-failure|always`, default `on-failure`.
Pytest already reserves `--trace` for its interactive debugger. Codex agreed to preserve that flag and use
an independent option/destination. `--trace-dir` overrides the default directory beside the nodeid or JUnit
report (otherwise `conformance/reports/traces`). Existing CI artifact directories already include this location.

**Validation:** 959 harness self-tests passed (19 platform/opt-in skips); final driver/client/trace checks
passed 160 tests after adding explicit per-test memory release. A temporary deliberately failing real Linux
probe produced a 1920×1080 private-Xvfb screenshot, real AT-SPI app tree, action/socket/CLI transcripts,
app log and two session text files while preserving its original assertion. The probe and its processes were
removed afterward; both VMs stayed stopped. Native Windows/macOS screenshot calls are covered by the shared
API; this task did not run those platforms. Successful reference reruns discard failed-attempt evidence.

### Task 4: GPUI publishes screen bounds to AccessKit on Linux

**Files:**
- Create: `build/patches/gpui-pre-linux/0005-a11y-window-bounds.patch`, through `scripts/vendor-gpui.sh`
- Modify: `conformance/selftests/test_linux_ui_integration.py` (real AT-SPI regression using the existing private-bus lifecycle)

- [x] write the test first: an X11 window at a known origin on a private Xvfb; the bounds AccessKit receives (read back
      through AT-SPI `get_extents(SCREEN)` on a private a11y bus, or the adapter's root bounds) include that origin
- [x] write the patch: call `a11y_update_window_bounds` on configure in both Linux backends (X11 and Wayland, where it
      gives window-relative bounds at least)
- [x] regenerate `vendor/` through the vendor script; its `--check` holds
- [x] run clippy and tests on `ngterm-gpui`

**Decision:** use the existing Python integration harness to observe real AT-SPI extents instead of
reimplementing its private accessibility bus/status service in the Rust window harness. Codex agreed.
The test failed before the patch with `(-1, -1, -1, -1)` versus the real X geometry `(512, 240, 900, 600)`.
The probe shares one module-owned bus because libatspi retains process-global connection state.

**Bounds:** initialize when the adapter is installed and update on configure. X11 translates the real
client origin to root coordinates, so combined resize/move and reparenting cannot leave a stale origin;
node coordinates include the whole client surface, so client-drawn shadow insets are not added twice.
Wayland reports scaled surface-relative bounds at zero origin and refreshes on scale changes; it does
not claim a global screen position. External window-manager frame decorations are outside the reported
client surface. The patch header records these coordinate assumptions.

**Validation:** vendor regeneration/check passed; `cargo clippy -p ngterm-gpui --all-targets -- -D warnings`
passed; all 5 native `x11_window` tests passed. All 3 real private-Xvfb/AT-SPI cases passed, covering initial,
moved, resized, combined move/resize, and reparented geometry at scale 1 and 2, plus descendant screen
coordinates and existing lifecycle/input checks. Host-free lifecycle checks passed 29 tests (3 real probes
skipped when not opted in); the Weston protocol-version smoke test passed against the patched build.
No VMs were used. The Wayland fallback was compiled and smoke-tested, not read back through AT-SPI.

### Task 5: The AT-SPI driver on one ui file

**Files:**
- Create: `conformance/agterm_conformance/drivers/atspi.py`
- Modify: `conformance/agterm_conformance/drivers/base.py` (`IMPLEMENTED`)
- Modify: `conformance/pyproject.toml` (PyGObject, python-xlib, Linux-only `linux-ui` extra)
- Modify: `.github/workflows/linux.yml` (conformance job packages: Xvfb, at-spi2-core, gobject-introspection, the
  typelib, PyGObject's build deps)

- [x] write host-free self-tests first: role mapping to `ROLES`, key names and text to keysyms with shift, chord
      modifiers, the lazy import (the self-tests run without `gi`)
- [x] implement `atspi.py`: the app by pid, ui ids from `get_accessible_id`, the portable attributes, the press through
      the Action interface, text through Text/EditableText and Value, focus, the in-window app menu (`_menu_command`
      through the title-bar menu button's popup), `_menu_item`, Settings tabs (`_tab`), keys, text and pointer through
      XTest on the instance's display, bounds from `get_extents(SCREEN)`
- [x] run `tests/ui/test_welcome.py` (the smallest file) on Linux green, fixing what the driver gets wrong
- [x] run the self-tests and that file's partial suite run
- [x] ⚠️ resolve the blocking final review finding: general pointer actions can hit an overlapping
      window instead of the saved target (fixed; the backlog item is closed)

**Implementation notes:** native bindings remain lazy, and the `linux-ui` extra follows the existing
`ax` extra so host-free installations do not need GI build dependencies. Linux conformance CI installs
that extra and the native AT-SPI/introspection packages. The driver accepts only a matching instance's
owned Xvfb context. XTest focuses that app's mapped window explicitly; portable `cmd` becomes Control.
Text uses US shifted symbols and, when needed, empty keycodes on the disposable private X server for
Unicode; exhaustion fails explicitly instead of replacing an existing mapping.

**App fix:** the welcome checkboxes had the correct label/role but no accessibility toggled state.
A Rust regression reproduced `None` instead of `Some(True)`; the modal now publishes both checked and
unchecked states. A disappearing child during AT-SPI traversal is retryable, not an empty-tree result.

**Validation:** the welcome UI file passed on Linux without skips. 987 host-free self-tests passed
(24 platform/opt-in skips); the retained real AT-SPI input probe passed pointer clicks, modifier chords,
and mixed ASCII/Latin/CJK typing with protocol read-back. All 10 modal Rust tests, native build and strict
GPUI all-target clippy passed. Two existing Task 3 self-test mismatches were corrected: the socket isolation
allowlist follows the renamed `_open_stream` helper, and the launch-options unit test stubs unrelated trace
registration. Other UI files and their accessibility gaps remain Task 6. No VMs were needed.

**Review status:** the review rounds fixed keyboard focus preservation, duplicate-title submenu lookup,
deferred-popup discovery and menu/window identity. The last round's blocker, general pointer actions
hitting an overlapping window, is fixed: every pointer action (move, hover, clicks, scroll, both drags)
now raises and focuses the resolved target's top-level X11 window before input, through the base
`_pointer_target` hook. In-window dialog overlays resolve to their frame. A private-Xvfb regression with
overlapping windows failed before the fix and passes after it.

**Notes for Task 6:** two observations remain to investigate when execution resumes. After
`Ctrl+Shift+S` toggles the sidebar in window 1, typing a command beginning with `s` in window 2 dropped
that first `s`; the isolated fresh-app multiwindow input probe passes, so shortcut-release bookkeeping
is suspected but not established. Custom app command menu rows also lack accessible names, blocking
`View → Find`; the named `File → Open Window` submenu probe passes. Neither observation is claimed fixed.

### Task 6: The Linux ui layer green and in the runner

**Files:**
- Modify: `crates/ngterm-gpui/src/*` (accessibility gaps, each with its Rust test)
- Modify: `conformance/docs/rust-build.md` (any capability a platform divergence needs)
- Modify: `conformance/tools/run-linux.sh` (ui into the default layers)
- Modify: `conformance/rust-passing.txt`

- [x] run each remaining ui file on Linux and list each red with its cause
- [x] per file: for each red caused by the app, write the Rust test for the element's role/name/value first, then fix
      it in `ngterm-gpui`; for each legitimate platform divergence, record a capability with its reason and branch the
      test as the reference discipline says (the branch green on reference-macos)
- [x] add `tests/ui` to `run-linux.sh`'s default layers and its self-test
- [x] run clippy and tests on the touched crates, the self-tests and the ui layer; add the newly green ids with
      `check_ratchet.py --add`

**Reds found (first run: 12 pass, 32 fail) and their causes:**

- *Labels without text* (ask title/message/buttons, palette rows and subtitle, the editor hint, the page strip's
  identity and title, the dashboard cover, settings hints): AccessKit names a Label after its value, and a Button,
  MenuItem or ListBoxOption only after its own label or its Label descendants; the app set neither. Fixed with
  `aria_value`/`aria_label`, Rust tests on the elements.
- *Menu items without names*, the Task 5 note: gpui-kit's element rows carry no label. Each row's title is now its
  Label descendant (`app_menu::item_title`), which AccessKit names the item after; View's long submenu scrolls
  (`scrollable`) and the driver wheels an item outside its menu into view before the press.
- *Shortcut release, the Task 5 note*: a keymap press whose release reached a window with no focused element (GPUI
  routes such a key-up to the root view only) left the key consumed, so the next press of it anywhere read as its
  autorepeat and was dropped. A fresh native press (the platform's repeat flag clear) now drops the stale entry
  (`keymap_dispatch::Router::press`, Rust test).
- *Disabled controls and menu buttons*: GPUI had no way to publish either (new GPUI patch 0005, `aria_disabled`,
  `aria_has_popup`) and AccessKit's AT-SPI mapping gave a disabled button Enabled and a menu button a push button's
  role (vendored `accesskit_atspi_common` with patch 0001, `scripts/vendor-accesskit-atspi.sh`, CI check). The chrome
  button publishes both and opens its menu on an accessibility press.
- *Roles upstream gives*: `command-palette`/`pick-palette` on the field too (upstream's field inherits the panel's id),
  `notify-badge` and `agent-status` as text valued with the count and status, settings switches as checkboxes,
  sliders as sliders, pickers as combo boxes (AT-SPI's popup), ask choices and Other as checkboxes/radios.
- *Driver*: a static label's text is its name; a read-only entry is text; text is replaced through the keyboard
  (AccessKit publishes no EditableText); a menu item outside its scrolled menu is wheeled in.
- *Platform divergences*: `ui-string-values` (rust-build.md): AT-SPI carries no string value on a button, image or
  row, so those predicates branch and `test_tinted_icon_names_its_tint` is unsupported on Linux (an open
  accessibility gap, recorded there). The Settings opener uses the app menu on Linux (no Settings chord there), and the
  Ctrl+Tab switcher test skips on AT-SPI (XTest releases Control after the chord, which ends the walk).
- *Selectable text* (review fix): the Settings paths and diagnostics, read-only gpui-kit fields, publish as labels
  (upstream's static text) through the field's role override, since no read-only flag reaches AccessKit and AT-SPI
  read them as editable fields; `test_general_tab` and `test_key_mapping_tab_reports_a_keymap_error_after_reload` are
  green and in the ratchet (`selectable` Rust test).

Was red on Linux at the end of this task: `test_html_overlay.py::test_page_strip_hide_hides_its_own_pane` (the Hide
button stayed in the tree after the pane's page hid). Fixed in Task 7 (GPUI patch 0006: an invisible element's
descendants publish no node) and in the ratchet.

### Task 7: The UIA driver and the Windows ui layer

**Files:**
- Create: `conformance/agterm_conformance/drivers/uia.py`
- Modify: `conformance/agterm_conformance/drivers/base.py` (`IMPLEMENTED`)
- Modify: `conformance/pyproject.toml` (comtypes, Windows only)
- Modify: `conformance/tools/run-windows.sh` (ui into the default layers), `.github/workflows/cross-platform.yml`
- Modify: `crates/ngterm-gpui/src/*` (accessibility gaps, each with its Rust test)
- Modify: `conformance/rust-passing-windows.txt`

- [x] write host-free self-tests first: UIA control types to `ROLES`, key names to virtual keys, the `SendInput`
      structures for a chord, a text and a drag, the lazy import
- [x] implement `uia.py`: the tree by process id, `AutomationId` as the ui id, the portable attributes, Invoke/Toggle/
      Value/SelectionItem patterns, the in-window menu and tabs, `SendInput` for keys, text (unicode) and pointer,
      bounds from `BoundingRectangle`
- [x] run the ui layer on the Windows CI runner (`ci/win-<task>` branch, traces from Task 3 for the reds); debug in the
      VM only; fix each app-caused red with its Rust test first, record each divergence as a capability
- [x] add `tests/ui` to `run-windows.sh`'s default layers; the Windows CI job green; add the newly green ids to
      `rust-passing-windows.txt`

**Reds found on the Windows runner (first run: 27 pass, 15 fail) and their causes:**

- *Menu items with no press*: gpui-component's menu rows publish their highlight as `aria_selected`, and AccessKit's
  UIA mapping withholds Invoke from every node with a selected state, while a menu item gets no SelectionItem either.
  Fixed in AccessKit: vendored `accesskit_windows` 0.34.0 with patch 0001 (a clickable node whose selected state UIA
  cannot express keeps Invoke), `scripts/vendor-accesskit-windows.sh`, its `--check` in `linux.yml`.
- *Menu buttons with no press*: UIA gives a button with a popup only expand/collapse, which needs an expanded state
  the trigger did not publish. The chrome button's menu trigger now publishes `aria_expanded` and takes Expand and
  Collapse as its press (`chrome::button::menu_button`, Rust test).
- *Settings opener and the Ctrl+Tab switcher*: as on Linux, Windows has no Settings chord (the app menu's item opens
  it) and `SendInput` releases Control after the chord; both branches take `uia` beside `atspi`.
- *The page strip's Hide button lingering* (the Linux red of Task 6 too): GPUI published accessibility nodes under an
  invisible element (`.invisible()`, a hidden page laid out where it would show). GPUI patch 0006 leaves an invisible
  div's descendants out of the tree; `test_page_strip_hide_hides_its_own_pane` is green on both and in both ratchets.
- *Not claimed*: `ui-string-values` stays unclaimed on Windows: UIA reads a value where the app publishes one, but
  `split-toggle` and the sidebar footer's toggles publish none
  (docs/backlog/the-split-and-sidebar-toggles-publish-no-state-value.md).
- ⚠️ The win11 VM has no interactive logon (no autologon, password unknown) and GPUI opens no window in its ssh
  session, so every ui run went to the runner; the VM checked only the UIA bindings.
- ⚠️ The Windows job's ui layer is green (41 pass, 1 skip, 2 unsupported, no red), but the job stays red from protocol
  ids that fail intermittently on `main` as well (companions, config browser), and the arm64 crates job from a clippy
  lint on `main` (docs/backlog/windows-ci-is-red-on-main-from-three-protocol-tests-and-a-clippy-lint.md).
- ➕ Task 6 left `the_view_menus_overlays_row_names_what_it_will_do_and_toggles` red (View's submenu now scrolls past
  450 points, so its overlays row lay outside it): the test wheels View to the row first.

### Task 8: Pointer and typing tests

**Files:**
- Create: `conformance/tests/ui/test_pointer.py`, `conformance/tests/ui/test_pane_input.py`
- Modify: `conformance/catalog.yaml` (behavior items a test covers, if any)
- Modify: `crates/ngterm-gpui/src/*` (if a target lacks its id, role or bounds)

- [x] read upstream (`.upstream/agterm`: sidebar row drag, split dividers, sidebar scrolling, row and pane context
      menus, typing into a pane, their UI tests) and list the interactions upstream has; for each, name its target
      (an upstream id, or an identified element plus geometry the control protocol reports) and its read-back (the
      tree, an element, `session.text`, `surface.cursor`); one with neither is left out with its reason here
- [x] write the ui tests first for the listed interactions: drag-reorder of sessions and workspaces, the divider drag,
      the scroll, each context menu and one of its items, `type_text` into the selected pane read back with
      `wait_text` and `wait_cursor`
- [x] run them on `reference-macos.yml` until green there (red on the reference means the test is wrong)
- [x] run them on Linux and on the Windows runner; fix what the app lacks with its Rust test first
- [x] add the newly green ids to both ratchets

**Interactions upstream has (v0.39.1) and how each is read:**

| interaction | upstream | target | read-back |
|---|---|---|---|
| session row drag onto a row | `ReorderUITests.testReorderSessionUp` | `session-row` by its text | tree order |
| workspace row drag above a row | `ReorderUITests.testReorderWorkspace` | `workspace-row` by its text, top edge | tree order |
| divider drag | `SplitRatioUITests.testDividerDragPersistsTheRatio` | the window (`ui.window()`) at the ratio's x, sidebar hidden | `splitRatio` |
| divider double-click | `ControlOverlaySplitUITests.testDoubleClickOnDividerRestoresEvenSplit` | as the drag, at 0.7 | `splitRatio` back to 0.5 |
| sidebar wheel | none (an `NSOutlineView` scrolls natively) | a `workspace-row` mid-window | the row's frame (`ui.frame`) moves up |
| session row context menu, Flag | `FlaggedViewUITests`, `ReorderUITests` | `session-row`, right click | `flagged` in the tree |
| workspace row context menu, New Session | `SidebarUITests` | `workspace-row`, right click | the workspace's sessions |
| typing into the selected pane | `typeText` throughout (`SessionNavUITests`, ...) | the focused pane | `wait_text` and `wait_cursor` |

- *Left out*: a pane context menu, since upstream has none (`GhosttySurfaceView+Input.swift`: a right click is the
  terminal's own right-click-action, by default paste, which reads the desktop clipboard the suite may not seed).
- The divider has no upstream id and no queryable element on every OS, so, as upstream's own test does, its point is
  the window's at the ratio (the panes span the window's width with the sidebar hidden), tried at small offsets until
  the ratio moves. The driver gained `frame(target)` (an element's live screen rectangle), `window()` and drag pacing
  (`press_for`, `duration`, `hold_for`: an outline view starts a row drag only from a held press at a human pace).
- The typed pane runs `cat` (`session.new --command`), not a login shell: a prompt of unknown length wrapped the line on
  the reference, and the protocol's cursor has no row.
- No app change was needed: every target already had its id, role and bounds.

**Validation:** reference-macos (scoped to the two files, branch `ci/ref-task8`)
[run 41573927676](https://github.com/nickalie/ngterm/actions/runs/41573927676): 8 passed, no reruns (the first run's
red was the prompt-wrapped cursor, fixed in the test). Linux, private Xvfb and a11y bus, debug stage: the whole ui layer
50 pass, 2 skip, 0 fail. Windows runner (`ci/win-task8`, suite scoped by `-k`)
[run 41573927395](https://github.com/nickalie/ngterm/actions/runs/41573927395): 8 passed. The 8 ids are in both
ratchets. Self-tests pass except the two known host-dependent `test_impls` noop-URI cases; the coverage gate passes.

### Task 9: Instrumented stage and the coverage flush

**Files:**
- Modify: `crates/ngterm-gpui/src/main.rs` (`exit_now`)
- Modify: `Cargo.toml` (`check-cfg` for `cfg(coverage)`)
- Modify: `conformance/tools/run-linux.sh`, `run-windows.sh` (`--coverage`)
- Modify: `conformance/agterm_conformance/impls/base.py`, `plugin.py` (`LLVM_PROFILE_FILE`, `lost-profiles.txt`)
- Modify: `conformance/selftests/test_impls.py`, `test_run_linux.py`

- [x] list every path in `ngterm-gpui`, `ngterm-session`, `ngterm-ctl` and the helper binaries that ends the process
      without running the exit handlers (`_exit`, `TerminateProcess`, signal deaths); check on Windows whether the
      session server ends by its idle grace or is killed by the reap (`IDLE_GRACE` vs `REAP_TREE_GRACE`)
      - `ngterm`: `exit_now` (`_exit`, `TerminateProcess` of itself) → flushed now; the early `ExitCode` returns of
        `main` run the handlers. Lost: a second SIGTERM's default action (`signals.rs`; the harness sends one), a
        crash, the harness's SIGABRT/SIGKILL of a stalled app (recorded).
      - `ngterm-headless`: the Unix relaunch by `exec` (`relaunch.rs`) → flushed before the `exec`; else `ExitCode`.
      - `ngterm-session`: the server returns its `ExitCode` (handlers run). Lost: a server its client kills as not
        detached (`spawn/mod.rs`, an error path); `ngterm-session attach` stopped by the harness (`PtyProcess.stop`,
        recorded). The sessions' programs it ends (`job/windows.rs`) are pane programs, not the build's.
      - `ngtermctl`: its `process::exit` paths (`session_wait.rs`, `overlay_job.rs`) run the handlers; it handles no
        SIGTERM, so a stream the harness closes (`CliProcess.close`) dies of it (recorded: 3 on the dev box's partial
        run). The app's presentation bridge and page channel children (`ngterm-app/src/remote.rs`,
        `page_channel.rs`) are ended by SIGTERM / `TerminateProcess`: an `ngtermctl` there loses its profile
        unrecorded (remote rows only). A pane's `ngtermctl` gets no `LLVM_PROFILE_FILE` (the panes' env is cleared).
      - Windows: the server ends by its idle grace. The stop's leftover check waits `LEFTOVER_GRACE` +
        `server_idle` (5 + 3 s) for it to exit before `reap_process_tree` (`REAP_TREE_GRACE`), so the reap ends it
        only after a leftover failure; it breaks away from the Job, whose close never ends it. Every process the reap
        ends by its pid is recorded in `lost-profiles.txt`.
- [x] write self-tests first: `--coverage` exports a pooled `LLVM_PROFILE_FILE` for every process the adapter and
      `CliProcess` start; a process killed or SIGTERMed without its quit is listed in `lost-profiles.txt`; a stage
      built without coverage is refused under `--coverage`
- [x] implement the flush in `exit_now` (and any path the list found) and the `check-cfg` entry; the runners'
      `--coverage` build and stage through `cargo llvm-cov show-env`
- [x] check on the dev box that a partial instrumented run leaves profiles for the app, `ngtermctl` and the session
      server, and that their count stays bounded
- [x] run clippy and tests on the touched crates and the self-tests; the Windows side on the Windows runner
      (`ci/win-task9`, run 41579360351: 3 pooled profiles for the app, `ngtermctl` and the session server, 3
      `ngtermctl` streams lost, no reap kill; 72 pass, `test_overlay_page_blocks_until_submitted` red under
      instrumentation by a 10 s socket timeout; the dev box's partial run: 11 files over 3 binaries, 4 lost)

### Task 10: Coverage report and the "ui ids exercised" line

**Files:**
- Modify: `conformance/tools/run-linux.sh`, `run-windows.sh` (merge, report, `report.md` sections)
- Modify: `conformance/agterm_conformance/plugin.py` (`covers` targets in `nodeids.jsonl`)
- Modify: `conformance/agterm_conformance/coverage.py` (exercised ids)
- Modify: `conformance/selftests/test_coverage.py`, `test_run_linux.py`, `test_ratchet.py` (the report lines' new
      field leaves the ratchet unchanged)

- [x] write self-tests first: report lines carry `covers`; the exercised count counts passed tests only; `report.md`
      carries both sections; no profiles or a binary mismatch exits 2 with the reason
- [x] implement the `cargo llvm-cov report` step (lcov, html, per-crate summary with the exclusions) and the line
- [x] run a partial instrumented run on the dev box and check the summary's crates and numbers are plausible
      (`--stage target/conformance-stage-cov`, 84 tests: 13 crates, `-sys` gone, total 45.9% lines, ngterm-headless 0%
      as never launched; 14/127 ui ids exercised; llvm-cov warns of 4 mismatched functions with the build's own
      stage, so the warning is noted in the summary and the mismatch refusal compares the staged binaries' bytes)
- [x] run the self-tests

### Task 11: Coverage floors and the CI coverage jobs

**Files:**
- Create: `conformance/tools/check_coverage.py`
- Create: `conformance/selftests/test_check_coverage.py`
- Create: `conformance/coverage-floor-linux.txt`, `conformance/coverage-floor-windows.txt`
- Modify: `.github/workflows/linux.yml`, `.github/workflows/cross-platform.yml`

- [x] write self-tests first: a drop within 0.5 pt passes, a larger one fails naming the crate and both numbers,
      `--add` raises and never lowers, a new crate is added, a removed one dropped, a malformed floor file exits 2
- [x] implement `check_coverage.py`
- [x] add a coverage job to `linux.yml` and to `cross-platform.yml` (Windows): `llvm-tools` and cargo-llvm-cov
      installed, the instrumented stage, all layers, `check_coverage.py` (no ratchet judgment), upload `coverage/` and
      `traces/`, the summary in the job summary; on pushes to `main` and on PRs
- [x] seed both floor files from the first green run of each job
- [x] both jobs green on CI (`ci/cov-task11`: seeding runs linux 41586363286, cross-platform 41586363312; with the
      seeded floors linux 41591055949 holds 13 crates, cross-platform 41591055924 holds 11)

### Task 12: Verify acceptance criteria

- [x] the ui layer runs (not skips) in the default layers on Linux (each shell entry) and Windows CI, its ids in both
      ratchets (`ci/verify-task12`, d76bcbea = 86485f92 plus a trigger-only commit: linux 41596232222 bash/zsh/fish
      green, layers protocol cli behaviors ui, ui 49/50/50 pass with 3/2/2 driver skips, ratchet holds 932; Windows
      job in cross-platform 41596231921: ui 49 pass/3 skip/0 fail, its only reds the three protocol ids of
      docs/backlog/windows-ci-is-red-on-main-from-three-protocol-tests-and-a-clippy-lint.md; 50 Linux and 49 Windows
      ui ids in the ratchets)
- [x] the pointer and typing tests are green on reference-macos, Linux and Windows (8/8 in each Linux shell entry and
      the Windows job above; reference-macos 41596241267 ui layer 52/52 on both attempts)
- [x] a failing test anywhere leaves a complete trace, uploaded by CI (ui failures: reference-macos 41573455447 and
      Windows 41560456592 artifacts hold screenshot.png, a11y-tree.txt, actions, socket, cli, panes, app.log; protocol
      failures: the Windows 41596231921 and coverage-windows artifacts hold socket, cli, panes and app.log; the Linux
      upload path is the same step, its runs had no failure; a Linux Xvfb failure trace was checked locally in Task 3)
- [x] both coverage jobs report per-crate numbers and hold their floors; `report.md` shows code and catalog coverage
      (coverage-windows 41596231921 held 11 crates; the Linux coverage entry broke ngterm-gpui at 63.80% against a
      64.37% floor on the same build that seeded it: ➕ floors reseeded from the per-crate minimum of three same-build
      runs per OS and `check_coverage.py --add` now takes a batch of at least two summaries, see Decisions; both
      reports carry `### code coverage` and `### catalog coverage`, 121/127 ui ids exercised)
- [x] gates on the changed part only: clippy on the touched crates and dependents, their tests, the self-tests, a
      partial suite run over the ui ids; the whole workspace and the full suite are CI's (clippy -D warnings on
      ngterm-gpui/app/headless/wpe clean, fmt clean, selftests 1133 pass with the 2 known noop-URI host failures;
      conformance-gate on `ci/verify-task12` 41596232288 broke on test_ui_setup_failure_withholds_runner_report (it
      needed at-spi2-core on the host since ui became a default layer) ➕ fixed, gate green on `ci/gate-task12`
      41596782313; the ui ids ran in full on CI above; reference-macos protocol reds are on `main` too, run 41603323048,
      docs/backlog/reference-macos-zmx-restart-and-hold-tests-time-out.md)

### Task 13: [Final] Update documentation

- [ ] `conformance/README.md`: the ui layer on the Rust build, the drivers, the pointer API, `--trace-mode`, `--coverage`,
      the floors; the reruns rule per side
- [ ] `conformance/docs/rust-build.md`: the Linux ui launch contract (own Xvfb, the run's a11y bus, the env), the
      coverage build and flush
- [ ] `README.md` > Test and Native dependencies: Xvfb, at-spi2-core, the `Atspi` typelib, the Python packages,
      cargo-llvm-cov and `llvm-tools`; the Windows setup if it changed
- [ ] the bounds patch's header says why it is there, as the other `build/patches/gpui-pre-linux` patches do
- [ ] `CLAUDE.md` > Porting a command group: a UI port lands with its ui test (the rule, no details)
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

**Manual verification:**
- look through one CI trace artifact and one coverage HTML report to confirm they read well for a person debugging a
  red run.

**External:**
- the Rust macOS frontend (open work) uses the `ax` driver; its ui run joins when that frontend lands.
- the GPUI bounds patch is a candidate for upstream GPUI.
