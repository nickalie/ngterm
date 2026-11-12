---
worth: yes
where: build/pins.env:23
added: 2026-11-12
---
# the libghostty-vt pin trails vt-1.0.0

`LIBGHOSTTY_VT_REV` is Ghostty's `v1.4.0` (`83fb9562`, 2026-10-27), moved there by
`docs/plans/20261111-ghostty-sync-83fb956.md`. Ghostty has since tagged `vt-1.0.0` (`0fa3d577`, 2026-11-06,
"libghostty-vt: prepare for 1.0.0"), 266 commits ahead with `v1.4.0` its ancestor, the first release with a stable C
ABI promise for `include/ghostty/vt*.h`. Its `minimum_zig_version` is still 0.17.0, so the Zig pins stay.

Its 1.0 ABI cleanup is what the move costs (37 files under `include/` change): option enums renumbered behind a new
`*_INVALID = 0`; the key and mouse encoders' `setopt`/`setopt_from_terminal` removed; event setters returning
`GhosttyResult`; buffer-filling functions retyped to `uint8_t*`/`out_written`; key UTF-8 taken as a `GhosttyString`;
the cursor-style enums replaced by one shared enum; the formatter's terminal options changed. It is mostly
`ngterm-ghostty-vt`, the wrapper, plus the regenerated bindings and whatever the carried patches (0002 to 0005,
`build/patches/ghostty-vt/README.md`) need to rebase onto the reworked headers.

Run it as its own change with `/libghostty-sync vt-1.0.0`, which does the analysis, the bump and the porting plan;
that run's commit deletes this item.
