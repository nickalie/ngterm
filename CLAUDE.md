# ngterm

A Rust port of upstream agterm for macOS, Linux and Windows, with one GPUI frontend that renders libghostty-vt itself.

- Build, run, test, CI, Windows setup and repo layout: [README.md](README.md).
- The conformance suite and the ratchet: [conformance/README.md](conformance/README.md),
  [conformance/docs/rust-build.md](conformance/docs/rust-build.md).
- `docs/plans/` records past decisions, not the current state: read the code instead. Scope a task from the user's
  words and the code; treat a plan as input only when the user points at it.

## Architecture

| crate | role | platform |
|---|---|---|
| `ngterm-proto` | wire types | any |
| `ngterm-host` | the seams a frontend implements, with fakes (feature `fakes`) | any |
| `ngterm-core` | model, dispatcher (one module per command group), config, hooks, persistence, settings | any |
| `ngterm-server` | the control socket server | any |
| `ngterm-ctl` → `ngtermctl` | the CLI | any |
| `ngterm-ghostty-vt-sys`, `ngterm-ghostty-vt` | libghostty-vt bindings and their safe wrapper | any |
| `ngterm-pty` | PTY layer (openpty, ConPTY) | any |
| `ngterm-term` | the toolkit-neutral pane core | any |
| `ngterm-session` | the live-session server, its client and `SessionHost` | any |
| `ngterm-app` | frontend-neutral app services | any |
| `ngterm-headless` | the core, control socket and live sessions with no window (an Attach Remote origin) | any |
| `ngterm-wpe-sys` and kin, `ngterm-wpe` | WPE WebKit bindings and the HTML overlays' host | Linux |
| `ngterm-gpui` → `ngterm` | the GPUI frontend | Linux, Windows |

- `any` means CI builds and tests the crate on all three OSes, so its dependencies must build there too.
- A crate's root docs and module list describe what it holds.
- GPUI owns the main thread and the state; other threads hop onto it through `MainLoop`.
- Put work that needs no terminal or widget in `ngterm-core` (tested on the `ngterm-host` fakes), and terminal work
  that needs no window in `ngterm-term` (tested on real PTYs). Keep both toolkit-neutral.

## Platforms

- Target parity on macOS, Linux and Windows. A platform-specific mechanism (PTY, IPC, process tables, daemon spawn,
  paths, notifications) should cover all three; say so when one is left open.
- Prefer cross-platform crates (`portable-pty`, `interprocess`), then `windows-sys`; keep POSIX-only code behind `cfg`.
- On Linux, don't depend on one compositor or desktop: give compositor-specific code a fallback.

## References

- **Upstream** (`.upstream/agterm`): the reference for the protocol, command semantics, error strings, `agtermctl`
  output, on-disk formats, the model and the UI. Its sources and Swift tests hold the exact strings.
- **Ghostty at the libghostty-vt pin** (`build/pins.env`): the terminal's reference. The C API covers what the
  terminal does; Ghostty's source at that revision covers what it leaves to the embedder. Other terminals
  (qwertty-term, gpui-ghostty, rustty) are read-only references, credited in module docs. Zed's terminal is GPL:
  ideas only, no code.
- **herdr** (`.upstream/herdr`, Apache-2.0): the source of the session server and PTY actor. Derived files name their
  herdr source; `NOTICE` carries the license. User-visible behavior follows upstream, not herdr.
- **Identity** is ngterm's own (README lists the names). agterm names appear only in citations and the reference
  build's harness adapter, never as fallbacks.

## Porting a command group

1. Read upstream's dispatcher, the `AppStore+*` and `ControlServer+*` code behind it, its unit tests,
   `site/commands.html` and its matching `.claude/rules/*.md`.
2. Implement in `ngterm-core` with tests on the fakes, asserting upstream's error strings.
3. Wire the frontend in `ngterm-gpui` (terminal behavior in `ngterm-term`), following upstream's UI in GPUI idioms.
4. Port the `ngtermctl` subcommands from upstream's `agtermctlKit`.
5. Claim a capability (rust-build.md > Rust target profile) once its commands implement it.
6. Check it as [Choosing checks](#choosing-checks) says; add newly green conformance ids to the ratchet.

Upstream syncs and pin bumps: use the `agterm-upstream-sync` skill.

## Choosing checks

There is no fixed gate: you decide which checks a change needs and how far to take them. Don't overrun the gate:
size it to what the change can break and stop there. A small change gets a small, fast check, not a long build or
suite run; a check the change can't affect is skipped, and so is waiting on CI unless its result decides your next
step. Reviews (revmux and similar) scale the same way. README > Test has the commands and how to scope them.

## What CI enforces

- The ratchet (`conformance/rust-passing.txt`): a listed id that goes red fails CI. Treat a flaky test as a bug in
  the build or the test, not as noise.
- Strict clippy with the `clippy.toml` complexity limits. Split a function that exceeds them; an item-level
  `#[expect(..., reason = "...")]` is the escape hatch, `#[allow]` is not.
- Generated bindings and `vendor/` are checked against their generator and vendor scripts: change them only through
  those scripts.
- Fill libghostty-vt gaps in Rust where possible; a Ghostty patch is recorded as `build/patches/ghostty-vt/README.md`
  describes.

## Never touch the user's running ngterm

Agents usually run inside a pane of the user's live ngterm. Keep every dev run off it:

- Launch a checkout's build only through `scripts/run-gpui-dev.sh` or the suite.
- Strip `NGTERM_*` from the environment of anything you launch.
- Give every `ngtermctl` call an explicit `--socket`.
- Use a private state dir; reap sessions through that dir's own server, never by process name.
- Run the GUI headless only (README > Test). A Windows GUI run goes on a CI runner or a throwaway VM.
