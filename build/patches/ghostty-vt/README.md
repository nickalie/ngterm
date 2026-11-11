# libghostty-vt patches

`scripts/build-ghostty-vt.sh` applies every `*.patch` here to the pinned Ghostty source (`LIBGHOSTTY_VT_REV` in
`build/pins.env`), in name order, with `patch -p1 --fuzz=0`; the patches and their sha256s key the prefix and land in
its stamp, so a changed patch prepares a new tree and rebuilds the library. Each is a unified diff against that
pristine tree (`a/` and `b/` labels), its header saying what it changes and where it comes from. A patch that changes a
header is followed by `scripts/gen-ghostty-vt-bindings.sh` (its `--check` holds).

The carried patches change library logic for fixes or parity behavior Rust cannot reach from outside (0002 to
0004). Each records its reason, upstream status and remove-when verification, in the form of herdr's
`vendor/libghostty-vt.patches.md`. A gap Rust can fill is filled in Rust, not here.

Patch 0001 (placement-id namespaces and parent links) was retired at v1.4.0: the official render-state image API
supersedes its raw getters, which were never upstreamed. The wrapper and renderer now draw only that API's resolved
placements, including placeholders and relative chains.

Remove a patch only when the pinned source has the behavior and the patch's verification still passes without it. A
pin bump rebases every patch onto the new revision (with no fuzz) and re-runs every verification below.

The verifications use the prepared tree, the pinned zig and the vendored Zig global cache its stamp names (Zig 0.17
takes that cache from the environment only, and without it a build fetches into the user's own):

```sh
tree="$(scripts/build-ghostty-vt.sh)/src"
zig="$(sed -n 's/^zig=//p' "$tree/../share/ngterm/ghostty-vt-STAMP")"
export ZIG_GLOBAL_CACHE_DIR="$(sed -n 's/^zig-global-cache=//p' "$tree/../share/ngterm/ghostty-vt-STAMP")"
```

Ghostty's own tests run there with a scratch cache, so the tree stays as the build script reads it:
`(cd "$tree" && "$zig" build test-lib-vt --cache-dir "$(mktemp -d)" -Dtest-filter='<name>')`. On Linux add the
build's glibc target, `-Dtarget=x86_64-linux-gnu.2.39` (or `aarch64-…`), as the library build does. Zig 0.16's
linker refused the distro's `crt1.o` (its `.sframe` relocations, `R_X86_64_PC64`, gcc 16) on the native target; Zig
0.17's links it (a native `-Dtest-filter='placement_get'` run passed against gcc 16.2's `crt1.o` at this pin), but the glibc
target stays, as the library's floor. The test filters below were run that way on the pin (`cursorAbsolute`: 38 of 38, herdr's two among them).

## 0002 hosted Wuffs builds

status: active (the allocator-stub half only)

patch: `build/patches/ghostty-vt/0002-hosted-wuffs-builds.patch`

origin: herdr's `vendor/patches/libghostty-vt/0004-fix-hosted-wuffs-builds.patch` (`HERDR_REV` 143bf5f,
Apache-2.0), modified: rebased onto the pin, its guard spelled `builtin.target.os.tag` (Zig 0.17, Ghostty #14519),
and its `pkg/wuffs/build.zig` half no longer carried (below)

herdr issue: none; it keeps non-SIMD hosted builds working

upstream discussion: not opened

upstream pr: not opened

base: `LIBGHOSTTY_VT_REV` `83fb95624b0439ee660c2679b43b38c8b63769cd` (v1.4.0; herdr's: `44f2a44d`), still unpatched

files:

- `pkg/wuffs/src/main.zig`

reason: The no-libc Wuffs module exports hidden weak `calloc`/`free` stubs; on a hosted target built with SIMD off
(libc then unlinked), the stubs in the static archive override the embedding executable's libc allocator, and every
zeroed allocation fails. The stubs are now exported for freestanding targets only. The SIMD builds ngterm makes are
unchanged.

The build `ngterm-ghostty-vt-sys` makes on Linux cannot observe this patch: it has SIMD on, which links libc
(`src/build/GhosttyZig.zig`), so the unpatched source exports no stubs either. Neither the wrapper's tests nor an `nm`
of that archive fail without the patch. Its check builds what it changes: a hosted build of the prepared tree with
SIMD off. Run at this pin, the patched tree's archive defines neither symbol (it only references them); at the
previous pin, with the hunk reverted, it defined both as weak (`W calloc`, `W free`).

not carried: herdr's other half, `pkg/wuffs/build.zig` forwarding the build's `--libc` file to Wuffs' translate-c step
(a `windows-msvc` build that names its SDK with `--libc` needs it), is a recorded scope reduction at this pin, not an
upstream fix: Zig 0.17 runs `build.zig` in a configurer process that never sees `--libc` (`std.Build` has no
`libc_file`; the maker hands it to compile steps only at make time), so the hunk has no form, and ngterm never passes
a `--libc` file (its builds find the SDK themselves). Should an explicit-SDK build be needed, restore it as an
explicit `-D` option threaded to both Wuffs dependency sites (`src/build/GhosttyZig.zig`, `src/build/SharedDeps.zig`)
and given beside `--libc`; reading `ZIG_LIBC` instead would be a different input, not this forwarding.

remove when: the pinned source exports no allocator stubs on hosted targets, and the check below passes without this
patch.

verification:

```sh
# a hosted build with SIMD off defines no calloc or free (only undefined references)
out="$(mktemp -d)" version="$(sed -n 's/^version-string=//p' "$tree/../share/ngterm/ghostty-vt-STAMP")"
(cd "$tree" && "$zig" build -Demit-lib-vt=true -Doptimize=fast -Dsimd=false -Dcpu=baseline \
  -Dtarget=x86_64-linux-gnu.2.39 -Dversion-string="$version" --prefix "$out/prefix" --cache-dir "$out/cache")
nm "$out/prefix/lib/libghostty-vt.a" | grep -E ' [TtWwVv] (calloc|free)$'   # prints nothing
```

The wrapper's `cargo test -p ngterm-ghostty-vt --test patches wuffs` checks only that the Wuffs build the patch
touches still swizzles in ngterm's build; it passes without the patch.

## 0003 keep the cursor hyperlink across page replacement

status: active

patch: `build/patches/ghostty-vt/0003-cursor-hyperlink-page-replace.patch`

origin: herdr's `vendor/patches/libghostty-vt/0008-cursor-hyperlink-page-replace.patch` (`HERDR_REV` 143bf5f,
Apache-2.0), modified: rebased onto the pin (paths and line numbers; the code and its two Screen tests are herdr's)

herdr issue: https://github.com/herdrdev/herdr/issues/5296

upstream discussion: not opened

upstream pr: not opened

base: `LIBGHOSTTY_VT_REV` `83fb95624b0439ee660c2679b43b38c8b63769cd` (v1.4.0; herdr's: `44f2a44d`), still unpatched
(`cursorChangePin`). v1.4.0's #14857 keeps the cursor's style and link across a `scrollClear` prune, a different path:
it does not reach the page replacement or split inside `cursorChangePin` this patch fixes.

files:

- `src/terminal/Screen.zig`

reason: A crash. Moving the cursor onto another page while an OSC 8 link and a non-default style are open can grow
that page during style migration. The growth replaces the page, re-adds the cursor's heap link and frees it; the move
then clears the pointer without clearing the id the replacement installed, so the link's end dereferences null and the
process dies. In the session server one libghostty-vt crash ends every live session, so the fix is carried with the
server's VT and the app's alike. The same function also crashes when style migration hits the maximum style capacity:
the page split moves the cursor again, and the nested pin change releases link id 0 from a page that no longer holds
it; the patch skips that release.

remove when: the pinned source keeps the cursor hyperlink across a page replacement or page split during a pin
change, and the checks below pass without this patch.

verification:

```sh
cargo test -p ngterm-ghostty-vt --test patches hyperlink   # SIGSEGV without the patch
(cd "$tree" && "$zig" build test-lib-vt --cache-dir "$(mktemp -d)" \
  -Dtest-filter='cursorAbsolute across a full page keeps the hyperlink')
(cd "$tree" && "$zig" build test-lib-vt --cache-dir "$(mktemp -d)" \
  -Dtest-filter='cursorAbsolute keeps the hyperlink when style migration splits the page')
```

## 0004 the HTML formatter writes skipped cells unstyled

status: active (HTML only)

patch: `build/patches/ghostty-vt/0004-formatter-unstyled-blanks.patch`

origin: upstream agterm v0.36.0's `scripts/zmx-patches/ghostty/0001-formatter-unstyled-blanks.patch` (MIT), the patch
upstream applies to the Ghostty its zmx links; rebased onto the pin (line numbers) and narrowed to HTML output (its
VT half is upstream now, below).

agterm issue: none; parity with upstream's re-attach snapshot

upstream discussion: not opened

upstream pr: not opened

base: `LIBGHOSTTY_VT_REV` `83fb95624b0439ee660c2679b43b38c8b63769cd` (v1.4.0): VT upstreamed in #14725, HTML still
needs it

files:

- `src/terminal/formatter.zig`

reason: The formatter wrote cells a program skipped with a cursor move as spaces under the previous cell's style, so a
gap after a colored run came back painted. Since #14725 the pinned VT formatter steps over such gaps with CUF (they
replay unpainted) and restores background-only cells with ECH, so the session server's snapshot needs nothing here
and a style close before those cells would undo the ECH restore: the patch leaves VT output alone. The HTML formatter
still buffers blanks under the open style (`formatter.zig`, the blank-cell branch), and no option controls it, so in
HTML a cell with no text closes the open style before it is buffered as a blank; cells the program wrote, spaces
included, keep their style, and plain output is unchanged. ngterm's own `copy_to_clipboard` and `write_*_file` in HTML
take it: their gap reads as the screen shows it.

remove when: the pinned HTML formatter writes skipped cells unstyled (or an option asks for it), and the checks below
pass without this patch.

verification:

```sh
cargo test -p ngterm-ghostty-vt --test patches skipped_cells
```

## herdr patches not carried

Reviewed against the pin and against what ngterm uses (herdr's `vendor/libghostty-vt.patches.md` at `HERDR_REV`):

- **0002 expose modifyOtherKeys mode through terminal data**: herdr runs inside another terminal and must ask that
  outer terminal for printable key releases while an inner program has modifyOtherKeys 2 on. ngterm is the outermost
  terminal: its key encoder reads the mode from the terminal itself (`ghostty_key_encoder_setopt_from_terminal`), so
  nothing needs the scalar.
- **0005 bounded word selection for wrapped link activation**: herdr's Ctrl+click resolves a wrapped token beyond the
  viewport through a new selection API. ngterm finds links in Rust with Ghostty's own matcher over the frame's rows
  (`ngterm-gpui` `terminal/links.rs`), as Ghostty does, and needs no library API for it.
- **0006 clear screen while preserving the cursor line**: a new C function for herdr's own "clear pane" feature.
  ngterm's `clear_screen` follows Ghostty's `Termio.clearScreen` and is written in Rust as VT sequences at a ground
  point (`ngterm-term` `actions.rs`).
- **0007 experimental encoded PNG and immutable source retention**: a default-off experiment for forwarding Kitty PNG
  uploads through a multiplexer without decoding. ngterm decodes and draws images itself (the frontend's PNG decoder
  through `sys.h`), and the session server's VT runs with image media off (plan Task 16), so nothing would retain or
  forward the encoded bytes.
