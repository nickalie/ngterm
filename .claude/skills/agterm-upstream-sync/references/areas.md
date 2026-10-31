# Areas of an upstream diff and their Rust side

`classify-diff.py` sorts each changed upstream file into one of these areas. This file says what the area means for
the port, where its Rust counterpart lives, and how it turns into plan work. Paths on the left are upstream's (in
`.upstream/agterm`), on the right this repository's. The crate map is in `CLAUDE.md`; when this table and the code
disagree, the code wins: check with `ls`/`grep` before naming a Rust file in the plan.

## Contents

- Behavior areas: protocol, dispatcher, app-control, cli, core, session-host
- Platforms
- Evidence areas: Swift tests, contract docs, upstream design notes
- UI
- Vendored copies and native pins
- No-port and review
- Telling ported from not ported

## Behavior areas

| area | upstream | Rust side | what to look for |
|---|---|---|---|
| protocol | `agtermCore/Sources/agtermCore/Control*.swift`, `ControlDispatcher.swift` | `ngterm-proto` (`command.rs`, `request.rs`, `result.rs`, `events.rs`, `tree.rs`), `ngterm-core/src/dispatch.rs`, `projection.rs` | new or renamed wire commands, args, result fields, tree fields, events, mode changes (`ControlModes.swift`: request vs stream), options parsing (`ControlDispatcherOptions.swift`) |
| dispatcher | `ControlDispatcher+<Group>.swift` | `ngterm-core/src/commands/<group>.rs` (+ `commands/tests/`) | validation order, refusal strings, defaults, read-back |
| app-control | `agterm/Control/ControlServer+<Group>.swift`, `ControlServer.swift` | `ngterm-gpui` (the app side of the group) and `ngterm-server` (socket: listen backlog, connection handling) | what the app does once the dispatcher accepted a command; socket-level changes |
| cli | `agtermCore/Sources/agtermctlKit/*` | `ngterm-ctl/src/<group>.rs`, `usage.rs`, `output.rs`, `socket.rs`, `client.rs` | subcommands, flags, help text, output format, exit codes, retry/backoff, local (no-socket) commands |
| core | other `agtermCore/Sources/agtermCore/*` | `ngterm-core/src/*` (model, persist, keymap, hooks, settings_model, remote, hud, palette, ...) | model and store behavior, and every on-disk format (`settings.json`, `windows.json`, keymap/hooks files, `remotes.conf`, `restore-denylist.conf`, ...): the format is part of the contract, the suite reads it back |
| session-host | `agtermCore/Sources/SessionHostRuntime/*`, `SessionHostTrampoline/`, `AgtermResponsibility/`, `agterm-session-host/` (tests: `Tests/SessionHostRuntimeTests/`, fixtures `session-host-*`) | `ngterm-session` (server, pane runtime, endpoint, detached spawn, attach), `ngterm-pty`; attribution: `liveAttribution` in `ngterm-proto/src/tree.rs`, set in `ngterm-core/src/commands/live/launch.rs`, the Linux classifier in `commands/live/live_reset.rs`, the `Attribution` enum in `ngterm-core/src/live/reset.rs` | upstream's own session host, which spawns, supervises and lists the Live daemons: what a pane sees while it is created and attached (startup replies such as device attributes), socket and pidfile lifetime, process attribution |

**The `zmx` group is `live` here.** Upstream's `zmx.*` commands, `agtermctl zmx`, `result.zmx` and `backedByZmx` are
`live.*`, `ngtermctl live`, `result.live` and `backedByLive` in the Rust build, and its strings that name zmx speak of
live sessions (the table of renamed strings: `conformance/docs/rust-build.md` > The `live` group). An upstream change
to `ControlDispatcher+Zmx.swift`, `ControlServer+Zmx.swift` or `ZmxCommands.swift` ports onto
`ngterm-core/src/commands/live*`, `ngterm-core/src/remote/` and `ngterm-ctl/src/live.rs` under those names: a new
command `zmx.foo` becomes `live.foo`, a new string naming zmx gets the live-session wording and a row in that table,
and a test the suite adds keeps upstream's names, read through the adapter's vocabulary
(`conformance/agterm_conformance/vocabulary.py`, with a `LIVE_TEXTS` entry for a new text that names zmx). What
upstream's zmx binary speaks (`zmx attach`, `ZMX_*`, `list`/`screen` output, the patched `zmx-role;` title) is
`ngterm-session`'s here, under its own names (`ngterm-session attach`, `NGTERM_LIVE_*`, `ngterm-live-role;`; the same
table): this build runs no zmx, so an upstream change to zmx or to its patches (`pins-delta.sh` reports both) is
ported into `ngterm-session` (`cli`, `attach_tty`, `server::lead`, `server::snapshot`) under those names.

**Upstream's session host is `ngterm-session` here.** `SessionHostRuntime` is the Swift supervisor in front of
upstream's zmx daemons; this build has one server per namespace that is both (`ngterm-session`, herdr's
architecture). A change there is ported as behavior into `ngterm-session` or `ngterm-pty`, on all three OSes: a
startup reply a pane needs, a socket published only once it listens, a file kept from aging out of the temp dir.
`AgtermResponsibility` is macOS's process attribution (who a Live child counts as for privacy prompts); its
counterpart is part of `ngterm-session`'s macOS form, and the plan records what Linux and Windows need of it.

**herdr is not upstream.** `ngterm-session` is herdr's session server taken as code (`HERDR_REV` in
`build/pins.env`); a herdr change is ported into `ngterm-session` as a change of its own, never through this sync.

A behavior change usually spans several of these at once (a feature commit touches protocol + dispatcher + cli +
core + ui). The plan groups it by **command group or feature**, not by area: one task per group, holding every area
that group's change touches. The per-commit block in the classify report ("Commits by area") is the seam to cut along.

## Platforms

Upstream is macOS only; ngterm targets all three OSes (CLAUDE.md > Platforms). A behavior upstream builds on a macOS
API (keyboard layouts, the Command key, pasteboard, Notification Center, dock, Finder) gets a counterpart per OS, or a
recorded reason why an OS has none (a Homebrew cask fix, an AppKit bug workaround). Put OS-specific logic in an `any`
crate where it can live, since CI builds only those on macOS and Windows.

## Evidence areas

- **Swift tests** (`tests-core`, `tests-cli`, `tests-app`, `tests-ui`): the source of exact strings, edge cases,
  validation order and field order. Every plan task names the Swift test files (and the test functions the classify
  report lists) its port asserts against; the Rust unit tests assert the full upstream string on the dispatcher's
  output. `tests-ui` adds or changes accessibility-driven checks: they map to `conformance/tests/ui` and
  `ui-ids.yaml`, which the Linux build cannot run (no ui driver), so they become items of the plan's UI-only task.
- **Contract docs** (`site/commands.html`, `site/docs.html`, `.claude/rules/*.md`, `docs/troubleshooting.md`): the
  documented contract. `commands.html` feeds the catalog's `cli` section; a rules file states the subsystem's
  invariants. Where a doc and the Swift code disagree, read both and record the conflict in the task.
- **Upstream design notes** (`docs/plans/`, `docs/backlog/` upstream): why a feature is shaped the way it is, and
  known upstream bugs. Read them for intent; they are never ported, and an upstream backlog item is not a target.

## UI

`agterm/Views/*`, `agterm/Ghostty/*`, `agterm/*.swift` (menus, app actions, settings model), `agterm/Commands/*`,
`agterm/Notifications/*` map to `ngterm-gpui` in GPUI idioms. The suite cannot see this UI on Linux, so
each UI change becomes an item of the plan's UI-only task with its GPUI form, plus Rust unit tests where the logic lives in `ngterm-core` (palette, keymap, settings model, link policy...).
Platform-specific changes (Finder reveal, a macOS keyboard-layout API, an AppKit workaround) follow Platforms above:
a form per OS, or a recorded "no counterpart on <OS>" with its reason.

## Vendored copies and native pins

`sync-resources.sh` handles the copies mechanically; only its `conflict` lines become plan tasks. Upstream's pins
and zmx patches have no copy here: `pins-delta.sh` reports their moves for the plan.

| upstream | copy here | notes |
|---|---|---|
| `agterm/Resources/agent-status/` | `resources/agent-status/` | upstream's tree plus `build/patches/agent-status/`: the Linux glue (the Codex adapter reads JSON with python3/jq; the Pi and OpenCode plugins call the wrapper in `~/.config/ngterm/agent-status`), then this build's `ngterm-*` file names and `NGTERM_*` variables; a conflict names the patch to refresh, keeping both |
| `plugins/agterm/skills/agterm/` | `resources/skills/ngterm/` | the agent skill the app installs, as `ngterm`: upstream's tree plus `build/patches/skill-ngterm/` (the `live` names, this build's names: its frontmatter `name` and description, and every command it runs is `ngtermctl`, the far side of a remote attach included, then the attach contract's `NGTERM_LIVE_*` variables), rebuilt whole on every sync; a `conflict` names the patch to refresh against the new tree (regenerate it as `diff -u` of upstream's files and the hand-merged copy), and `sync-resources.sh --check TAG TAG` verifies the copy. An upstream change that brings new `agtermctl` calls is renamed the same way in the names patch |
| `agterm/Resources/hud/` | none | `hud.sh`, upstream's panel painter, is ported to Rust (`crates/ngterm-core/src/hud/painter.rs`, painted in process by `ngterm-term`, no shell on any OS); `classify-diff.py` files a change to it under `core`, ported as code with the painter's byte-for-byte tests |
| `agterm/Resources/custom-themes/` | none | upstream's own `agterm` theme, its seeded default; this build ships no theme of its own (Ghostty's catalog only) and seeds none, so a change here is no-port |
| `agterm/Resources/ghostty-defaults.conf` | `crates/ngterm-core/src/ghostty_config/ghostty-defaults.conf` | adapted (Linux chords, comments): usually a conflict, merge the setting, keep the Linux binds |
| `scripts/zmx-patches/*.patch` (+ `ghostty/`) | none | upstream's own zmx patches; this build runs no zmx, so `pins-delta.sh` reports a change and a plan task ports it into `ngterm-session` (the leadership patch is `server::lead`); `ghostty/` is also `build/patches/ghostty-vt/0004` (below) |
| `LICENSE` | `resources/LICENSE.upstream` | |
| `scripts/setup.sh` (`ZMX_REV`) | none | not a pin here: `pins-delta.sh` reports a move, a plan task ports what zmx changed into `ngterm-session` |
| `scripts/setup.sh` (`GHOSTTY_REV`) | none | not a pin here: the Ghostty pin is `LIBGHOSTTY_VT_REV` (ghostty main), `pins-delta.sh` reports upstream's move |

`build/patches/ghostty-vt/*.patch` are this repository's libghostty-vt patches (C API the app needs, and recorded fixes
and parity behaviors; its `README.md` lists each with its origin): a `LIBGHOSTTY_VT_REV` bump, a change of its own,
re-applies them (`scripts/build-ghostty-vt.sh` does, with `--fuzz=0`), and `scripts/gen-ghostty-vt-bindings.sh`
regenerates the checked-in bindings afterwards, `scripts/gen-ghostty-config-keys.sh` ngterm-core's ghostty config key
table. One of them is upstream's: `scripts/zmx-patches/ghostty/0001-formatter-unstyled-blanks.patch`, rebased onto the
Ghostty pin as `0004-formatter-unstyled-blanks.patch`, so an upstream change to it is ported into that copy.

A new entry under `agterm/Resources/` (reported `unmapped`) has no copy yet: deciding where it goes is a plan task.

## No-port and review

`no-port` (upstream CI, packaging, release metadata, the plugin manifests' version bumps, site pages other than
commands/docs, upstream's own CLAUDE.md, other scripts) goes to the report's "no port needed" list with one line
each, so the user can see nothing was dropped silently. `review` is a path the table does not know: decide its
area by reading it, and add a rule to `classify-diff.py` when it will recur.

## Telling ported from not ported

Grep `crates/` for the new wire names, fields, subcommands and strings (without the Swift interpolation), run
`coverage-map.py <key>`, and compare the Swift tests with the module's Rust tests. A behavior that comes from a
dependency rather than our code counts as already in the build, with that evidence.
