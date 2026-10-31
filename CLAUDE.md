# ngterm

ngterm: upstream agterm in Rust, for macOS, Linux and Windows, with one frontend on GPUI that renders libghostty-vt
itself (built and tested on Linux and Windows; its macOS form is open work).
Build, run and test: [README.md](README.md). The suite and the ratchet: [conformance/README.md](conformance/README.md),
[conformance/docs/rust-build.md](conformance/docs/rust-build.md).
Plans in `docs/plans/` record what was decided at the time, not the current state: check the code. Scope a task from
the user's words and the code; use a plan as input when the user points at it.

## Platforms

- The aim is parity on macOS, Linux and Windows. When you add a platform-specific mechanism (PTY, IPC, process tables,
  daemon spawn, paths, notifications), cover the other OSes too where you can, and say so when you leave one open.
  Behavior on another OS is best confirmed on its CI runner.
- Prefer cross-platform crates (`portable-pty` with ConPTY, `interprocess`), `windows-sys` for the rest; POSIX-only
  code sits behind `cfg`.
- On Linux, don't assume one compositor: the dev box runs niri, users run anything (KDE, GNOME, wlroots, X11).

## Crates

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

A crate's root docs and module list show what it holds. `any` means CI builds and tests the crate on all three OSes
(README > CI has the list), so its dependencies need to build there too.

GPUI owns the main thread and the state; other threads hop onto it through `MainLoop`. Work that needs no terminal or
widget fits in `ngterm-core`, tested on the `ngterm-host` fakes; terminal work that needs no window fits in
`ngterm-term`, tested on real PTYs. Keeping both toolkit-neutral leaves room for a future TUI frontend.

## References

- **Upstream** (`.upstream/agterm`, README > Layout) is the reference for the protocol, command semantics, error
  strings, `agtermctl` output, on-disk formats, the model and the UI. Its sources and Swift tests hold the exact strings.
- **Ghostty at the libghostty-vt pin** (`build/pins.env`) is the terminal's reference: the C API for what the terminal
  does, Ghostty's source at that revision for what it leaves to the embedder. Other terminals (qwertty-term,
  gpui-ghostty, rustty) are useful read-only references, credited in module docs. Zed's terminal is GPL: ideas only.
- **herdr** (`.upstream/herdr`, Apache-2.0) is where the session server and PTY actor come from. Derived files name
  their herdr source and `NOTICE` carries the license; what the app shows follows upstream, not herdr's names.
- **Identity** is ngterm's own (README lists the id, dirs, sockets, binaries, env vars): agterm names appear in
  citations and the reference build's harness adapter, not as fallbacks.

## Porting a command group

A path that works well:

1. Read upstream's dispatcher, the `AppStore+*` and `ControlServer+*` code behind it, its unit tests,
   `site/commands.html` and its matching `.claude/rules/*.md`.
2. Implement in `ngterm-core` with tests on the fakes, using upstream's error strings.
3. Wire the frontend in `ngterm-gpui` (terminal behavior in `ngterm-term`), following upstream's UI in GPUI idioms.
4. Port the `ngtermctl` subcommands from upstream's `agtermctlKit`.
5. Claim a capability (rust-build.md > Rust target profile) once its commands implement it.
6. Check it as [Choosing checks](#choosing-checks) says. Newly green ids can join the ratchet with
   `check_ratchet.py --add`.

Upstream syncs and pin bumps have the `agterm-upstream-sync` skill and `build/pins.env`.

## Choosing checks

Pick the checks a change needs from what it can break; there is no fixed gate. Docs or Markdown alone: no build, no
tests, and no waiting on CI (CI skips such a push, except for the Markdown it builds or ships, such as the bundled
skill under `resources/`). A comment or a script: whatever exercises it. Code: clippy
and tests on the touched crates, plus a partial suite run (README > Test) over the ids for behavior it touches. The
whole workspace, the full suite and the other OSes are CI's; wait on CI only when its result changes what you do next,
such as code for another OS that only its runner proves. The same judgment covers reviews (revmux and the like): worth
it for real code changes, not for a typo.

## What CI checks

Knowing these up front saves a red run:

- The ratchet (`conformance/rust-passing.txt`): an id listed there that goes red fails CI. A flaky test usually points
  at a real bug in the build or the test.
- Strict clippy with the `clippy.toml` complexity limits. Splitting a long function is the usual fix; an item-level
  `#[expect(clippy::..., reason = "...")]` covers the rare case where splitting hurts.
- Generated bindings and `vendor/` are kept in sync by their generator and vendor scripts (`--check` runs in CI), so
  change them through those scripts.
- Ghostty patches: gaps Rust can fill are usually best filled in Rust; `build/patches/ghostty-vt/README.md` describes
  how patches are recorded.

## Working beside the user's live ngterm

Agents usually run inside a pane of the user's own ngterm, so keep dev runs off it: launch a checkout's build
through `scripts/run-gpui-dev.sh` or the suite, strip `NGTERM_*` from the environment, give
`ngtermctl` an explicit `--socket`, use a private state dir, run the GUI headless (README > Test), and reap sessions
through their own server rather than by process name. A Windows GUI run goes on a CI runner or a throwaway VM.
