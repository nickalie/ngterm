# ngterm

ngterm: upstream agterm in Rust, for macOS, Linux and Windows as equal targets, with one frontend on GPUI that renders
libghostty-vt itself (built and tested on Linux so far; its macOS and Windows forms are open work, not a later phase).
Build, run and test: [README.md](README.md). The suite and the ratchet: [conformance/README.md](conformance/README.md),
[conformance/docs/rust-build.md](conformance/docs/rust-build.md).
Plans in `docs/plans/` record what was decided at the time: never read one as the current state, never update one to
match a later change.

## Platforms

- A feature is done when it works on macOS, Linux and Windows, not on the dev box; nothing ships as "Linux now, the
  others later". Code for another OS is proven on that OS's CI runner, not only by `cargo check`.
- A platform-specific mechanism (PTY, IPC, process tables, daemon spawn, paths, notifications) comes with its other
  two forms in the same change or plan; POSIX-only code sits behind `cfg` beside its Windows form.
- Prefer cross-platform crates (`portable-pty` with ConPTY, `interprocess`) over per-OS code, `windows-sys` for the rest.
- On Linux any desktop counts (wlroots, KDE, GNOME, weston, X11): every compositor-specific path has a fallback.

## Crates

| crate | role | platform |
|---|---|---|
| `ngterm-proto` | wire types | any |
| `ngterm-host` | the seams a frontend implements, with fakes (feature `fakes`) | any |
| `ngterm-core` | model, dispatcher (one module per command group), config, hooks, persistence, settings | any |
| `ngterm-server` | the control socket server | Linux, macOS |
| `ngterm-ctl` → `ngtermctl` | the CLI | Linux |
| `ngterm-ghostty-vt-sys`, `ngterm-ghostty-vt` | libghostty-vt bindings and their safe wrapper | any |
| `ngterm-pty` | PTY layer (openpty, ConPTY) | any |
| `ngterm-term` | the toolkit-neutral pane core | any |
| `ngterm-session` | the live-session server, its client and `SessionHost` | any |
| `ngterm-app` | frontend-neutral app services | Linux |
| `ngterm-wpe-sys` and kin, `ngterm-wpe` | WPE WebKit bindings and the HTML overlays' host | Linux |
| `ngterm-gpui` → `ngterm` | the GPUI frontend | Linux |

A crate's root docs and module list show what it holds. `any` means CI builds and tests the crate on all three OSes (README > CI
has the list); a crate joins it in the change that makes it so. An `any` crate depends on another crate only if that
one is `any` too, or behind the `cfg` of the OSes it builds on.

GPUI owns the main thread and the state; other threads hop onto it through `MainLoop`. Work that needs no terminal or
widget goes in `ngterm-core`, tested on the `ngterm-host` fakes; terminal work that needs no window goes in
`ngterm-term`, tested on real PTYs. Both stay toolkit-neutral.

## References

- **Upstream** (`.upstream/agterm`, README > Layout) is the reference for the protocol, command semantics, error
  strings, `agtermctl` output, on-disk formats, the model and the UI. Port from its sources and its Swift tests, which
  hold the exact strings.
- **Ghostty at the libghostty-vt pin** (`build/pins.env`) is the terminal's reference, not upstream's libghostty: the C
  API for what the terminal does, Ghostty's source at that revision for what it leaves to the embedder. Ported code
  cites that revision. Other terminals (qwertty-term first, gpui-ghostty, rustty) are read-only references, credited in
  module docs, never dependencies. Zed's terminal is GPL: ideas only.
- **herdr at its pin** (`.upstream/herdr`) is the source of the session server and PTY actor's design and code
  (Apache-2.0). Each derived file names its herdr source, license and that it was modified; `NOTICE` carries the
  license. What the app shows (`live.*`, its errors, the attach contract) follows upstream; no herdr name, path or
  socket reaches the build.
- Linux glue with no upstream counterpart follows the platform's specs (XDG Base Directory, GApplication's D-Bus
  protocol), never another app's build.
- **Identity is ngterm's own.** Application id, dirs, sockets, binaries, env vars, CLI, skill, install paths and the
  links shown to users are ngterm's (README holds them). Nothing falls back to, runs or reads an agterm name or path;
  those names appear only in citations and the harness adapter for the reference build. A rename covers every place
  at once; keeping an old name "for compatibility" is the user's call.

## Porting a command group

1. Read upstream's dispatcher, the `AppStore+*` and `ControlServer+*` code behind it, its unit tests, `site/commands.html`
   and its matching `.claude/rules/*.md`.
2. Implement in `ngterm-core` with tests on the fakes, asserting upstream's full error strings.
3. Wire the frontend in `ngterm-gpui` (terminal behavior in `ngterm-term`), following upstream's UI in GPUI idioms.
4. Port the `ngtermctl` subcommands from upstream's `agtermctlKit`.
5. Claim a capability (rust-build.md > Rust target profile) only once its commands implement it.
6. Gate: README > Test's checks on the changed part only (clippy on the touched crates and their dependents, tests on
   the touched crates, a partial suite run over the ids covering the touched behavior), never the whole codebase, not
   even at the end of a plan. The whole workspace and the full suite are CI's. Newly green ids go into the ratchet
   with `check_ratchet.py --add`.

Upstream syncs and pin bumps (upstream's zmx, herdr, Ghostty) follow the `agterm-upstream-sync` skill and
`build/pins.env`; a herdr, Ghostty or toolchain bump is a change of its own.

## Gates

- A ratchet id (`conformance/rust-passing.txt`) never goes red: one a scoped run or CI finds red is fixed in the change
  that broke it.
- No reruns: a flaky protocol, cli or behaviors test is a bug in the build or the test.
- A function over a `clippy.toml` limit is split. The only exception is an outer `#[expect(clippy::..., reason = "...")]`
  on that one item; `#[allow]` and inner `#![allow]`/`#![expect]` are refused. Thresholds are only ever lowered.
- A Ghostty patch exposes data the library already stores; one that changes its logic is taken only for a fix or parity
  Rust can't reach, recorded as `build/patches/ghostty-vt/README.md` says. A gap Rust can fill is filled in Rust.
- Generated bindings change only through their generator scripts; an edit under `vendor/` goes back into its patch
  through its vendor script, whose `--check` holds.

## Never touch the user's running ngterm

Agents usually run inside a pane of the user's live ngterm or agterm. Every dev instance, probe and `ngtermctl` call
stays off it:

- Launch a checkout's build only through `scripts/run-gpui-dev.sh` or the suite; a plain launch joins the running
  instance or its state.
- Strip `NGTERM_*`, `AGTERM_*` and `ZMX_*` from the environment before launching anything.
- Give every `ngtermctl` call an explicit `--socket` (after the subcommand name).
- Use a private state dir, so the live-session namespace and its session server are private too. Reap sessions only
  through that namespace's own server or the processes it names, never by name (`pkill`, `taskkill /im`), and never
  touch another namespace's server or pipe.
- Run the GUI headless only (the suite's private compositor and bus, or `xvfb-run` with a private bus), on lavapipe,
  never on the user's display, compositor, bus or GPU; README > Native dependencies and Test have the setup. The
  render-path harnesses' offscreen GPU use is the one exception.
