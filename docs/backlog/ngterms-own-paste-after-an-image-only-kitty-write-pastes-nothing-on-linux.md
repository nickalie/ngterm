---
worth: yes
where: vendor/gpui-pre-linux/src/linux/x11/client.rs:1912
added: 2026-11-11
---
# ngterm's own paste after an image-only Kitty write pastes nothing on Linux

A verified, unfixed defect introduced by Task 15 of `docs/plans/20261111-ghostty-sync-83fb956.md` (Kitty clipboard:
desktop clipboards). The task's review budget ran out with this MAJOR finding still open, so it is recorded here
rather than fixed.

After a Kitty clipboard write that holds only an image (a valid `image/png` representation, no text), ngterm's own
ordinary clipboard read returns nothing on Linux, on both X11 and Wayland:

- X11: `write_clipboard_representations` (`vendor/gpui-pre-linux/src/linux/x11/client.rs`, around line 1912) builds the
  cached `clipboard_item` only from `text/plain`, so an image-only write sets it to `None`. While ngterm owns
  CLIPBOARD, `read_from_clipboard` answers from that cache and never reads the stored image.
- Wayland: `Clipboard::set_representations` (`vendor/gpui-pre-linux/src/linux/wayland/clipboard.rs`) builds its
  contents only from `text/plain`, and `read` returns those contents for ngterm's own offer.

Both live in `build/patches/gpui-pre-linux/0007-clipboard-representations.patch`. Other applications and Kitty reads
still get the image; only ngterm's own paste loses it.

Repro: in a pane, make an image-only `image/png` Kitty clipboard write so ngterm owns CLIPBOARD, then press Ctrl+V in a
remote-host pane. Nothing is pasted: `terminal.rs::run_effect` gets no `ClipboardItem`, so the remote image-paste path
(`image_paste::start`) never runs.

Suggested fix: when a write has no text, fill the cached `ClipboardItem` with the supported image representations; or
answer ordinary reads from the owned representations on both Linux backends. Carry the fix into patch 0007 through
`scripts/vendor-gpui.sh`.

Verify: a test that an image-only write followed by an ordinary clipboard read returns the image item, on X11 under
xvfb and on Wayland (the backend's offer/serve unit test).
