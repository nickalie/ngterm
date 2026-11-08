---
worth: yes
where: build/pins.env:22
added: 2026-11-08
---
# the libghostty-vt pin trails Ghostty v1.4.0 and vt-1.0.0

`LIBGHOSTTY_VT_REV` is `81598385` (ghostty main, 2026-10-13). Ghostty has since tagged `v1.4.0` (`83fb956`,
2026-10-27, 249 commits ahead) and `vt-1.0.0` (`0fa3d577`, 2026-11-06, "libghostty-vt 1.0.0", 515 commits ahead, with
`v1.4.0` its ancestor). Both descend from the pin (`git merge-base --is-ancestor` in `.upstream/ghostty`). Found
reviewing upstream agterm's move of its own `GHOSTTY_REV` to v1.4.0 (#883, `6511255`) in Task 19 of
`docs/plans/20261107-upstream-sync-v0.41.1.md`. That move needs no port here: the fixed-memory trims it names
(ghostty #14130, #14137, #14138) are already at the pin, and its display-link pause (#14035,
`renderer/generic.zig`) and drain-mailbox patch (`App.zig`, `Surface.zig`, renderer and io threads) are in the
full app library, which libghostty-vt does not build (`src/lib_vt.zig` imports only `terminal/`, `input/`,
`unicode/`).

Move the pin as its own change, with the `libghostty-sync` skill (`.claude/skills/libghostty-sync/`):
`/libghostty-sync vt-1.0.0` (or `v1.4.0`) does the analysis, the bump when it is mechanical and the porting plan.
Neither bump is mechanical. What that run has to settle, read from the skill's scripts on 2026-11-08:

- **Zig 0.16 -> 0.17**: both tags have `minimum_zig_version = "0.17.0"`. Every `LIBGHOSTTY_VT_ZIG_*` pin and sum
  moves, and the move reaches every build that links the library: desktop on Linux, macOS and Windows, the session
  server, and the Android and iOS libraries (`setup-android.sh`, `setup-ios.sh`). Recheck the Linux glibc-target
  workaround for gcc 16's `.sframe` relocations (`build/patches/ghostty-vt/README.md`) on 0.17's linker.
- **C API**: `classify-diff.py` reports `API_BREAK=used` for both. At v1.4.0 it is 4 declarations, of which only one
  breaks: `ghostty_terminal_vt_write_until_ground` is gone, replaced by `ghostty_terminal_vt_write_until`
  (`crates/ngterm-ghostty-vt/src/terminal.rs:400`, with callers in `ngterm-term` `pane.rs` and `actions.rs` and the
  wrapper's `tests/link.rs`). At vt-1.0.0 it is 60: the 1.0 ABI cleanups renumber option enums behind a new
  `*_INVALID = 0`, remove the `setopt`/`setopt_from_terminal` functions of the key and mouse encoders, make the
  event setters return `GhosttyResult`, retype the buffer-filling functions to `uint8_t*`/`out_written`, take key
  UTF-8 as a `GhosttyString`, replace the cursor-style enums with one shared enum and change the formatter's
  terminal options. It is mostly `ngterm-ghostty-vt`, the wrapper.
- **Patch 0001** (kitty placement namespace and parent) conflicts at both tags: `kitty_graphics.h` is reworked
  (v1.4.0 adds render-state image iteration and resolves Kitty placeholders in the render state; vt-1.0.0 rewrites
  the header). Neither tag exposes the placement key's namespace or a relative placement's parent, so it is a hand
  rebase. Also check whether the new render-state image API can replace ngterm's Rust resolution of placeholders and
  chains (`ngterm-term`, `ngterm-gpui`), which would make it an API adoption that drops the patch.
- **Patch 0004** (formatter writes skipped cells unstyled) may become removable: `152bdbc6` ("formatter: restore
  blank cells and erased backgrounds in VT output", in v1.4.0) steps over a gap with CUF instead of writing styled
  spaces, which is the case 0004 fixes. upstream agterm's
  `docs/backlog/ghostty-pin-bump-past-vt-1-0-breaks-swift-nullability.md` reaches the same conclusion for its zmx
  copy of the patch. The patch still applies with an offset, so drop it only if its README verification
  (`cargo test -p ngterm-ghostty-vt --test patches skipped_cells`) passes without it, and say whether the
  formatter's carried-over wrapped blanks (still spaces, though no longer in the previous pen) matter to the
  session snapshot.
- **Patch 0003** (cursor hyperlink across page replacement) applies with an offset. `cursorChangePin` is not fixed
  at either tag, so it stays: rebase it and re-run its verification (the SIGSEGV test and both Zig filters).
  Patch 0002 applies clean at v1.4.0 and with an offset at vt-1.0.0.
- **Terminal fixes gained**, all of them already in v1.4.0: #14761 (CUB and BS stop at the left margin), #14762
  (CHA, VPA, HPR and VPR stay origin-relative), #14814 (continuations restart at C1 introducers), #14812 (cancelled
  or aborted APC commands are discarded), #14832 (Kitty placements in erased history are removed), #14856 (rows from
  `Capacity.adjust` are capped), #14858 (tmux layouts reject zero-sized and oversized panes).

The pin rule in `build/pins.env` (it follows ghostty main, with green libghostty-vt CI) favors vt-1.0.0, the first
release with a stable C ABI promise for `include/ghostty/vt*.h`. v1.4.0 is the cheaper step: the same fixes, one
rename and patch 0001's rebase. The `libghostty-sync` run picks the target, and that commit deletes this item.
