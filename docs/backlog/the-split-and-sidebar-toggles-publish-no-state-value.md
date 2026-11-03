---
worth: yes
where: crates/ngterm-gpui/src/titlebar.rs:853
added: 2026-11-03
---
# The split and sidebar footer toggles publish no state value, so no build can claim `ui-string-values`

Upstream's `split-toggle` carries its split state as its accessibility value (`none`, `both`, a shown side) and the
sidebar footer's `focus-filter-toggle` and `flagged-view-toggle` carry `on`/`off`. ngterm's GPUI frontend publishes
such a value on the overlays button (`overlay-toggle`), the session icon's tint and a companion row, but not on these
three buttons. UIA (Windows) and AX (macOS) would read the value through ValuePattern/AXValue, so the only thing
standing between those builds and the `ui-string-values` capability is the app: with the values published, Windows
can claim it (`CAPABILITIES` in `crates/ngterm-gpui/src/app.rs`, `#[cfg(windows)]`) and the value-reading branches of
`tests/ui/test_titlebar.py`, `tests/ui/test_sidebar_rows.py` (and `test_tinted_icon_names_its_tint`) run there.

Found by Task 7 of `docs/plans/20261031-e2e-ui-and-coverage.md`: the Windows ui layer (drivers/uia.py) runs green on
the branch without the capability, and its inventories show `overlay-toggle value='none'` but `split-toggle` and the
footer toggles without a value. Fix: `aria_value` with the state name on those buttons (each with its Rust test on
the element's node), claim the capability on Windows, run the ui layer on the Windows runner. Linux stays unclaimed:
AT-SPI carries no string value on a button.
