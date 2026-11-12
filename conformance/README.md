# agterm conformance suite

Black-box tests that define feature parity with upstream agterm.
The same tests run against the released macOS app and the Rust build, over the control socket,
`agtermctl` and the accessibility tree.

## Install

```sh
python -m venv .venv && .venv/bin/pip install -e conformance          # Linux, CI gate
python -m venv .venv && .venv/bin/pip install -e "conformance[linux-ui]"  # Linux ui layer: PyGObject, python-xlib
python -m venv .venv && .venv/bin/pip install -e "conformance[ax]"    # macOS: adds the ax ui driver's pyobjc
python -m venv .venv && .venv/Scripts/python.exe -m pip install -e conformance   # Windows (Git Bash)
```

The Linux ui layer also needs Xvfb, at-spi2-core (`at-spi-bus-launcher`, `at-spi2-registryd`) and the distribution's
`Atspi-2.0` typelib (docs/development.md > Prerequisites); pip builds PyGObject against gobject-introspection and cairo.

## Running

`--impl` picks the adapter; `--bin` is the app to launch; `--ctl` overrides the CLI found
beside it (`agtermctl`, the Rust build's `ngtermctl`: the adapter's `ctl_name`).

```sh
# upstream macOS release (a bundle or its executable); fetch_reference.py downloads and caches it by tag
bundle=$(.venv/bin/python conformance/tools/fetch_reference.py --tag v0.41.1 | sed -n 's/^bundle=//p')
.venv/bin/python -m pytest conformance/tests --impl swift-macos --bin "$bundle"

# the Rust build; without --bin every test skips and parity reads 0%
.venv/bin/python -m pytest conformance/tests --impl rust --bin <rust-binary>

# the Rust build of this checkout: build, stage, protocol, cli, behaviors and ui, then the ratchet;
# report in conformance/reports/rust-linux/; --coverage: the instrumented build and its code coverage (Code coverage)
conformance/tools/run-linux.sh [--stage DIR] [--coverage] [-- PYTEST_ARGS...]

# the same on Windows, from Git Bash (Windows runner and ratchet below); report in conformance/reports/rust-windows/
conformance/tools/run-windows.sh [--stage DIR] [--coverage] [-- PYTEST_ARGS...]

# whether the staged Rust build's WPE WebKit loads a page under the same headless isolation (a diagnosis of the web
# process's bubblewrap sandbox; the report carries the frame mode from the app's log)
.venv/bin/python conformance/tools/webkit_probe.py [--bin PATH]

# whether the staged Rust build's terminal surface is created and draws a frame under the same isolation (`version`
# alone answers while the terminal cannot render); defaults to run-linux.sh's stage
.venv/bin/python conformance/tools/ghostty_probe.py [--bin PATH]

# whether the staged Rust build's title bar, sidebar and terminal draw one color, the terminal theme's background, and
# a sidebar tint washes the sidebar alone: screenshots of the maximized window from the same isolation, its weston
# started with --debug for weston-screenshooter (a Rust-build check outside the catalog); defaults to the same stage
.venv/bin/python conformance/tools/chrome_probe.py [--bin PATH] [--keep DIR]
```

`--macos-launch-mode plain` replaces the default `uitest` launch (XCUITest sentinel with live sessions on).
The Rust build renders on lavapipe only (see Rust runner and ratchet): its probes and the harness take the ICD manifest
from `$CONFORMANCE_LAVAPIPE_ICD`, the probes else from `scripts/fetch-lavapipe.sh --print-icd`.
The Rust build's side of the contract (the `version` extension, the launch environment, the capability
vocabulary) is in [docs/rust-build.md](docs/rust-build.md).

## Layers

- `tests/protocol`: one JSON line per request over the socket, through `ctl`.
- `tests/cli`: `agtermctl` output, exit codes and socket resolution, through `cli`.
- `tests/behaviors`: relaunch, keymap and hooks, across instances started by `launch`.
- `tests/ui`: accessibility ids from `ui-ids.yaml`, through `ui` (UI layer below).
- `selftests`: the harness itself, host-free; they run without `--impl`.

## Markers

- `since("0.41.1")`: skipped as `not-yet-at-level` when the build's parity level is lower. A `pytest.param`, test,
  class or module may each carry one; the latest of an item's levels gates it.
- `requires("capability", reason=...)`: skipped as `unsupported` when the build lacks it.
  `requires("a", "b", any=True)` is skipped only when the build lacks both (Live sessions in the login shell, from
  upstream's `live-sessions` or the Rust build's `live-any-shell`); `--must-pass` on either counts it.
  A test that branches instead asks `adapter.has("capability")`.
  Both refuse names outside the vocabulary in `impls/base.py`.
- `covers("item", ..., streams=(...))`: the catalog keys a test covers; a stream-mode command counts only when
  `streams` names it too, from a test that drives the stream after the ok.
- `upstream_bug("name", reason=...)`: the test asserts the correct behavior where the reference has a named bug (see
  Reference discipline). On `swift-macos` the test is an expected failure, not strict (`xfailed` when the bug shows,
  `xpassed` when it does not). Only assertions in the test body are excused: other exceptions and setup or teardown
  failures remain failures. The Rust build gets no allowance and must pass. The reason cites upstream's code or
  backlog, or the evidence under Reference observations when the cause is unknown.

The run summary prints pass (including xpassed), fail, both skip kinds and reruns, plus xfailed when nonzero, and
parity: passed over passed, failed, xfailed and not-yet-at-level. Expected failures remain gaps in parity even when
the reference gate excuses them; an unsupported capability is a recorded divergence, not a gap.

- `--must-pass CAPABILITY` fails the run unless every collected test requiring it ran and passed, and at least one
  did (reference-macos uses it for `live-sessions`). A test skipped as unsupported for lacking only other
  capabilities it requires is excused: the reference's skip of a Rust-only extension's Live tests is that divergence.
  So is the reference's expected failure under an `upstream_bug` allowance, and no other xfail.
- `--nodeid-report PATH` writes one JSON line per run test with its outcome merged over setup, call and teardown,
  plus the run's collection errors and `--must-pass` problems; `tools/check_ratchet.py` reads it.
  A run pytest refuses (a usage error, a path or node id that matches nothing) or that breaks internally writes none.
- `--reruns N` (pytest-rerunfailures) is for the ui layer only, where accessibility reads race the app.
  Protocol, cli and behaviors tests wait on read-back and never rerun.

## UI layer

`ui` is a driver over the OS's accessibility tree of the instance under test, picked by host
(`drivers/base.py`, `PLATFORM_DRIVERS`); each imports its bindings lazily, so the host-free self-tests need none:

- `ax` (macOS): the AX tree, AXPress, CGEvent keys and pointer (`conformance[ax]`; the run needs the Accessibility
  grant, a screenshot the Screen Recording one);
- `atspi` (Linux): the AT-SPI tree of the app found by pid on the run's private a11y bus (libatspi through PyGObject),
  XTest input on the instance's own Xvfb (python-xlib);
- `uia` (Windows): UI Automation over `UIAutomationCore` (comtypes), `SendInput` for keys, text and the pointer.

An element's ui id is the app's accessibility author id (AccessKit's, read as AT-SPI's `AccessibleId` and UIA's
`AutomationId`). Tests find elements by id (`find`, `all`, `count`, `absent`; only `find` filters, by `role=`, `value=`,
`enabled=` and `name=`, a string or a compiled regex; the waits dump the inventory on timeout) and act on snapshots:

- `click` is the accessibility press; `press` posts key chords, `type_text` any printable text through the OS's text
  input; `text`, `set_text`, `focus`, `hover`, `menu`, `menu_item` and `tab` read, write and open what they name;
- pointer input has its own names: `move`, `mouse_click` (`button="left"|"right"`), `mouse_double_click`, `drag`,
  `drag_by`, `scroll`; all but `scroll` (which acts at the element's center) take an `offset` within the element,
  `drag` one per end (`src_offset`, `dst_offset`); `frame` reads an element's screen rectangle and
  `window` the app's first window, a target for points the protocol locates (a split divider);
- a pointer action reads the element's bounds at action time and finds a stale element again once, by id and name
  (role and name without an id), else fails with `ElementNotFound`; nothing retries after input went out.
- `wait.wait_cursor` waits on a pane's cursor, `wait.wait_text` on its text.

On Linux every instance a `tests/ui` module launches (its module fixtures, `launch()`, `relaunch`) runs on X11 on an
Xvfb of its own, all of them on one a11y bus the run starts (docs/rust-build.md > Launch contract); the other layers
stay on weston. A missing piece of that setup (Xvfb, `at-spi-bus-launcher`, `at-spi2-registryd`, the typelib) fails
the run with exit 2 naming it, never a skip.

Reruns: the Rust ui runs do not rerun; they wait on read-back like the other layers. reference-macos keeps
`--reruns 1` on its ui layer for the AX races of upstream's release.

### Failure traces

Each test keeps a trace in memory, written to `traces/<nodeid-slug>/` beside the nodeid report (else the JUnit report,
else `<rootdir>/reports/traces/`; `--trace-dir DIR` overrides) as `--trace-mode` says: `on-failure` (the default) keeps
only a failed test's, an `upstream_bug` expected failure's included, `always` every test's, `off` none, and a rerun that
passes drops its failed attempt's. A trace holds `actions.jsonl` (driver actions), `socket.jsonl` (control socket
requests and replies), `cli.jsonl` (each `ngtermctl` argv, exit code and output), the instance's `app.log` and `panes/`
(each pane's text), and, where a driver exists, `a11y-tree.txt` and `screenshot.png` (Pillow's `ImageGrab`, on Linux
from the instance's Xvfb). A second instance's files go under `instances/<slug>/`; whatever could not be collected is
named in `trace-errors.txt`. The CI jobs upload them with their reports.

## Reference discipline

The reference is upstream's macOS release: a test is accepted only when it is green on reference-macos in CI.
Red on the reference means the test is wrong, unless the reference is: upstream is a reference, not the source of
truth. Where upstream's behavior is a bug (lost output, a race, a doubled message, a pane that never starts), the
test asserts the correct behavior and the Rust build must show it. The reference run then gets an `upstream_bug`
allowance that names the bug and cites its cause. When the cause is unknown, its reason must say so and cite a
Reference observation below recording the failed assertion, run and evidence; a related bug is not evidence of the
same cause. A test is never weakened to match an upstream bug, and a test that merely races the reference is fixed
in the test, not allowed. An unexplained stall remains an observation, not an established upstream mechanism.

### Reference observations

These symptoms were reproduced against v0.41.1; their causes remain unknown. The allowances keep the assertions on
both builds and excuse only the reference. They are tracked apart from upstream's
`docs/backlog/fast-live-command-loses-output-in-host-handoff.md`, whose command ends before the pane attaches, and
`docs/backlog/session-host-attach-to-ended-command-misses-its-window-on-ci.md`, whose guard client is slow to exit.

- `live-restarted-shell-reads-no-input`: in reference-macos runs
  [42515259661](https://github.com/nickalie/ngterm/actions/runs/42515259661) and
  [42524091626](https://github.com/nickalie/ngterm/actions/runs/42524091626), the command-parent half of
  `test_a_parent_whose_shell_exits_restarts_it[live]` timed out after Ctrl+C ended `sleep 600`: the new shell echoed
  typed marker commands for 10 s without a prompt or marker output. The marker helper retries input while waiting,
  so this is not a single keystroke sent before the pane exists. That half now has its own test,
  `test_a_command_parent_whose_command_exits_restarts_as_a_shell`; the plain-shell respawn has no allowance.
- `live-restarted-command-output-missing`: in the eight-iteration diagnostic run
  [42520472215](https://github.com/nickalie/ngterm/actions/runs/42520472215), with allowances disabled,
  `test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt` missed its first-line marker from the whole text in
  one iteration: for 10 s after the restart the pane showed only the login banner and a shell prompt, the tree at the
  failure named no foreground program, and whether the command ran is unknown. A separate observation was a marker
  in scrollback above an empty visible screen. The test requires the marker on the visible screen before sending
  Ctrl+C.
- `live-wait-command-loses-output`: in that same diagnostic run, the `--wait` tests failed three of eight
  (`test_a_live_command_session_closes_when_its_command_exits_and_holds_with_wait`) and two of eight
  (`test_a_wait_command_keeps_scrollback_after_attached_exit`) iterations after their readiness checks read output.
  The former's traces retained only `Process exited...` in the whole text at exit; the latter releases its command
  only after the pane has read the marker, so its failure cannot be a command ending before attach. Output was also
  seen in scrollback above an empty visible screen. Both readiness checks require the visible marker; the latter
  still asserts the entire scrollback after exit.

### Recorded divergences

Where the Rust build legitimately differs from the reference on observable state or effects (a platform divergence,
such as Wayland giving a client no window position), the divergence becomes a capability with a reason, and the test
asserts each branch under `requires` or `adapter.has`. Everything else asserts upstream's behavior and upstream's
full error text directly.

The Rust build is held to upstream's contract, its capability branches included: it claims the capabilities listed
in [docs/rust-build.md](docs/rust-build.md) > Rust target profile, and where a test branches on a capability it
takes upstream's branch unless that list says otherwise. The Rust claim is v0.41.1
(docs/rust-build.md > `version` extension), so tests introduced through that release are eligible. The catalog's
reference release and the implementation claim are independent: updating the catalog alone never unlocks `since`
tests. Host capability skips remain recorded separately from passing coverage.
The Rust build's progress is held by a ratchet: `conformance/rust-passing.txt` lists the ids that went green, and
`run-linux.sh` fails when one of them goes red again (see rust-build.md > Ratchet).
A Rust-only extension (today `live-any-shell`, Live sessions in bash and PowerShell login shells; `agent-chat`, the
agent record behind the Android chat view; `pages-survive`, a page parked while no presenter shows it; and
`kitty-clipboard`, a program's Kitty clipboard protocol) is a
capability the reference does not claim: its tests skip as unsupported there and are validated by the Rust CI with
`--must-pass` on the capability (see rust-build.md > Rust-only extensions).
A platform adaptation no test reads differently stays a recorded divergence, with no capability of its own. v0.41's
are in rust-build.md > Release behavior at v0.41.1: Live panes also start a bash or PowerShell that a ghostty
`command` names (under `live-any-shell`), a clicked HTML page on Windows has its link hints made inert, and the
compact sidebar's metrics apply upstream's trims to this build's own row geometry.

## Rust runner and ratchet

On Windows the runner is `tools/run-windows.sh`, run from Git Bash: it builds the release app (with its WebView2 HTML
overlays), `ngtermctl` and `ngterm-session`, stages them with `scripts/stage.sh` (`bin\ngterm.exe`, `bin\ngtermctl.exe`
and `bin\ngterm-session.exe` beside `share\ghostty` and `share\terminfo`) into `target/conformance-stage`, runs the
same layers with the same options and holds its own list, `conformance/rust-passing-windows.txt`, with the same
`check_ratchet.py` (`--list`). The panes run Git for Windows' `sh.exe` (`CONFORMANCE_POSIX_SHELL`, else the one
beside `git`, else Git's default install directory) in place of `/bin/sh`, with `PATH` the system directories, then
Git's `usr\bin` (the POSIX tools a command line's `cmd.exe` finds there, as `/bin/sh -c` finds them in `/usr/bin`). A
foreground argv names its program by the path its parent started it with there (an MSYS shell gives the image's full
path), so the tests compare it by the program's name (`match.program_argv`).
Before the suite (not under `--coverage`) `tools/warm_webview2.py` opens one page in an instance of the stage: the
first web view after a boot pays the WebView2 runtime's cold start (seconds past a test's read-back wait on a runner),
and would charge it to whichever test opens the suite's first page.
The rest of this section describes the Linux runner.

`tools/run-linux.sh` builds the workspace (release), stages it with `scripts/stage.sh` into
`target/conformance-stage` (or runs an existing stage given with `--stage DIR`) and runs the protocol, cli,
behaviors and ui layers with `--impl rust`, each instance on its own headless weston and session bus, a ui instance on
its own Xvfb and the run's a11y bus instead of weston (Safety guards).
It runs pytest with `$PYTHON`, else `.venv/bin/python`, else `python3`, with this checkout's harness first on
`PYTHONPATH`.
Into `--out DIR` (default `reports/rust-linux/`) it writes `report.md`, `junit.xml`, `pytest.log`, `nodeids.jsonl`
(the `--nodeid-report`) and `traces/`; `report.md` ends with the ui ids a passing test exercised
(`python -m agterm_conformance.coverage nodeids.jsonl`).
It exits with the ratchet's status, not pytest's, since red tests are expected until parity: 0 holds, 1 broken (or an
instance root left behind), 2 no usable report (or, under `--coverage`, no code coverage report).
One suite run at a time per user: concurrent runs, from any checkout, queue on `$CONFORMANCE_LOCK` (default
`$XDG_RUNTIME_DIR/agterm-conformance.lock`, else `/tmp/agterm-conformance-<uid>/`), and a waiting run says so; the
deadlines assume one suite's load, and the lock does not hold back a stand-alone `cargo build` or `cargo test`.

```sh
conformance/tools/run-linux.sh                          # full run
conformance/tools/run-linux.sh -- -k test_version       # partial run: listed ids not selected are not judged
.venv/bin/python conformance/tools/check_ratchet.py \
  --report conformance/reports/rust-linux/nodeids.jsonl --add  # merge the newly green ids into rust-passing.txt
conformance/tools/run-linux.sh --ratchet-only \
  --also tests/protocol/test_split.py --also-k sidebar       # the listed ids plus what a task works on
```

The build is the GPUI one (`bin/ngterm`; the GTK frontend, and the GPUI ratchet kept beside its own while both
coexisted, went with the plan's Task 34, the GPUI list folded into `rust-passing.txt`).
It renders through Vulkan, here on Mesa's lavapipe and nothing else: the runner takes its ICD manifest
from `scripts/fetch-lavapipe.sh` (`NGTERM_LAVAPIPE_ICD`, else the distribution's package, else the pinned Arch package
verified and unpacked into the vendor dir) and hands it to the harness as `CONFORMANCE_LAVAPIPE_ICD`; without one the
run exits 2.
The adapter makes it the instance's only Vulkan driver (`VK_DRIVER_FILES`), forces GL onto llvmpipe, drops every
inherited device selection, refuses a manifest that does not load lavapipe, and starts the instance's weston with
`--fake-seat` (GPUI panics on a compositor without a `wl_seat`).
`--fake-seat` needs weston 15.0 or later: where the weston on PATH lacks it (Ubuntu 26.04 ships 14), the runner takes
the one `scripts/build-weston.sh` vendors, else exits 2 naming that script, and the adapter refuses such a weston with
the same pointer. The ICD manifest must name a driver that is there.

`rust-passing.txt` holds one pytest node id per line, verbatim (an id may end in spaces) and sorted, after its `#`
header.
`check_ratchet.py` prints the breaks and the newly passing ids (into the job summary in CI); `--add` writes only when
the ratchet holds, and never removes an id.
What breaks it is listed in [docs/rust-build.md](docs/rust-build.md) > Ratchet.

A full run waits out the read-back of every red test (a GPUI run short of parity: ~47 min, almost all of it in tests
that cannot pass yet), so `--ratchet-only` runs just the ids the list holds, plus each
`--also` directory, file or node-id prefix (a path outside the protocol, cli and behaviors layers is collected too)
and each `--also-k` `-k` expression, repeatable: a task names what it is working on.
An `--also` or `--also-k` that matches no collected test is a usage error (exit 2), unless `-k`, `-m` or node ids
narrowed the run. The plugin options behind it are `--ratchet-only LIST`, `--also` and `--also-k`. The nodeid report's run header then says `"mode": "ratchet-only"`, and
`check_ratchet.py` judges it as a full run: every listed id must pass, a listed id that was not collected is warned
about, and `--add` adds only ids that ran and passed. A narrower selection (`-k`, `-m` or node ids after `--`) makes
the mode `partial`: the listed ids that did not run are not judged, so a partial run can neither break nor shrink
the list through what it left out. The report names the mode and the suite time.
A full run stays the script's default, but locally a change runs only a partial run (`-- -k EXPR`) over the ids that
cover the behavior it touches (docs/development.md > Test); full and ratchet-only runs are CI's, which holds the whole list on every
PR and push to main. A partial run finds newly passing ids only among what it selected.
`--nodeid-report` cannot be combined with `--coverage-gate`.

## Catalog and coverage gate

`catalog.yaml` (commands, fields, events, CLI subcommands, behaviors, exemptions) and `ui-ids.yaml` are generated
from an upstream tag:

```sh
.venv/bin/python conformance/tools/gen_catalog.py --tag v0.41.1 --repo ../agterm   # or --source <extracted tree>
```

Generated sections are replaced and statuses survive; `behaviors` and `exemptions` are hand-maintained.
Retire an item with `status: removed-upstream`.

```sh
.venv/bin/python -m pytest conformance --coverage-gate
```

The gate launches nothing.
It fails on any active item that no `covers` names and no exemption with a reason excuses, and on a `covers` of an
unknown or removed item.
`covered`/`uncovered` statuses are informational.

## Code coverage

`run-linux.sh --coverage` (and `run-windows.sh --coverage`) measures the Rust code a suite run exercises, with
cargo-llvm-cov and the toolchain's `llvm-tools` component:

- build: `cargo llvm-cov clean --workspace`, then the release build under `cargo llvm-cov show-env` into
  `target/llvm-cov-target`, staged into `target/conformance-stage-cov` (a `--stage` that is not such a build exits 2);
- run: every process the harness starts from the build (the app, `ngtermctl`, `ngterm-session attach`, and what they
  start) writes a profile pooled per binary into `target/llvm-cov-target` (`agterm_conformance/llvm_profiles.py`); one
  the harness had to end past its exit (a kill, a SIGTERM without a quit, the Windows job's close) writes none and is
  listed in `DIR/lost-profiles.txt`, so a drop can be told from lost data;
- report: the profiles merged and read against each staged program on its own, then joined: a line, region or
  function counts as run when any program ran it (llvm-cov given several programs at once keeps the first record of a
  function it meets, often a program's unused one, and drops the run's counts). Into `DIR/coverage/` (`lcov.info`,
  `html/` from `cargo llvm-cov report` as a browsing aid only, `summary.md`: line, region and function % per workspace
  crate and the total, `vendor/` and the `-sys` crates left out; `agterm_conformance/code_coverage.py`),
  its table in `report.md`. A run that left no profile, or a stage that is not the instrumented build's, exits 2.

The runners still judge a coverage run's ratchet and exit with that verdict; the CI coverage jobs ignore its exit 1
(instrumentation slows the app; the ids stay the normal run's) and judge the floors separately:
`conformance/coverage-floor-linux.txt` and `coverage-floor-windows.txt` hold one `crate line%` per line, and

```sh
.venv/bin/python conformance/tools/check_coverage.py --summary DIR/coverage/summary.md \
  --floor conformance/coverage-floor-linux.txt          # 0 holds, 1 a crate > 0.5 pt below its floor, 2 bad input
.venv/bin/python conformance/tools/check_coverage.py --floor conformance/coverage-floor-linux.txt --add \
  --summary A/coverage/summary.md --summary B/coverage/summary.md   # raise the floors from a batch of runs
```

names each crate more than 0.5 pt below its floor with both numbers. `--summary` repeats, and each crate is judged by
its lowest value; `--add` takes at least two summaries of one build (one run's line % varies by more than the
tolerance) and raises each floor to the batch's lowest, never lowering one; a crate new to the workspace is added, a
removed one dropped. Raise the floors from CI runs of the same commit.

A crate whose % moves between runs of one build has code the suite runs only on some timing. `coverage_diff.py` names
it from two or more runs' `lcov.info` (the CI coverage artifacts of one commit): each crate's % per run and its spread,
and each group of lines some runs hit and others missed, with the runs, the function and the source line:

```sh
.venv/bin/python conformance/tools/coverage_diff.py A/coverage/lcov.info B/coverage/lcov.info C/coverage/lcov.info \
  --max-spread 0.1                                     # 0, 1 a crate's spread over 0.1 pt, 2 bad input
```

Fix such a line in the test or the harness (wait on a read-back of the state that runs it), not with a wider
tolerance.

## Safety guards

These runs share a machine with a live terminal (an installed ngterm, or an agterm), so the harness refuses a launch
that could reach it:

- `NGTERM_*`, `AGTERM_*` and upstream's `ZMX_*` (which the Swift reference's CLI reads) are scrubbed from the
  environment at plugin import, and the launch env may carry only the harness's own.
- Each instance gets a fresh root (`/tmp/agt-*`, or under `CONFORMANCE_TMP`) holding its `HOME`, XDG homes, state
  dir and socket; the guards refuse any of them outside it.
  Its live-session socket directory (the Rust build's session server namespace, the reference's daemon directory)
  derives from that state dir.
- Linux instances get a unique app id (`NGTERM_APP_ID`), never the installed one, and a private `XDG_RUNTIME_DIR`.
- Linux instances draw into their own headless weston with their own dbus-daemon, so no window, notification or
  a11y object reaches the desktop.
  The harness requires both programs.
- A Linux ui instance draws into its own Xvfb (`-displayfd`, no TCP) with its own dbus-daemon, and publishes its
  accessibility tree on an a11y bus the run started (a private session bus with `at-spi-bus-launcher` and
  `at-spi2-registryd`) with `GSETTINGS_BACKEND=memory`. The harness refuses an inherited `AT_SPI_BUS_ADDRESS`, an
  `Atspi.init()` before the run's bus is set, a `DISPLAY` that is not the instance's own Xvfb and a ui run outside
  the headless display mode.
  `CONFORMANCE_LINUX_DISPLAY=session` uses the desktop's display and bus instead, for debugging only.
- A GPUI instance renders on lavapipe only, in either display mode: never on the machine's GPU driver.
- URI and file launches go to a no-op handler through the isolated `mimeapps.list` and `BROWSER=true`.
  The ghostty config turns off copy-on-select and clipboard writes.
- `cli` gives every `agtermctl` call the instance's `--socket`; a call without one must resolve under the root.
- Stopping reaps Live daemons over the protocol (`live.kill`, in the adapter's vocabulary,
  [docs/rust-build.md](docs/rust-build.md)), SIGTERMs only the launched pid, and fails on the Rust build's session
  server (`ngterm-session server <its namespace>`) or a process left with its `NGTERM_LIVE_DIR`, once the server's idle
  grace has passed, or on any of the reference's daemon or session-host processes left with its directory variable.
  The macOS session host outlives the app by design: the harness ends only the one started from the instance's own
  bundle copy, by exact pid, re-checking ownership before each signal.
- A macOS instance runs from its own copy of the bundle, because a second launch of the same bundle path binds no
  socket ([docs/spike-macos.md](docs/spike-macos.md)).
- `CFFIXED_USER_HOME` does not isolate the `com.umputun.agterm` defaults domain, which every macOS instance shares
  with the account.
  Run reference-macos only on an ephemeral runner, never on a Mac with a live agterm.
- A Windows instance (the Rust build's) keeps its local and roaming application data (`LOCALAPPDATA`, `APPDATA`) in
  its root with its home, so its live-session namespace (`<LOCALAPPDATA>\ngterm\live\<hash>`) and everything a pane
  writes per user stay there too; its control socket's path names a named pipe, which nothing else serves. It gets a
  unique app id, starts in a process group of its own, quits on the session's end posted to its hidden window (the
  Restart Manager's `WM_QUERYENDSESSION`/`WM_ENDSESSION`), and its process tree (every app process launched on the
  root and their descendants, each re-checked by its creation time) is ended by pid at the stop, never by image name;
  a Job object the harness puts each of its app processes in ends what is left of them at the stop, a program whose
  parent chain no longer leads to the app included (Git for Windows' `sh.exe` launcher, ended alone, leaves its
  `bash.exe` running), while breakaway lets the live-session server leave the job as it is built to. Windows offers no
  headless desktop and no no-op URI handler: the instance's window opens on the desktop the run is on, and
  `session.reveal` or a clicked link reaches the system's handlers. Run it only on a CI runner or a throwaway VM,
  never beside a live ngterm.

## CI

- `conformance-gate` (ubuntu): self-tests and the coverage gate on every PR and on `main`, unless only Markdown
  outside `conformance/docs/` changed.
- `reference-macos` (`macos-26`): protocol, cli and behaviors with `--must-pass live-sessions`, then the ui layer
  with `--reruns 1`, against the release named by the catalog's `upstream`; failure traces are kept with the report.
- `linux` (`ubuntu-26.04`): the Rust workspace's format check, strict clippy, lint report (in the job summary)
  and `cargo test`, over the vendored libghostty-vt prefix, without what links WPE WebKit; `wpe` (an Arch container):
  the WPE crates and the app with its HTML overlays; then the staged build under the suite in the same Arch container,
  on the distribution's lavapipe, held by the ratchet, once per login shell (`conformance`, a matrix, every layer, the
  ui layer through `atspi`), each with `--must-pass live-any-shell`: bash, fish and zsh (the last two with
  `--must-pass live-sessions` too); the matrix's `coverage` entry runs the zsh leg's suite (the widest: the bash leg's
  tests and the live ones) on the instrumented build (`--coverage`) and judges `coverage-floor-linux.txt` only, its
  report and `coverage/` uploaded as `rust-linux-coverage-conformance`.
- `cross-platform`'s `windows` job (`windows-2025`): the harness's Windows self-tests (the pipe transport, the client,
  the launch and stop, the UIA driver's mappings), then `tools/run-windows.sh` with `--must-pass live-any-shell` (the
  login shell is Windows PowerShell): protocol, cli, behaviors and ui (UI Automation and `SendInput`,
  `drivers/uia.py`, its windows on the runner's desktop) against the staged Windows build, held by
  `rust-passing-windows.txt`. The
  staged tree is kept as an artifact, for a run on a Windows VM against the same build (`--stage DIR`).
  `coverage-windows` runs the same suite on the instrumented build (`run-windows.sh --coverage`) and judges
  `coverage-floor-windows.txt` only, its report uploaded as `rust-windows-coverage-conformance`.
