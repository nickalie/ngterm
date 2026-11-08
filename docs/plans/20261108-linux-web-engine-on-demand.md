# Linux web engine: the system's where a package manager has it, a download on demand elsewhere

## Overview

- The v0.1.1 Linux packages are 195–223 MB each against 19–35 MB on macOS and Windows: they bundle a source-built
  WPE WebKit, its libraries and every GStreamer plugin, while macOS and Windows use the OS's web engine. The app
  binaries themselves are about the same size on every desktop OS.
- After this plan the Linux packages carry no web engine (user, 2026-11-08):
  - where the distro's package manager can install WPE WebKit at the pinned API version or newer, the package asks for
    it and the app uses the system's engine;
  - everywhere else (Ubuntu 24.04, the deb/rpm baseline; the tarball; the AppImage) the app downloads a verified,
    trimmed engine runtime the first time a page needs it and keeps it per user, shared by later app versions until
    the runtime itself changes.
- Expected sizes (measured on the v0.1.1 x86_64 tree, not yet built): about 30–40 MB per package and per app update;
  the engine runtime about 110 MB once (zstd), down from today's 551 MB unpacked engine-and-app tree.
- macOS and Windows are untouched: they already get their engine from the OS (WKWebView, WebView2).

## Context (from discovery)

- Bundle breakdown of `ngterm-0.1.1-linux-x86_64.tar.gz` (551 MB unpacked, 1,964 files): app binaries 92 MB;
  `libWPEWebKit` and its helpers 120 MB; WebKit's own libraries 114 MB (ICU 35 MB, flite voices ~27 MB, the AVIF
  encoders Ubuntu's libavif links ~15 MB); GStreamer libraries and what its plugins load 166 MB (libav alone 80 MB:
  Ubuntu's FFmpeg with every encoder); all 257 plugins 24 MB; a second copy of every plugin in `usr/lib` 23 MB, made
  by linuxdeploy's `--library`.
- `scripts/package-release.sh` (Linux branch): copies the release WPE prefix, every `.so` under the runner's
  `gstreamer-1.0` and `gio/modules`, runs linuxdeploy over all of it, sets `$ORIGIN` RUNPATHs on every ELF, checks
  that nothing resolves to an undeclared host library (`packaging/linux/runtime-libraries.txt`), smoke-tests the
  packaged engine, then builds the tarball, deb, rpm, Arch package (nfpm, `packaging/linux/nfpm.yaml`) and the
  AppImage from one tree.
- `scripts/build-release-wpe.sh`: WPE from source on Ubuntu 24.04 with `packaging/release/relocatable-wpe.patch`,
  cached by a key over its inputs (no architecture in it); `bwrap` and `xdg-dbus-proxy` are compiled in as the host's
  `/usr/bin` ones. `.github/workflows/release.yml` job `linux` (x86_64, aarch64) builds it, builds the binaries
  against it in one `cargo build -p …` and packages.
- `relocatable-wpe.patch` (read, settles what the runtime layout needs): helpers resolve as
  `dladdr(libWPEWebKit)/wpe-webkit-2.0/<process>`; the sandbox binds the prefix two levels above the helper dir
  read-only; the network process falls back to the host's CA bundle. So any directory with the library in `lib/` and
  the helpers in `lib/wpe-webkit-2.0/` works. The injected bundle comes from `WEBKIT_INJECTED_BUNDLE_PATH`.
- `crates/ngterm-wpe`: the client decides the launch (`launcher::provider()` from the build-time `NGTERM_WPE_LIBDIR`,
  the vendored prefix under `bwrap --ro-overlay`) and runs the app's own image as the worker (`own_image`,
  `worker/client.rs`, opened from `/proc/self/exe`; `serve_if_asked` in `crates/ngterm-gpui/src/main.rs`), so
  `ngterm` links `libWPEWebKit`. The client side also uses WebKit in `input.rs` (`platform_sys` modifier bits),
  `build.rs` (fails without `pkg-config wpe-webkit-2.0`), `host.rs` (the in-process backend). The worker handshake
  compares only `PROTOCOL` (`worker/message.rs`). A marked release bundle's worker gets its GIO, GStreamer and
  injected-bundle paths from `bundle_environment` (`launcher.rs`).
- Builds of the frontend that name only `-p ngterm-gpui`: `release.yml`, `.github/workflows/linux.yml`,
  `conformance/tools/run-linux.sh`, `scripts/lib/dev-instance.sh`; `scripts/lib/native.sh` keeps
  `WORKSPACE_WITHOUT_WPE`. Worker-backed tests: `crates/ngterm-gpui/tests/html_overlay/linux.rs`,
  `crates/ngterm-wpe/tests/headless/main.rs` (`WPE_TEST_BACKEND=worker`), the release smoke binary.
- The workspace uses `resolver = "3"`: features unify across one cargo invocation, so only a crate boundary keeps
  WebKit out of `ngterm`.
- `crates/ngterm-gpui/src/html_overlay/registry.rs`: `EngineState` (`Starting`, `Failed`, running) and the pages
  waiting for the engine.
- `crates/ngterm-update`: the update check reads `releases/latest` and rejects a tag that is not `v<semver>`
  (`download.rs`); its HTTP agent has a 15-minute global timeout; `install.rs` `validate()` requires the four app
  binaries in a downloaded tree. No zstd dependency in the workspace.
- `packaging/linux/apparmor/ngterm-release` grants `userns` to `/opt/ngterm/usr/bin/ngterm` with
  `flags=(unconfined)`: a program it execs that has no profile of its own stays in it (the profile's own comment, for
  `bwrap`). `ngterm-web`, started by `ngterm`, inherits it the same way.
- In flight elsewhere: `docs/plans/20261105-release-signing.md` also edits `release.yml`; whichever lands second
  rebases onto the other.

## Development Approach

- **testing approach**: Regular (code first, then tests in the same task), as the repo's other plans.
- Complete each task, with its tests green, before the next; keep this file in sync with the work.
- Every task leaves main releasable: until Task 7 the packages still bundle the engine, now driven through
  `ngterm-web`.
- Checks are sized to the change (AGENTS.md > Choosing checks): scoped clippy and tests locally; the packaging and
  release changes prove themselves through `scripts/lint.sh`, the pytest selftests and a release dry run.
- Keep the dev paths (system WPE on Arch, the vendored prefix under `bwrap`) working throughout.
- Media features stay as they are (parity): libav, speech synthesis and AVIF decoding remain in the engine runtime.

## Testing Strategy

- **unit tests**: the worker launch and build identity, the source resolution, the worker environment, the runtime
  cache (verify, unpack, lock, cleanup), the frontend's engine states.
- **script tests** (`conformance/selftests`, pytest, as `test_fetch_lavapipe.py` tests its script): the runtime's
  plugin allowlist, closure and key; the packaging's `NEEDED` and RUNPATH checks on fixture ELFs;
  `release-artifacts.py`'s runtime reuse and release flags.
- **release smoke**: the packaged `ngterm-web` against the extracted runtime with the build prefix hidden, running the
  audio cases of `crates/ngterm-gpui/tests/html_overlay/cases.rs` and a video case.
- **real systems**: a fresh Ubuntu 24.04 (download path) and Arch (system path), Task 9, against a local HTTP server
  through the URL override.

## Test machines

Four desktop Linux VMs on the host, created for this plan (2026-11-08), check the packages on real distros. Each has a
GNOME desktop, the operator's user with sudo and the host's ssh key; none starts on its own.

| placeholder | distro | expected engine path (Task 1 confirms) |
|---|---|---|
| `<ubuntu-ssh>` | Ubuntu 26.10 desktop | deb: system WPE if 26.10 ships 2.54+, else download |
| `<debian-ssh>` | Debian 13 (trixie), GNOME | deb: download |
| `<fedora-ssh>` | Fedora 45 Workstation | rpm: system WPE if Fedora ships 2.54+, else download |
| `<opensuse-ssh>` | openSUSE Leap 16.1, GNOME | rpm: download |

- The values (ssh aliases, VM names, state, start, stop and snapshot commands, the password variable) are in the
  git-ignored `.claude/skills/agterm-upstream-sync/references/machines.local.md`, section "Linux distro VMs". Never
  copy them into a tracked file.
- Use them as `.claude/skills/agterm-upstream-sync/references/machines.md` says for the Windows VM: check the state
  first, start on demand, one VM at a time (the host's CPU cap), stop what you started when the step ends, failures
  included.
- They are throwaway: a GUI run on their desktops is fine there. Before the first install, take a snapshot of each
  with no ngterm on it, and revert to it before every fresh-install check.
- Ubuntu 24.04, the deb's baseline, has no VM: its download path is checked in a container (Task 9).

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Update the plan if implementation deviates from the original scope.

## Solution Overview

- **The engine in its own crate and binary.** `ngterm-wpe` keeps the engine (the `-sys` crates, `engine/`, `serve`,
  its tests) and gains the `ngterm-web` worker binary. A new crate, `ngterm-wpe-client`, holds what the app needs: the
  `Host` over the worker socket, the wire messages, config, events and input types, with no `-sys` crate and no
  pkg-config build script. `ngterm` depends on the client only; a packaging check keeps WebKit, WPEPlatform,
  GStreamer, libsoup and JavaScriptCore out of its `NEEDED`.
- **`ngterm-web` owns its launch.** The client always runs the sibling binary. `ngterm-web` knows the libdir it was
  built against: for the dev box's vendored prefix it re-executes itself under the `bwrap` overlay, as the client does
  today; for the system engine or a runtime it runs as is. It drops `LD_LIBRARY_PATH` from its environment first
  thing (ld.so has already cached it for its own loads), so WebKit's helpers and the host's `bwrap` and
  `xdg-dbus-proxy` never inherit it.
- **One build.** `ngterm` opens `ngterm-web` as a descriptor in `main`, at process start, so an update that replaces
  `/opt/ngterm` later does not swap the worker. The worker's `Hello` carries a build identity (version and commit);
  a mismatch fails with today's "replaced while ngterm ran; restart" message.
- **Where the engine comes from**, resolved by the app (`ngterm-gpui` with `ngterm-update`), in order, and handed to
  the client as a source (`System | Runtime(dir) | NeedsDownload`):
  1. the system's: `ngterm-web --probe` runs with no extra environment and reports a WebKit version at or above the
     pin;
  2. the downloaded runtime in `<data dir>/web-engine/<key>/` (`ngterm_core::paths`, so dev instances keep theirs
     private), complete;
  3. otherwise the runtime is downloaded, verified against the size and hash compiled into the app, unpacked, and the
     waiting pages go on loading.
- **What the packages ask for**: the Arch package depends on `wpewebkit` (Arch always has a current one). The deb and
  rpm *recommend* the versioned WPE library package only (its own recommendations bring the GStreamer plugins); apt
  and dnf skip a recommendation they cannot satisfy, so the same package installs on Ubuntu 24.04 (and downloads) and
  on newer distros (and uses the system's). The host libraries the runtime needs stay hard dependencies. The tarball
  and AppImage ask for nothing and download.
- **The engine runtime** is one archive per architecture: the release WPE prefix, the GStreamer plugins WPE 2.54
  creates elements from, their library closure once (no duplicates), GIO modules, the plugin scanner. Its key hashes
  everything that defines it, so it changes with the engine, its packaging scripts, or a security update of a copied
  Ubuntu package (by design), not with every app release.

## Technical Details

- **Runtime archive**: `ngterm-web-engine-<key>-linux-<arch>.tar.zst` (zstd level 19, `--long=27`), layout `lib/`,
  `lib/gstreamer-1.0/`, `lib/gio/modules/`, `lib/wpe-webkit-2.0/`, `libexec/gstreamer-1.0/gst-plugin-scanner`,
  `share/doc/` (licenses), `MANIFEST` (key, arch, WPE version). Every ELF in it gets an `$ORIGIN` RUNPATH into the
  runtime's `lib/`.
- **Key**: the WPE build key, the architecture, the plugin allowlist, `package-web-engine.sh`,
  `web-engine-closure.py`, and the versions of the copied Ubuntu packages.
- **Hosting**: a GitHub release `web-engine-<key>` per key, created **as a prerelease with `--latest=false`**, so
  `releases/latest` (the update check) keeps naming the app release. The release workflow looks for this arch's asset
  by `--print-key` before building the runtime and uploads it only when missing; the engine release is published
  before the app release leaves draft. A `web-engine-*` release that a shipped version pins is never deleted (release
  docs). The runtime's SHA-256 is also listed in the app release's `SHA256SUMS` for people checking by hand; the app
  trusts only its compiled-in pins.
- **Build-time pins** (`ngterm-update`, from `build.rs` env): the runtime's URL, size, SHA-256 and key. Builds without
  them (dev, distro rebuilds) never download. `NGTERM_WEB_ENGINE_URL` overrides the URL only (mirrors, Task 9's local
  server); size and hash stay pinned.
- **Download and unpack**: an HTTP agent without the update check's global timeout, with resume by `Range`; the body
  is capped at the pinned size; SHA-256 checked before unpacking; the `zstd` crate with `window_log_max` 27 and the
  workspace's `tar`; only regular files, directories and relative symlinks that stay inside the tree are accepted;
  setuid and setgid bits are cleared; unpacked into `<key>.partial/`, renamed to `<key>/` when complete. A lock file
  serializes concurrent installers. `ENOSPC` reports the space needed.
- **In use and cleanup**: each worker holds a shared `flock` on `<key>/.in-use` while it runs; cleanup removes another
  key's dir only when it can take that lock exclusively, so older installs and running instances keep theirs.
- **Worker environment for a runtime** (applied to the worker's command only; the app, its panes and session servers
  never see it): `LD_LIBRARY_PATH` (dropped by `ngterm-web` at start), `GIO_MODULE_DIR` (replaces glib's default dir,
  so a newer host's modules never load into the runtime's glib), `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_PLUGIN_SCANNER`,
  `GST_REGISTRY_1_0` (a registry file per runtime), `GST_PLUGIN_PATH` and `GST_PLUGIN_PATH_1_0` removed,
  `WEBKIT_INJECTED_BUNDLE_PATH`. For the system engine: no extra environment.
- **Probe**: `ngterm-web --probe` prints the loaded WebKit version and exits; a loader failure (no
  `libWPEWebKit-2.0.so.1`) or a version below the pin means no usable system engine. `ngterm-web` has no RUNPATH
  into the app's `usr/lib`, so on the system path it loads only system libraries.
- **Offline**: `ngterm --install-web-engine <archive>` installs a runtime from a file through the same checks.
- **Frontend states**: `EngineState` gains `Downloading { received, total }` and a download failure with retry; a
  waiting page shows progress in its own area, not a modal.
- **AppArmor**: no new profile entry; `ngterm-web` and the `bwrap` it starts inherit `ngterm-release` from `ngterm`.
  The app's sandbox probe (`html_overlay/sandbox.rs`) therefore keeps reflecting the worker's situation.

## What Goes Where

- **Implementation Steps**: code, scripts, packaging, workflow and docs in this repo.
- **Post-Completion**: runs on real installs and the first published runtime.

## Implementation Steps

### Task 1: Survey distro WPE packages and ABI stability

**Files:**
- Modify: `docs/plans/20261108-linux-web-engine-on-demand.md`

- [ ] list, for Debian (stable, testing), Ubuntu (24.04, the latest release), Fedora (current two) and openSUSE
      Tumbleweed: the package that ships `libWPEWebKit-2.0.so.1`, its version, whether it has the WPEPlatform API,
      and whether it recommends the GStreamer plugins playback needs; for the four test machines' distros, check it
      there too (`apt-cache policy`, `dnf info`, `zypper info`)
- [ ] confirm from WebKit's API/ABI policy that a binary built against the 2.54 headers runs on later 2.x releases
      with the same soname (the probe's lower bound only), and note any WPEPlatform API marked unstable
- [ ] derive the plugin allowlist from the elements WPE 2.54 creates (`makeGStreamerElement`,
      `gst_element_factory_make` in its sources), mapped to plugin files; check at least `opengl`, `encoding`, `gio`,
      `alsa` and `libav` against it
- [ ] record the results in Technical Details (package names for Task 7, the allowlist for Task 4)

### Task 2: The engine worker as its own binary

**Files:**
- Create: `crates/ngterm-wpe-client/` (client `Host`, wire messages, config, events, input types)
- Create: `crates/ngterm-wpe/src/bin/ngterm-web.rs`
- Modify: `crates/ngterm-wpe/` (`lib.rs`, `worker.rs`, `worker/*`, `launcher.rs`, `input.rs`, `host.rs`, tests),
  `Cargo.toml` (workspace members), `crates/ngterm-gpui/Cargo.toml`, `crates/ngterm-gpui/src/main.rs`,
  `crates/ngterm-gpui/src/html_overlay/engine_tests.rs`, `crates/ngterm-gpui/tests/html_overlay/linux.rs`
- Modify: `crates/ngterm-update/src/install.rs`, `scripts/stage.sh`, `scripts/package-release.sh`,
  `scripts/lib/native.sh`, `scripts/lib/dev-instance.sh`, `conformance/tools/run-linux.sh`,
  `.github/workflows/release.yml`, `.github/workflows/linux.yml`, `AGENTS.md` (crate table)

- [ ] move the client side into `ngterm-wpe-client` with no `-sys` dependency and no pkg-config build script; the
      modifier bits become client constants, with an engine-side test that they equal the `platform_sys` ones
- [ ] add the `ngterm-web` binary that serves the engine; it re-executes itself under the `bwrap` overlay for a
      vendored libdir (reusing `overlay_args`) and drops `LD_LIBRARY_PATH` from its environment at start
- [ ] `ngterm` depends on the client only and opens the sibling `ngterm-web` as a descriptor in `main`; `Hello` carries
      a build identity, a mismatch fails with the "replaced while ngterm ran; restart" message; the client takes a
      worker path override for test harnesses
- [ ] every build of the frontend also builds `ngterm-web` (release, linux CI, run-linux, dev-instance, native.sh);
      `stage.sh` stages it; `package-release.sh` bundles it with the engine as today (RUNPATH into the bundle until
      Task 7) and points the smoke test at it; `validate()` requires `ngterm-web` in a Linux tree
- [ ] write tests: build identity match and mismatch; the descriptor survives replacing the file; the vendored
      re-exec builds the overlay command; the worker path override; `validate()` without `ngterm-web`; the modifier
      bits
- [ ] run the `ngterm-wpe` worker and headless tests, the `ngterm-gpui` HTML overlay tests (headless) and
      `cargo test -p ngterm-update` - must pass before Task 3

### Task 3: Engine source and the worker's environment

**Files:**
- Modify: `crates/ngterm-wpe-client/` (launch from a source), `crates/ngterm-wpe/src/bin/ngterm-web.rs` (`--probe`),
  `crates/ngterm-gpui/src/html_overlay/registry.rs` (resolution), `crates/ngterm-update/src/lib.rs` (the pins, the
  runtime dir under `ngterm_core::paths`)

- [ ] `ngterm-web --probe`: print the loaded WebKit version, exit; loader failure and old versions map to "absent"
- [ ] `Host::start_worker(config, source)` with `source` = `System | Runtime(dir)`; the client turns it into the
      command and the environment from Technical Details, on the worker's command only
- [ ] the app resolves the source in order: system probe, complete runtime dir for the pinned key, else
      `NeedsDownload` (handled in Tasks 5–6; until then the bundled engine keeps the `bundled-wpe` marker path, which
      Task 7 removes)
- [ ] write tests: resolution order with a fake probe and temp runtime dirs; a `.partial` dir is not used; the
      runtime environment (`GIO_MODULE_DIR`, `GST_*` set and removed) reaches only the worker command; a web process
      started by the worker carries no `LD_LIBRARY_PATH`
- [ ] run tests - must pass before Task 4

### Task 4: The engine runtime archive

**Files:**
- Create: `scripts/package-web-engine.sh`, `scripts/web-engine-closure.py`, `packaging/linux/gstreamer-plugins.txt`,
  `conformance/selftests/test_package_web_engine.py`
- Modify: `scripts/package-release.sh` (shares the host-library check)

- [ ] build the runtime from the release WPE prefix, the allowlisted plugins (Task 1), the GIO modules and the plugin
      scanner; collect their library closure once, set `$ORIGIN` RUNPATHs into the runtime's `lib/`
- [ ] run the undeclared-host-library check on the runtime and write the host libraries it needs to a list that Task 7
      keeps as package dependencies
- [ ] write `MANIFEST` and the licenses; compute the key from Technical Details; `--print-key` prints it without
      building
- [ ] pack as `tar.zst` (level 19, `--long=27`); print size and SHA-256
- [ ] write tests: an allowlisted plugin and its dependency land in the runtime, a plugin off the list does not, no
      library appears twice, the key changes with the allowlist, the scripts and the arch
- [ ] run `python -m pytest conformance/selftests/test_package_web_engine.py` and `scripts/lint.sh` - must pass
      before Task 5

### Task 5: Download, verify and cache the runtime

**Files:**
- Create: `crates/ngterm-update/src/web_engine.rs`, tests beside the existing ones
- Modify: `crates/ngterm-update/Cargo.toml` (`zstd`), `crates/ngterm-update/build.rs`,
  `crates/ngterm-update/src/download.rs`, `crates/ngterm-gpui/src/main.rs` (`--install-web-engine`)

- [ ] the pins (URL, size, SHA-256, key) from build-time env, the `NGTERM_WEB_ENGINE_URL` override; without pins,
      downloading is unavailable
- [ ] download with progress callbacks and `Range` resume, no global timeout, body capped at the pinned size; verify;
      unpack with the entry rules from Technical Details into `<key>.partial/`; rename into place; lock installers
- [ ] the shared in-use lock taken by the worker launch, and cleanup of other keys only under an exclusive lock
- [ ] `ngterm --install-web-engine <archive>` through the same checks
- [ ] write tests (local HTTP fake): success; resume after a cut; hash mismatch; oversize body; truncated body; a
      traversal entry, an absolute path, a symlink out of the tree, a setuid bit; `ENOSPC`; a crash leaving
      `.partial`; two concurrent installers; cleanup skips a runtime in use
- [ ] run `cargo test -p ngterm-update` - must pass before Task 6

### Task 6: The waiting page shows the download

**Files:**
- Modify: `crates/ngterm-gpui/src/html_overlay/registry.rs` and the page view that draws a waiting page

- [ ] invoke the `frontend-design` skill for the waiting page's download state
- [ ] `EngineState::Downloading { received, total }`: the first page that needs the engine starts the download; every
      waiting page shows its progress; the engine starts when the runtime is in place
- [ ] a failed download shows its reason and a retry; without pins or network, the page says how to install the engine
      (the distro package, or `--install-web-engine`)
- [ ] write tests: state transitions (absent → downloading → starting → running; downloading → failed → retry); pages
      opened during a download wait for it
- [ ] run the `ngterm-gpui` HTML overlay tests - must pass before Task 7

### Task 7: Packages without the engine

**Files:**
- Modify: `scripts/package-release.sh`, `packaging/linux/nfpm.yaml`, `packaging/linux/runtime-libraries.txt`,
  `packaging/linux/AppRun`, `crates/ngterm-wpe-client/` (remove the `bundled-wpe` marker path)
- Create: `conformance/selftests/test_package_release_checks.py`

- [ ] the app tree holds the four app binaries and `ngterm-web`, no WPE, GStreamer or their libraries; linuxdeploy runs
      over the four app binaries only; `ngterm-web` gets no RUNPATH (`patchelf --remove-rpath`); remove the
      `bundled-wpe` marker and its code
- [ ] dependencies (Task 1's names): the Arch package depends on `wpewebkit`; the deb and rpm recommend the versioned
      WPE library package only; the runtime's host libraries (Task 4's list), `bubblewrap` and `xdg-dbus-proxy` stay
      hard dependencies of every package
- [ ] packaging checks: `ngterm` has none of `libWPEWebKit`, `libWPEPlatform`, `libgstreamer`, `libsoup`,
      `libjavascriptcore` in its `NEEDED`; `ngterm-web` has no RUNPATH into `usr/lib`
- [ ] write tests: the `NEEDED` and RUNPATH checks pass and fail on fixture ELFs
- [ ] run the selftests and `scripts/lint.sh` - must pass before Task 8

### Task 8: Release workflow publishes the runtime and pins it

**Files:**
- Modify: `.github/workflows/release.yml`, `scripts/release-artifacts.py`, `conformance/selftests/` (its tests)

- [ ] in job `linux`: compute the key by `--print-key`; when release `web-engine-<key>` lacks this arch's asset, build
      the runtime and hand it to publishing, otherwise download the published one for the smoke test
- [ ] pass the runtime URL, size, SHA-256 and key to the app's cargo build
- [ ] publishing creates or updates `web-engine-<key>` as a prerelease with `--latest=false`, before the app release
      leaves draft; dry runs keep the runtime as a workflow artifact; the runtime's SHA-256 joins the app's
      `SHA256SUMS`
- [ ] the release smoke test runs the packaged `ngterm-web` against the extracted runtime with the build prefix
      hidden: the audio cases of `cases.rs` and a video case
- [ ] write tests: `release-artifacts.py` chooses reuse versus upload per arch, and its engine release carries the
      prerelease and not-latest flags
- [ ] run the selftests, `scripts/lint.sh` and a release dry run - must pass before Task 9

### Task 9: Verify acceptance criteria

- [ ] every Linux package is under 45 MB; the runtime archive under 130 MB per architecture
- [ ] a fresh Ubuntu 24.04 container (headless GUI as README > Test describes), runtime served locally through
      `NGTERM_WEB_ENGINE_URL`: the deb installs without WPE, the first page downloads the runtime, plays a video and
      Web Audio, the second app start reuses it
- [ ] each of the four test machines, reverted to its clean snapshot: install the deb or rpm, then on its desktop open
      a page that plays a video and Web Audio; it takes the engine path Task 1 recorded for it (system engine and no
      download, or the download through `NGTERM_WEB_ENGINE_URL`); the tarball and the AppImage download on at least
      one of them
- [ ] Arch: the package pulls `wpewebkit`, the probe picks the system engine, nothing is downloaded
- [ ] offline: `--install-web-engine` from a file; a tampered archive is refused
- [ ] an older app and a newer one sharing the data dir: neither deletes the other's runtime while it runs
- [ ] scoped clippy and the touched crates' tests green

### Task 10: [Final] Update documentation

- [ ] README > WPE WebKit and the release section: where the engine comes from on each install, the `web-engine-*`
      releases (prerelease, never deleted while pinned), the offline install, the URL override
- [ ] the root docs of `ngterm-wpe` and `ngterm-wpe-client`: the worker binary, the sources, the runtime
      environment, AppArmor inheritance
- [ ] AGENTS.md crate table (if not already done in Task 2)
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification**
- The first real release: `web-engine-<key>` exists with both architectures and is not "latest"; the app downloads it
  on a clean Ubuntu 24.04 and a clean Fedora; auto-update from v0.1.1 lands on a package without the engine and then
  downloads the runtime on the first page.
- AppArmor on Ubuntu 24.04: WebKit's sandbox starts from `ngterm-web` under the inherited profile.

**External**
- Users of the tarball and AppImage need a network connection (or the archive file) the first time they open a page;
  the release notes say so.
