# macOS

The native frontend uses AppKit and Metal, with WKWebView HTML pages. The application bundle targets macOS 14 or
newer on Intel and Apple silicon. The terminal/session libraries retain their macOS 13 deployment target.

Install Xcode command line tools and the Rust toolchain in `rust-toolchain.toml`, then build from a checkout:

```sh
scripts/build-ghostty-vt.sh
scripts/fetch-fonts.sh
cargo build -p ngterm-gpui -p ngterm-ctl -p ngterm-session -p ngterm-headless
scripts/bundle-macos.sh target/ngterm.app --profile debug
open target/ngterm.app
```

For release builds, add `--release` to Cargo and use `--profile release` when bundling. The bundle includes the CLI,
session server, fonts embedded in the executable, themes, terminfo, shell integration, agent integrations, icon,
and build identity. It is ad-hoc signed for local use; public distribution still needs Developer ID signing and
notarization. Install by copying the bundle to Applications. The CLI is `ngterm.app/Contents/MacOS/ngtermctl`.
Resource files live in `Contents/share`, beside `Contents/MacOS`, preserving the staged layout's relative lookup.

A second launch forwards activation over an owner-only Unix socket, protected by a file lock that also serializes
recovery after a crash. Application ids and state directories isolate dev instances. The state and configuration
locations follow the Unix XDG rules documented in the README; `NGTERM_STATE_DIR` overrides the state directory.

When no locale is supplied, pane launches set `LANG=en_US.UTF-8`, so readline preserves Unicode input. Explicit
locale settings take precedence. Frontends and session servers raise their soft descriptor limit to support many
panes; the hard limit is preserved. The remotes watcher accounts for FSEvents reporting `/var` and `/tmp` through
`/private`.

Native services include the AppKit menu bar, physical Carbon global hotkeys, AppKit keyboard events and the layout's Command table,
Text Input Source Services layout locks, system sounds, a private selection pasteboard, HTML clipboard content,
window state queries, Reduce Motion/Transparency, network and wake retries, and UserNotifications banners.
`macos-option-as-alt` selects neither, both, left or right Option as Meta; other Option presses retain layout
text and dead-key composition. Network/wake and accessibility changes arrive through system notifications.
Notifications require launching a bundle and allowing notifications when macOS requests authorization.

WKWebView pages share the native page lifecycle and navigation policy with the Windows frontend. Ephemeral pages
use memory stores; persistent browsing uses a WKWebsiteDataStore UUID derived from the state directory's browser
profile. Navigation, history, title, load failure, bridge replies, file grants, remote page resources, themes,
zoom, focus and window reparenting run through the same core interfaces. Native delegates refuse file choosers,
script dialogs, media capture, new windows and unapproved navigation. Escape is handled by the page responder so it
does not leave fullscreen through AppKit's default cancel action.

## Checks

```sh
ulimit -n 8192  # test binaries bypass application startup; macOS otherwise defaults to 256 descriptors
cargo clippy -p ngterm-gpui --all-targets -- -D warnings
cargo clippy -p ngterm-gpui --no-default-features --all-targets -- -D warnings
cargo test -p ngterm-gpui
cargo test -p ngterm-gpui --no-default-features
scripts/smoke-macos-services.sh
```

The services smoke check uses the real AppKit and WebKit frameworks. It exercises accessibility settings, Carbon
registration/unregistration, HTML clipboard contents, and a WKWebView load, title, theme injection, reparenting and
navigation refusal, fragment/subframe loads, download interruption, native bridge replies, editing menus, dead
keys and reparented focus. The `macos_webview` integration test additionally uses real Wry and the production
bridge scripts to check pending evaluations and promise replies. They can run in a macOS VM without a Metal GPU. The services runner launches a temporary app bundle through
LaunchServices and needs an unlocked desktop for native Edit shortcuts. It writes the clipboard, so use a test desktop.

The graphical terminal requires a Metal device. The available Tahoe QEMU VM's VMware framebuffer exposes none
(`MTLCreateSystemDefaultDevice()` returns nil), so it can validate compilation, nonvisual Rust tests, sessions and
the native services smoke check, but cannot validate GPUI window rendering or the graphical conformance suite.
Run those on a Metal-capable Mac. CI builds, lints and tests both frontend feature configurations on macOS.

## VM validation (2026-11-02)

The supplied Intel Tahoe VM built the frontend in both feature configurations and passed strict clippy. Frontend
model tests passed in parallel with HTML (783) and without it (756); the build-identity test and both sprite
golden tests also passed. The `ngterm-session`, `ngterm-term`, `ngterm-headless` and `ngterm-ctl` suites passed, including the repeated
live-launch stress case and exact Unicode argv replay. Fixture probes use their own deadline while the production
probe retains three seconds. The ignored fontconfig fixtures are Linux-specific; the child-only installer fixture is invoked by its parent test. The post-review
`ngterm-app`, `ngterm-core`, VT and `ngterm-term` model suites passed (149, 3236, 76 and 90 tests respectively).

The native services smoke test passed, including real Command-C/X/V/A/Z and Command-Shift-Z editing and a shifted
menu-model shortcut. The Wry integration test passed with the production bridge transport. The ad-hoc signed
bundle passed `codesign --verify --deep --strict`; its bundled headless executable and CLI created a live zsh session, typed and read a command, closed the session, and
shut down. The graphical executable reported the VM's missing Metal device. Window rendering and graphical
conformance remain unverified on a Metal-capable Mac.

Host regression checks passed: both frontend clippy configurations, 816 Linux frontend model tests, five Windows
bridge protocol tests and 870 conformance harness self-tests (18 skips). The Linux font fixtures require `ttf-dejavu` on CachyOS.
