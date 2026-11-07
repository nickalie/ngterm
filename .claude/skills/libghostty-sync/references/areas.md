# Areas of a Ghostty diff and their ngterm side

`classify-diff.py` sorts each changed Ghostty file into one of these areas. This file says what the area means for
ngterm, where its counterpart lives, and how it turns into plan work. Paths on the left are Ghostty's (in
`.upstream/ghostty`), on the right this repository's. When this table and the code disagree, the code wins: the
report's citation checklist is built from the code, so trust it over the table, and check with `ls`/`rg` before naming
a file in the plan.

## Contents

- The parity contract
- Areas
- Platforms
- Apps and the session server
- Evidence: Ghostty's tests and docs
- No-port
- Telling mirrored from not mirrored

## The parity contract

ngterm's terminal behaves and renders as Ghostty does, as closely as the toolkit allows. The work splits by who owns
the behavior:

- **The VT** (`src/terminal/`, behind the C API): libghostty-vt itself. ngterm links it and never re-implements it, so
  its changes arrive with the bump. What ngterm owes is: its wrapper follows the API, its tests' expectations follow
  the new behavior, and the patches (`build/patches/ghostty-vt/`) still hold or are dropped.
- **What Ghostty leaves to the embedder** (renderer, font, sprite font, Surface, termio, input tables, config
  semantics): ngterm wrote these in Rust (and in Kotlin and Swift for the mobile renderers) as ports of Ghostty's code,
  each module citing its Ghostty source. A change to that source is ported into the mirror, in the mirror's own
  structure: the outcome (pixels, metrics, colors, bytes sent to the PTY, selection extents, config meaning) matches
  Ghostty's, not its internal design (GPU pipelines, threads, atlases).
- **A new C API** that exposes something ngterm computes on its own, or that a patch adds: ngterm moves to the official
  API (and drops the Rust gap-filler or the patch) once it covers the case, since the library's answer is Ghostty's
  answer by definition.

Precedence where Ghostty is not the last word:

- **ngterm's recorded deviations** win: README's "differs from upstream by choice" bullets, `conformance/docs/rust-build.md`
  (deviations, Rust-only extensions), the parity patches with their reasons (`build/patches/ghostty-vt/README.md`), and
  ngterm's built-in defaults that override Ghostty's for upstream agterm's behavior (`ghostty-defaults.conf`, e.g.
  `copy-on-select`). A Ghostty change that contradicts one is not ported toward Ghostty; the plan records it.
- **Upstream agterm** owns the app around the terminal (windows, tabs, sidebar, menus, the control protocol): a Ghostty
  change to its own app chrome is never a target here. Upstream's macOS app embeds full libghostty, so for the
  terminal itself Ghostty's behavior is upstream's too, except where upstream configures or patches its Ghostty.
- **Config keys** mean what Ghostty at the pin makes of them (rust-build.md says so), so a changed key meaning is a
  port, not a deviation, unless rust-build.md records otherwise.

## Areas

| area | Ghostty | ngterm side | what to look for |
|---|---|---|---|
| vt-api | `include/ghostty/vt.h`, `include/ghostty/vt/**`, `src/terminal/c/**`, `src/lib_vt.zig`, `src/lib/**` | `ngterm-ghostty-vt-sys` (generated `bindings.rs`, `zig_build.rs`, `build.rs`), `ngterm-ghostty-vt` (one module per header: `terminal.rs`, `render.rs`, `key.rs`, `mouse.rs`, `format.rs`, `selection.rs`, `search.rs`, `kitty.rs`, `snapshot.rs`, `sys.rs`, ...) | the report's C API delta: removed or changed declarations ngterm uses break the build (Task 1); added ones may replace a Rust gap-filler or a patch (an adoption task) or expose a behavior ngterm lacks; ABI/version gates (`test/abi/`) are Ghostty's, no-port |
| vt-core | `src/terminal/**` (not `c/`), `src/unicode/`, `src/simd/`, `src/datastruct/` | arrives with the bump; its consumers: `ngterm-term` (stream, effects, selection, search, kitty storage readers), `ngterm-session` (the server's VT: replies, snapshot, lead filter, `attach_tty`), `ngterm-ghostty-vt` tests (`tests/patches.rs`, `tests/effects.rs`) | behavior an ngterm test pins (a red test after the bump), a new OSC/DCS/mode the embedder must answer or act on (an effect with no handler in `ngterm-term/src/effects.rs`), fixes that make a patch removable, snapshot-format changes (live sessions survive app updates: check the session protocol's compatibility), parser tables ngterm copies (`ngterm-ghostty-vt/src/scan.rs`, `ngterm-session` filters cite `parse_table.zig`) |
| renderer | `src/renderer/**` (`generic.zig` above all: `rebuildRow`, cursor, colors, `image.zig`, `size.zig`, shaders) | `ngterm-gpui/src/terminal/`: `element.rs` (layer order, cursor, decorations), `colors.rs`, `row_cache.rs`, `kitty.rs` + `kitty/`, `background.rs`, `metrics.rs` (`size.zig`); mobile: iOS `NgtermTerminal/Drawing/*`, Android `ui/terminal/TerminalPainter.kt`, `TerminalDecorations.kt`, `GlyphCache.kt`, `ui/theme/TerminalPalette.kt` | color rules (inverse, faint, bold-is-bright, minimum contrast, selection and search colors, cursor color/text), what is drawn in which layer, cursor shapes and blink, padding and balance, image placement and z-layers, custom-shader-free effects; Metal/OpenGL plumbing is no-port unless it changes the output |
| font | `src/font/**` (not `sprite/`) | `ngterm-gpui/src/terminal/font.rs` + `font/`, `metrics.rs`; embedded fonts (`FONTS_*` pins, `scripts/fetch-fonts.sh`); mobile `TerminalFont.swift`, `GlyphCache.*` | metrics (`Metrics.zig`: cell size, baseline, underline/strikethrough position and thickness, `adjust-*` keys), fallback and discovery order, Nerd Font constraints, shaping features (`shaper/feature.zig`), synthetic bold/italic; CoreText vs FreeType differences follow Platforms below |
| sprite | `src/font/sprite/**` | `ngterm-gpui/src/terminal/sprite/` (one module per `draw/*.zig`, via qwertty-term) | a glyph's drawing, the codepoint ranges the sprite font claims, line thickness rules |
| sprite-goldens | `src/font/sprite/testdata/` | `crates/ngterm-gpui/tests/golden/` (Ghostty's test images, with its license) | refreshed in the same task as the sprite change they show, so the golden test stays green |
| termio | `src/termio/**`, `src/pty.zig`, `src/Command.zig` | `ngterm-term` (`launch.rs`, `effects.rs`, `pwd.rs`, `stream*`, `actions.rs`), `ngterm-pty`, `ngterm-session/src/server/replies.rs`, `ngterm-core/src/ghostty_config/launching.rs`, `ngterm-core/src/live/shell_integration.rs` | the replies and side effects `stream_handler.zig` gives the embedder (OSC 7/52/133/9/777, title, bell, color reports, clipboard), the child's environment and shell-integration injection (`Exec.zig`, `shell_integration.zig`), resize ordering, exit handling |
| surface | `src/Surface.zig`, `src/App.zig`, `src/apprt/{embedded,surface,action,structs,runtime,ipc}.zig` | `ngterm-gpui/src/terminal/` (`surface.rs`, `mouse.rs`, `links.rs`, `selection_clipboard.rs`, `clipboard.rs`, `input.rs`, `image_paste.rs`), `ngterm-term` (`actions.rs`, `selection.rs`, `wheel.rs`, `gesture.rs`), `ngterm-core` keymap actions | mouse selection and click counts, scrolling (wheel multipliers, alternate scroll), link detection and opening, paste safety, copy-on-select, binding actions' semantics, focus reporting, resize/padding decisions |
| input | `src/input/**` | `ngterm-gpui/src/terminal/keyboard.rs`, `keycodes.rs` (`keycodes.zig` table), `mouse.rs`; `ngterm-core/src/ghostty_config/binds*` (`Binding.zig`: triggers, actions, default binds); mobile key mapping (`KeyMapping.swift`, `KeyMapping.kt`: Ghostty's key names) | keycode tables, key names, new binding actions or trigger syntax (they also reach `gen-ghostty-config-keys.sh` and the keymap), default bindings, mouse encoding inputs; the encoders themselves are vt-api |
| config | `src/config/**` | `crates/ngterm-core/src/ghostty_config/` (`keys.rs` generated by `scripts/gen-ghostty-config-keys.sh`; `validate.rs` decides new value kinds; `parse.rs` for syntax), the code reading each key, `docs/backlog/implement-deferred-ghostty-config-keys.md` | new, renamed or removed keys (the regenerated `keys.rs` diff shows them), changed defaults or meanings of keys ngterm implements, new value syntax, compatibility handlers; a new key with no ngterm effect is either implemented (renderer/font/surface keys usually are) or added to the deferred-keys backlog item |
| resources | `src/shell-integration/**`, `src/terminfo/**` | vendored by `scripts/build-ghostty-vt.sh` into the prefix (staged by `scripts/stage.sh`); `ngtermctl terminfo install`; `ngterm-term/src/launch.rs` | changes come with the bump; check ngterm's own injection still matches (env vars, file names), the terminfo entry's capabilities against what the renderer and ngterm-term support, and the `ngtermctl terminfo` counterpart of Ghostty's `+ssh` terminfo handling |
| os | `src/os/**` | `ngterm-core/src/paths.rs` (`xdg.zig`), `ngterm-term/src/pwd.rs` (`hostname.zig`), link opening in `ngterm-gpui` | path resolution, hostname rules for OSC 7, open/URL handling; macOS-only helpers (`mach`, `cf_release_thread`) are no-port |
| cli | `src/cli/**` | `ngterm-ctl` where ngterm has the counterpart (`terminfo.rs` + `terminfo/`, `theme.rs`, `font.rs`), `ngterm-core/src/ghostty_config/parse.rs` (`args.zig`, `CommaSplitter.zig`: config value parsing) | config parsing changes (they reach `parse.rs`), the ssh terminfo flow; Ghostty's own actions with no ngterm counterpart are no-port |
| build | `build.zig`, `build.zig.zon`, `src/build/**`, `src/build_config.zig`, the lib-vt deps in `pkg/` (`wuffs`, `simdutf`, `highway`, `utfcpp`, `translate-c`, `android-ndk`, `apple-sdk`) | `build/pins.env` (`bump-pins.py`), `scripts/build-ghostty-vt.sh`, `scripts/lib/native.sh`, `crates/ngterm-ghostty-vt-sys/zig_build.rs`, patch 0002 | Zig version moves, package deps (themes, fonts), new build options or flags the lib-vt build needs, target list changes (the `ZIG_TARGETS` lists must stay equal), library naming and versioned exports |
| apprt-macos | `macos/**` | none directly: the reference for the terminal surface on macOS (`ngterm-gpui` terminal and `macos.rs`) | only behavior of the terminal view itself (IME, selection, scrolling, mouse, secure input, Quick Look, link hover); windows, tabs, menus, settings are agterm's domain: no-port |
| apprt-gtk | `src/apprt/gtk/**`, other `src/apprt/*` | none directly: the reference for the terminal surface on Linux | as above, for Linux: IME (preedit), primary selection, DnD into the terminal, scroll behavior; GTK window chrome is no-port |
| review | anything the table does not know | decide by reading it | add a rule to `classify-diff.py` when it will recur |

A Ghostty pull request often spans areas (a renderer feature with its config key and Surface wiring): the plan groups
by pull request or feature, not by area, so one task holds every ngterm side of one change.

## Platforms

ngterm draws through GPUI on macOS, Linux and Windows. Ghostty ships a macOS app (CoreText discovery and shaping,
Metal, AppKit) and a Linux app (fontconfig, FreeType, HarfBuzz, OpenGL, GTK). Where Ghostty's behavior differs per
platform, ngterm takes the macOS form on macOS and the Linux form on Linux; Windows, which Ghostty does not ship, takes
the Linux form unless a Windows convention decides otherwise, and the module records that choice (as
`selection_clipboard.rs` and `keycodes.rs` do). Text rasterization belongs to GPUI's text system on each OS, so
parity targets what ngterm computes: metrics, positions, colors, layers, sprite pixels.

## Apps and the session server

- **Desktop** (`ngterm-gpui`): every renderer, font, sprite, surface and input change.
- **Android and iOS**: they draw the terminal themselves from `ngterm-mobile`'s frames (iOS
  `mobile/ios/Packages/NgtermTerminal/Sources/NgtermTerminal/Drawing/`, Android
  `mobile/android/app/src/main/kotlin/.../ui/terminal/`), and name keys as Ghostty does. A color, cursor, decoration
  or glyph rule reaches them where they draw that thing; a key-name or keycode change reaches their key mapping. Mobile
  UI work goes through the `frontend-design` skill inside that app's `DESIGN.md`. A change that does not reach them is
  said in one line.
- **The session server** (`ngterm-session`) runs its own libghostty-vt: vt-core changes reach its replies, snapshot and
  filters. A live session outlives an app update, so a snapshot or wire change needs a compatibility answer.
- **`ngterm-mobile` and `ngterm-term`** build on every OS, Android and iOS included (`any` crates): a wrapper API change
  must keep them building.

## Evidence: Ghostty's tests and docs

- **Zig tests** live beside the code (`test "..."`). The report lists the ones each file adds or renames: they are the
  cases a Rust port's unit tests take over, with Ghostty's inputs and expected values.
- **Sprite test images** (`src/font/sprite/testdata/`) are the pixel truth for the sprite font.
- **The C API docs** (`include/ghostty/vt/*.h` comments, `dist/doxygen/`) state the contract of each function, its
  ownership and lifetime rules.
- **Pull request text** (`gh pr view N --repo ghostty-org/ghostty`) says why: read it for any change whose intent the
  diff does not show.

## No-port

CI, packaging (`nix/`, `flatpak/`, `snap/`, `dist/`), translations (`po/`), examples, benchmarks, the inspector, Ghostty's
own app chrome and IPC, other platforms' deps in `pkg/`, `include/ghostty.h` (the full libghostty ngterm does not link),
and the ABI baselines under `test/abi/`. The report lists them by directory; the plan's Context names them in one line
so nothing is dropped silently.

## Telling mirrored from not mirrored

- The report's **review checklist** lists every ngterm file citing a changed Ghostty file: read each one against the
  new Zig and decide.
- The **uncited changes** list holds mirrored-area files no ngterm file cites: search for the Ghostty function names
  the diff touches (`rg -n 'rebuildRow|cursorStyle'`), the config key, or the behavior; when ngterm has a counterpart
  that does not cite its source, the task adds the citation; when ngterm has none, decide whether the behavior is
  visible in ngterm (then it is a port) or not (then one line in the no-port list says why).
- A behavior that comes with the library (vt-core) counts as present once the bump lands, with an ngterm test showing
  it where the behavior matters to ngterm.
