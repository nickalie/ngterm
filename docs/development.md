# ngterm technical reference

How ngterm is built, tested, released and put together, and where it differs from upstream
[agterm](https://github.com/umputun/agterm). For what ngterm is and how to install it, see the
[README](../README.md).

## Names, paths and differences from upstream

ngterm: [agterm](https://github.com/umputun/agterm) in Rust, a GPUI terminal over Ghostty's libghostty-vt, driven by
`ngtermctl` over a control socket. Linux, Windows and macOS have native frontends; see [Windows](#windows) and
[macOS](macos.md) for their build and test instructions. The macOS frontend requires a Metal-capable Mac;
the Tahoe VM can test native services and nonvisual behavior but has no Metal GPU. Everything below describes the
Linux build unless it names another OS.

- The Linux build's application id is `com.nickalie.ngterm`, its state dir `$XDG_DATA_HOME/ngterm` (else
  `~/.local/share/ngterm`) with the control socket at `<state dir>/ngterm.sock`, and its config dir `~/.config/ngterm`
  (keymap, hooks, ngterm-scoped ghostty config).
  One instance runs per application id: the first owns the id's name on the session bus and serves `GApplication`'s
  D-Bus protocol there (through `zbus`), and a later launch of this build forwards its activation to it and exits
  before touching the state. The Windows build's dirs, control pipe and registrations are in [Windows](#windows).
- The binaries are `ngterm`, `ngtermctl` and `ngterm-session`, the crates `ngterm-*`, and every variable the app sets
  or reads is `NGTERM_*` (a pane's `NGTERM_SESSION_ID`, `NGTERM_SOCKET`, `NGTERM_PANE_ID`, the attach contract's
  `NGTERM_LIVE_*`; upstream's are `AGTERM_*`). A reinstall of the agent-status hooks or the skill over one made
  before the build took these names (in `~/.config/agterm/agent-status`) moves it onto them, into
  `~/.config/ngterm/agent-status`: the rc and Codex blocks, the Claude hooks, this build's old Pi and OpenCode plugins
  (agterm's own, at the same paths, stay) and the skill's marker. The old
  `agterm-*` wrappers stay in the package: shells started before the update keep reporting through them, and a config
  the merge could not repoint (a hand-written Codex block) finds them but reports nothing from a new pane until it
  names the `ngterm-*` ones.
- The CLI is `ngtermctl` (upstream's is `agtermctl`), and the Help menu's installers use this build's names too:
  Install Command Line Tool… links the bundled `ngtermctl` into `~/.local/bin` (an `agtermctl` there is left alone),
  and Install Agent Skill… installs the `ngterm` skill, which drives the app through `ngtermctl`, into
  `~/.claude/skills/ngterm` and `~/.codex/skills/ngterm` (an `agterm` skill there is left alone).
- One extension beyond upstream: Live sessions in bash and PowerShell login shells too (upstream: zsh and fish),
  under the Rust-only capability `live-any-shell`; PowerShell gets ngterm's own shell integration
  (`resources/shell-integration/pwsh`), since Ghostty has none.
- Another extension beyond upstream: Install Agent Status Hooks… also wires a restore adapter
  (`resources/agent-status/ngterm-agent-restore.sh`) into Claude Code's `SessionStart` and `SessionEnd` hooks (Claude
  Code only: Codex gets no restore hooks). On every session start it pins the pane's `session restore` line to the
  live conversation (`claude --resume <id> || claude`), so a rerun launch brings each pane back to the conversation it
  was in last, through /clear, /resume and forks; a deliberate exit clears the pin, an ngterm quit keeps it. It skips
  `claude -p`, SDK runs and subagents. Upstream ships the idea only as its `claude-session-resume` cookbook recipe;
  the script's header holds the details. On POSIX, an ngswap Claude launch uses helper-prepared resume and fresh
  commands instead of bare `claude`, preserving the account and configuration paths. That optional branch needs
  python3 and the ngswap profile recipe; Windows retains the ordinary pin form.
- Another extension beyond upstream: a viewer makes and ends another machine's sessions, where upstream's only attaches
  and detaches. Attach Remote's picker ends with New Session, also offered when the machine has nothing to attach, and
  an attached row's menu has New Session on HOST and End Session on HOST… (`crates/ngterm-core/src/remote/origin_sessions.rs`).
- Another extension beyond upstream: attached rows come back after a relaunch. Each window's file lists them, and the
  next launch lists each machine once and attaches every row it still offers where it stood; a machine that does not
  answer is listed again on the reconnect schedule for up to a week, and a session gone there is named in a banner
  (`crates/ngterm-core/src/remote/relaunch.rs`).
- Another extension beyond upstream, which has no mobile app: Android and iOS apps that view and drive another
  machine's live sessions over ssh ([Android](#android), [iOS](#ios)); both also have a chat view of a pane running
  Claude Code or Codex. The origin serves the chat through the Rust-only `agent.*` commands (`ngtermctl
  agent`, capability `agent-chat`) and an opt-in `agent` kind on the presentation stream, with no desktop UI.
- Another extension beyond upstream, where every overlay is ephemeral: an HTML page overlay (`--html`, `--url`,
  `--image`, `--markdown`, session-wide or `--pane`, and link-click pages) lives until it is closed. A restart of the
  app or the headless server brings back each page in the slot it held
  (`crates/ngterm-core/src/model/page_snapshot.rs`).
  On a session another machine presents, losing that viewer (a detach, a lost link, its row closing, a restart of
  either side) parks the page on the origin, read back as `parked: true`, and the next viewer that attaches is handed
  it again (`crates/ngterm-core/src/remote/page.rs`); a viewer tells the page's close (`page.closed`) from its own
  helper's end (`page.detached`, this build's presentation kind, sent only to an origin that lists it). Closing the
  page (its X, ⌘W/Ctrl+W, `session overlay close`, `window.close()`, a submit, the session's close) ends it for good.
  Page companions are still not kept; a page already open on a desktop origin before a viewer attached stays there;
  a page comes back from its original source, without its in-page state. Rust-only capability `pages-survive`.
- No bundled theme of its own: upstream ships its `agterm` theme and starts a fresh install on it; this build ships
  Ghostty's theme catalog only, so a fresh install starts on Ghostty's built-in colors (the "default ghostty" row),
  and a settings file that still names `agterm` (a build from before the rename seeded it) loads on them too.
- One behavior differs from upstream by choice: restore is lazy. A launch (or a window reopened from its
  file) spawns only each window's selected session; every other restored session starts its shell, re-runs its
  command or attaches to its live session once it is first shown (selected, zoomed, in the dashboard) or a command
  needs its terminal (`session.type`, `paste`, `selectall`, `font.*`, `session.search`, `session.restart`,
  `live.present`). Until then the tree reports it `realized: false` and reads (`session.text`, `copy`) answer
  `session not realized`; its pin stays saved, its captured command carries over to the next launch through a clean
  quit (not a crash: the launch takes a capture off disk so it replays only once), and its live session keeps
  running in the session server, so the tree still reports it `backedByLive`. Sessions made at runtime start at
  once, shown or not. With only the shown sessions spawning, upstream's launch spawn pacer has nothing to pace, and
  the build has none.
- Behavior is checked by the black-box suite in [conformance/](../conformance/README.md), held by a ratchet.

## Downloads and releases

[GitHub Releases](https://github.com/nickalie/ngterm/releases) receive these assets when a `vMAJOR.MINOR.PATCH`
tag matching the workspace version in `Cargo.toml` is pushed:

| Platform | Architectures | Downloads |
|---|---|---|
| Linux | x86_64, aarch64 | `.deb`, `.rpm`, Arch `.pkg.tar.zst`, `.AppImage`, portable `.tar.gz` |
| Windows | x86_64 | per-user `.msi` installer, portable `.zip` |
| macOS 14+ | Intel x86_64, Apple Silicon arm64 | `.pkg` installer, drag-to-Applications `.dmg`, `.app.zip`, binary `.tar.gz` |
| iOS | arm64 | unsigned `.ipa` |
| Android 8.0+ | arm64-v8a, x86_64 | signed `.apk` |

The archives include `ngterm`, `ngtermctl`, `ngterm-session`, `ngterm-headless`, themes, terminfo, shell and agent
integrations, license notices and, on Linux, the HTML overlays' worker `ngterm-web`. Extract the entire archive
together. Linux archives start through `ngterm/AppRun`;
Windows binaries are under `ngterm/bin`; the macOS binary archive has `bin` and `share`. Native Linux packages install
under `/opt/ngterm` and put all four commands on `PATH`. The macOS PKG installs `ngterm.app` into `/Applications`;
the app's Help menu can install the CLI. Windows ZIPs include the same app-local VC++ runtime as the MSI.

Every official release checks GitHub for stable updates at launch and hourly. Updates download in the background and are verified against the release's
`SHA256SUMS`, whose minisign signature must match the key the running build was released with
(`crates/ngterm-update/trust/minisign.pub`). Choose **Install Update** in the titlebar, or **Help → Check for Updates…**, to install after the
normal quit save and reopen ngterm. Ready updates also install on an ordinary quit, without reopening the app.
Live sessions reconnect; other shells close and follow the configured restore
mode. A failed archive replacement restores the previous installation. Windows MSI updates use Windows Installer
without forcing other ngterm processes to close; busy files can require closing those processes and retrying.
Interrupted downloads are cleaned up on a later launch or check. Backups needed for recovery are retained.
Installations the system owns (`.deb`, `.rpm`, Arch packages and a root-owned `/Applications/ngterm.app` from the PKG)
download the matching package into the state directory and install it through apt, dnf, zypper, rpm, pacman or
`installer`, with the app still open. Rights come from polkit (`pkexec`) or the macOS administrator dialog; a Linux
desktop with no polkit agent runs `sudo` in a new ngterm session instead. Dismissing the prompt changes nothing.
These installs happen only from the button, since a quit can't ask for a password. Each package's
`share/ngterm/update-format` marker names its format; builds packaged by others have none and don't self-update.

Disable background checks and downloads in **Settings → General → Updates** (`autoUpdate: false` in `settings.json`);
manual checks remain available. Archive installs must stay together in a dedicated directory named `ngterm`, with
its parent writable by your user. For the macOS binary tarball, create that directory and extract `bin` and `share`
inside it. Shared prefixes and source builds are updated manually. Keep your state and configuration directories outside the installation. AppImages update their
original file; manually extracted AppImages are updated manually.

Linux packages target glibc 2.39+ and libstdc++ 13+ (Ubuntu 24.04 or newer and current Debian, Fedora, Arch, and
openSUSE releases).
A Vulkan-capable graphics driver, `bubblewrap`, and `xdg-dbus-proxy` are required on the host, along with the desktop
libraries `packaging/linux/runtime-libraries.txt` lists: the packages bundle no library, so the app shares the copy the
compositor and the GPU drivers use; the native packages declare all of these dependencies in
`packaging/linux/nfpm.yaml`. Alpine/musl and older glibc
releases need a source build. AppImage users also need FUSE, or can use `--appimage-extract-and-run`. On hosts with
restricted AppArmor user namespaces, native packages load a profile for their fixed install path; a portable install
needs a profile for its extracted `usr/bin/ngterm`; see [WebKit sandbox on Ubuntu](#webkit-sandbox-on-ubuntu-apparmor).
An AppImage on such a host must first be extracted to a permanent directory (`./ngterm-*.AppImage --appimage-extract`),
because its ordinary mount path changes on every launch. For an extracted AppImage, install the bundled profile with:

```sh
binary="$(realpath squashfs-root/usr/bin/ngterm)"
sed "s|@NGTERM_BIN@|$binary|" squashfs-root/usr/share/doc/ngterm/apparmor.profile.in | sudo tee /etc/apparmor.d/ngterm >/dev/null
sudo apparmor_parser -r /etc/apparmor.d/ngterm
./squashfs-root/AppRun
```

For a tarball, use the same commands with its extracted `ngterm` directory in place of `squashfs-root`.

Linux packages carry no web engine. HTML overlays need WPE WebKit 2.54 or newer of the 2.x series, and the app looks
for it when the first page opens: the system's if `ngterm-web --probe` loads a new enough one, then a runtime it already
downloaded, and only then a download.

| Install | Where the engine comes from |
|---|---|
| Arch package | the system's: the package depends on `wpewebkit` and the GStreamer plugins |
| `.deb`, `.rpm` | the system's where the distribution has 2.54+ (Debian testing, openSUSE Tumbleweed): the package recommends it and the playback plugins; otherwise a download (Ubuntu, Debian 13, Fedora, openSUSE Leap 16.1), since a recommendation nothing provides is skipped |
| `.tar.gz`, `.AppImage` | the system's if one is installed, otherwise a download |

A download needs network access once: about 120 MB, shown on the waiting page, verified against the size and SHA-256
compiled into the app, unpacked into `web-engine/<key>/` in the state directory. It is per user and later app versions
share it until the runtime itself changes; a runtime another running version still uses is kept. Builds without release
pins (development and distribution rebuilds) never download and need the system's package.

To install without network access on the machine, fetch the archive from the runtime's `web-engine-<key>` release
(the app release's `SHA256SUMS` names it, `ngterm-web-engine-<key>-linux-<arch>.tar.zst`) and run
`ngterm --install-web-engine <archive>`; the same checks apply. To download from a mirror instead, set
`NGTERM_WEB_ENGINE_URL` when starting the app: it replaces the URL only, and the size and SHA-256 stay pinned.

What is signed: the Android `.apk` with ngterm's own key (which the Google Play release will carry too, so either
install updates the other; its certificate's SHA-256 is in `packaging/android/apk-certificate.sha256`), and every
asset through `SHA256SUMS`, its minisign signature `SHA256SUMS.minisig` and a GitHub build provenance attestation.
Not yet: macOS bundles are ad-hoc signed, not Developer ID signed or notarized, so a download can need a manual
Gatekeeper approval; Windows installers are unsigned; the iOS `.ipa` is unsigned and installs only once re-signed (with
a sideloading tool or your own Xcode team), while the signed app comes through TestFlight and the App Store.
[docs/release-signing.md](release-signing.md) has the details and how to verify a download
(`minisign -Vm SHA256SUMS`, `gh attestation verify <file> --repo nickalie/ngterm`).

To install the Android app, download the `.apk` on the device (or `adb install ngterm-<version>-android.apk`) and
allow installs from that source when Android asks. Later releases install over it: the app notifies you of a new
release and installs it from Settings > Updates, after checking it against the release's signed `SHA256SUMS` and the
installed app's signing key; Android asks you to allow installs from ngterm the first time.

To release, set the workspace version and lockfile, commit, and push the matching tag (for example `v0.1.0`); the
`release` workflow waits for the release reviewer's approval, then builds, signs, verifies and publishes every
platform. A dispatch with **rehearse** on a tag signs with the real keys and publishes nothing; one without it is a dry
run on any ref. [docs/release-signing.md](release-signing.md) covers the runs, the keys and the signing mode.
Tag/version mismatches fail before any builds. Packaging tool versions, checksums, and the WPE source checksum live
in `packaging/release/pins.env`; update the WPE source checksum and relocation patch when changing `WPE_VERSION`.

Each Linux architecture's web engine runtime has its own GitHub release, `web-engine-<key>`: a prerelease that is never
"latest", so the update check keeps seeing the app. The workflow reuses the release while the key is unchanged, and
publishes a new one before the app release leaves draft. Shipped apps pin these files, so never delete a `web-engine-*`
release a shipped version pins ([docs/release-signing.md](release-signing.md)).

## Parity

The build claims upstream v0.41.1 behavior, reported by `version` as `conformance.parityLevel`
([rust-build.md](../conformance/docs/rust-build.md) defines the claim). What upstream brought from v0.37.0 up to that release,
and how this build does it where the OS matters:

- Commands: `recent list|open|clear` (File ▸ Open Recent: the last 20 closed sessions and workspaces), `session wait
  --until STATUS... [--timeout SEC]` (exit 124 on the timeout), `workspace flag on|off|toggle` (Flag All Sessions /
  Unflag All Sessions), `session overlay hide|show|toggle` (hidden overlays keep running; bound to Ctrl+Alt+Shift+J on
  every OS, upstream's ⇧⌘J waiting for per-OS keymap defaults), point-sized floating overlays (`session overlay
  open|resize --width/--height`, read back as `overlayWidth`/`overlayHeight`), `--url --chromeless` for loopback hosts,
  `session scratch --follow` (showing a scratch no longer selects its session unless `--follow` is given), forms in
  `ask` (input fields, choices with an Other answer, hotkeys resolved by keyboard layout), Mark for Attention on a
  session's row menu, agent status in the session pickers and in `pick` items.
- `version` reports the installed build beside the running one (`result.installed`, an `installed:` line when they
  differ): the staged `share/ngterm/app-BUILD`, or the `Info.plist` inside a macOS app bundle.
- Events: `app.launched` once the control server is up, `tree.changed` when an overlay, page, HUD or remote slot opens
  or closes, `pane.focused` when a session's split-pane selection settles.
- A second instance on a state dir another one owns exits 1 before touching it ([Build and run](#build-and-run)).
- Remote presenters: an HTML page opened in a session shown on another machine opens there, its files and bridge
  calls served back from the origin over a page-serve channel (`ngtermctl session overlay page-serve`, which the
  presenter runs over ssh) and loaded under the `ngterm-page` scheme; a page on the origin's loopback (a dev server,
  a status page) loads through an ssh port forward the presenter opens, where upstream refuses it
  (`crates/ngterm-core/src/remote/page_tunnel.rs`); `ask` forms show and status sounds play on the
  presenting machine too. When the presenter shows no pages (a phone took the role first, or a desktop without web
  views), pages go to a viewer of the session that shows them, and so do program overlays when the presenter refuses
  them (a phone), while asks and sounds stay the presenter's (`PresentationHub::surface_holder`). A status sound on
  Linux, which has none of macOS's named sounds, rings the display's bell (Wayland's `xdg_system_bell_v1`, X11's core
  bell).
- Settings ▸ Key Mapping ▸ "Lock in terminals" selects a keyboard layout while a terminal or an HTML overlay page (its
  fields included) has the keyboard and restores the previous one when the keyboard goes to another app or to an ngterm
  text field outside the page: macOS Text Input Source Services, Windows' keyboard layouts, and on Linux XKB's group
  lock on X11 or, on Wayland, the compositor's own control (sway, Hyprland, niri, KDE Plasma); on GNOME, weston and any
  other compositor the setting is disabled with the reason shown (`crates/ngterm-gpui/src/input_source.rs`).
- `notify` banners get a notification slot of their own per session, replaced and cleared as upstream's
  (freedesktop `replaces_id` and `CloseNotification` on Linux, a toast's group and tag on Windows).
- Live sessions: a fish login shell claims `live-sessions` as zsh does, with the shell integration's root merged into
  the pane's `XDG_DATA_DIRS`; a `session new --command` pane closes when its command exits, as outside Live, and
  `--wait` holds its final output (v0.39 reversed v0.38's return to the login shell), while a captured command
  replayed after a clean quit and a `session restart --command` still return to the login shell's prompt; the
  session server touches its namespace lock hourly, so a temp-dir cleaner never ages it out.
- A custom command in a session whose directory is gone runs in `HOME`.
- Companion sessions (v0.39): `session new --parent` nests a companion under its parent's row, `session
  collapse|expand` folds the group, a companion moves only among its siblings, closes with its parent's group and
  comes back with it from close undo and Open Recent, and a parent whose shell exits restarts it while it has
  companions; New Companion and Duplicate as Companion sit on the row menu, in the File menu and in the palette, and
  flat lists label a companion `parent › child`. `session overlay open --companion` opens a page as a companion row
  (WPE WebKit on Linux, WKWebView on macOS, WebView2 on Windows). `session duplicate --companion` re-runs the source pane's foreground
  program, read from the process table: `/proc/<pid>/exe` and `environ` on Linux, `proc_pidpath` and `KERN_PROCARGS2`
  on macOS, `QueryFullProcessImageNameW` and the process's environment block on Windows. Recognized identity
  arguments in a duplicated command are rebound to its new session and panes. A companion of a session
  attached from another machine is made on that machine over ssh (a Windows machine is never that far side).
- Row tints (v0.39): `session tint`/`workspace tint` and `--tint`/`--tint-on` on both `new`s color a row's icon
  and/or name (tree `tint`/`tintOn`), with a Tint submenu on the row menu and Tint Session rows in the palette; the
  colors are AppKit's system ones on every OS, their dark variants on a dark sidebar.
- The flagged view keeps a saved order (tree `flaggedOrder`): rows drag in every flagged layout, `session flag move
  --before|--after` reorders them, and navigation follows it. Edit Flagged Sessions… and Edit Focused Workspaces…
  (View menu, palette, the empty flagged view's Add Sessions… link) toggle membership in a palette that stays open.
- `session close --undoable` (and the `undoable` result field); the tree's per-session `lastSelectedAt`.
- New settings: `attentionIncludesActive` (the attention steps stop on active sessions too),
  `attentionNavigationAcrossWindows` (they reach the other windows, raised through GPUI's window activation),
  `ignoreReduceTransparency` (Appearance ▸ Window: the window stays translucent when the system asks for less
  transparency: macOS reads the Reduce Transparency accessibility preference; Windows reads the transparency
  effects switch, `UISettings.AdvancedEffectsEnabled`; Linux has no such rule), the `linkOpenMode` values `overlay95`, `overlay75` and
  `overlay50` (a link in an overlay of that size, Ctrl+Shift-click sending one to the browser instead where upstream
  has ⌘⇧-click), and the Ghostty config key `split-divider-color` for the split divider.
- Pane messages (v0.40): `ngtermctl msg send|read|ack` delivers to stable `SESSION:PANE-ID` addresses or role
  aliases, or broadcasts with `--all` and optional `--workspace`. Only realized local panes receive mail.
  `--as` overrides the pane's complete
  `NGTERM_SESSION_ID`/`NGTERM_PANE_ID` pair. Mail stays with its pane through swaps, is not saved, and is discarded
  when that pane closes or restarts. Reads repeat until acknowledged; `read --ack` consumes one bounded batch,
  `read --wait --timeout SEC` exits 124 on timeout. The tree reports nonzero `pendingMessages`; `message` events omit bodies.
- Agent mail (this build's own): with the agent-status hooks installed, mail sent to a pane running Claude Code or
  Codex reaches the agent by itself (`ngtermctl agent mail`, run by the hooks): after each tool call, at the end of a
  turn (which then goes on), with Codex's next prompt, and an idle Claude Code is woken by an `asyncRewake` waiter.
  Only the pane's interactive agent takes it, never a subagent, a `claude -p` worker or a `codex exec` run inside the
  pane. An agent weighs mail like any text it did not get from the user, so tell it whose mail to follow.
- Session info (v0.40): `ngtermctl session info TEXT [--markdown] [--hide-after SEC]` draws an activity line below
  the sidebar row; `--clear` removes it. Inline bold, italic, code, `^[text](color: "green")` and
  `^[text](dim: true)` are supported. A positive expiry restarts on every successful set, even with identical text;
  omitting `--hide-after` cancels an earlier expiry. The line is never saved or duplicated. Attached sessions mirror
  the origin's line unless a local line overrides it. Settings ▸ Interface ▸ Session info chooses Always, Flagged or
  Off without clearing the value; Workspace Icons and Session Icons independently control the two row icon kinds.
- Local document pages (v0.40): `session overlay open --image FILE` embeds PNG, JPEG, GIF or WebP with natural
  aspect sizing; `--markdown FILE` renders Markdown with highlighted code, task lists and heading anchors. Markdown
  embeds eligible pictures from its own folder and fetches nothing from the network; raw HTML displays as text.
  These pages have no bridge, refuse companion placement and stay local. Ctrl-clicking a document path opens the
  same page on Linux and Windows (⌘-click on macOS); other files are revealed by the platform's file manager.
- Remote groups (v0.40): `ngtermctl live attach HOST ID --companions` attaches a parent with its companions in
  origin order, retaining successful attachments and reporting individual failures in `remoteGroup`; Attach Remote
  offers the same group action. Attaching one companion joins its already attached parent. Remote program overlays
  and HTML pages retain origin-requested `--width`/`--height` on presenters advertising `overlay.points`; an older
  presenter receives percent sizes. Point resizing is requested on the origin, rather than its replica.
- Managed Live attach exit (v0.40) preserves the final screen and scrollback while resetting terminal input modes,
  styles and colors. A completed creation command is not restarted as a replacement login shell; `--wait` keeps
  its output visible.
- Text pages (v0.41): Ctrl-clicking a source, config, text or log file, or a `path:line` location, opens a read-only
  highlighted page at that line on Linux and Windows (⌘-click on macOS; `session overlay open --text FILE [--line
  N]`, tree `htmlOverlays[].text`). A path that ends a sentence (`/tmp/review.md.`) opens without the punctuation; on
  Windows, which drops a name's trailing dots on open, the dotted spelling is not tried when the undotted one exists.
  Revealing a path in the file manager moved to a chord of its own: Ctrl+Alt-click or Super+Alt-click on Linux and
  Windows (a Linux desktop may take one of the two for its window moves; AltGr counts as Ctrl+Alt on Windows
  layouts), ⌘⌥-click on macOS, with Shift added under mouse capture. A click over a hidden page an earlier click
  opened replaces it (`htmlOverlays[].clicked`).
- Clicked HTML files (v0.41.1) open as a document page with no script, no bridge and no network
  (`htmlOverlays[].document`). Each engine blocks every network load before the page's first one: WPE's user content
  filter, WebView2's request filter, WKWebView's content rule list. WebView2 has no per-page switch for link hints,
  so on Windows every `<link>` of the page is made inert and DNS prefetch is off, and no preconnect leaves it.
- Log pages (v0.41): a clicked `.log` or `.out` file, or `session overlay open --text FILE --tail N
  [--follow-file]`, shows the file's last lines and follows its appends, rotation and truncation (`session overlay
  follow on|off|toggle`; scrolling up pauses it). Each read compares the file's stamp: the device and inode, size and
  nanosecond modification time (`fstat`) on Linux and macOS; the volume serial number, file index, size and last-write
  time (`GetFileInformationByHandle`) on Windows, where the file is opened with the default sharing so its writer can
  still rotate it.
- Search in text pages (v0.41): `session overlay search TEXT|--next|--prev|--close`, and Find (Ctrl+Shift+F on Linux
  and Windows, ⌘F on macOS) goes to what is on top of the focused pane: over a text page it focuses the page's own
  search field, otherwise the terminal's Find bar. The chord reaches the app while a page has the keyboard: WPE's host
  routes the keymap's chords ahead of the page's on Linux, and WebView2's view hands it to the window on Windows.
- Diff pages (v0.41.1) step between changes (`session overlay changes --next|--prev`, tree `text.changes`) and color
  each file's code by its language over the row bands.
- On a session presented on another machine (v0.41), image, Markdown and text pages open on the presenter, its
  viewer listing `page.content`; a log page stays a local one.
- Status shapes (v0.41): triangle-down, play, seal, bolt and bell, drawn as outlines traced after SF Symbols on Linux,
  Windows and Android; a viewer that does not list `status.shapes` gets only the six shapes it decodes.
- Row tints (v0.41): teal, pink, brown, indigo and mint, and `#rrggbb` hex tints from the CLI, with a "Custom
  #rrggbb" entry on the Tint menu of a hex-tinted row; a new tint starts on the last Show On pick.
- Sidebar (v0.41): a compact density (`sidebar density default|compact|toggle`, tree `sidebarDensity`, Settings ▸
  Appearance ▸ Sidebar density); a Workspace chevrons setting, with Collapse/Expand Workspace on the workspace row menu;
  the palette's Move Session rows act on the live selection.
- Live panes (v0.41) start the shell the ghostty config's `command` names in place of the login shell: zsh and fish
  as upstream, bash and PowerShell too (`pwsh.exe` or `powershell.exe` on Windows). The quit-time capture of what Live
  panes run gives a slow session server 300 ms.
- `tree.snapshotUnsaved` (v0.41) reports a window whose state saves keep failing. A pane admits one queued title at a
  time, and the OS window title changes at most twice a second (v0.41.1).
- Selecting text stays off the system clipboard ([The selection clipboard](#the-selection-clipboard)).

## Prerequisites

- Rust 1.98.1 with clippy and rustfmt, pinned by `rust-toolchain.toml` (`rustup toolchain install` reads it).
- Zig 0.17.0 for libghostty-vt (`LIBGHOSTTY_VT_ZIG_VERSION` in `build/pins.env`). `scripts/build-ghostty-vt.sh`
  takes `NGTERM_GHOSTTY_VT_ZIG`, a zig 0.17.0 on `PATH`, else it downloads the pinned ziglang.org archive
  of the host's OS and arch and checks its sha256. A Mac or a Windows machine needs a little more
  ([macOS and Windows](#macos-and-windows)).
- pkg-config, and the development files GPUI links: Wayland, xkbcommon, xcb, fontconfig and freetype.
- WPE WebKit 2.54+ with its WPEPlatform API for the HTML overlays (feature `html`, the default; see
  [WPE WebKit](#wpe-webkit)).
- For the native builds: curl, tar, xz, zstd, patch, sha256sum (or `shasum` on macOS) and ncurses' `tic`/`infocmp`;
  for `scripts/build-weston.sh` also meson, ninja, python3, a C++ compiler and weston's own development files (the Ubuntu
  list below); for regenerating the libghostty-vt bindings `bindgen` (`cargo install bindgen-cli`).
- A Vulkan loader and driver: GPUI renders through wgpu on Vulkan (Mesa's drivers, lavapipe for every headless app
  instance).
- For the app's tests: Xvfb's `xvfb-run` and dbus' `dbus-run-session`, so they get a display and a session bus of
  their own instead of the desktop's.
- For the HTML page harness (`cargo test -p ngterm-gpui --test html_overlay`): first build its worker with
  `cargo build -p ngterm-wpe --bin ngterm-web`. The process tests that load pages (`--test privileges`,
  `--test quit`) need that worker too. The HTML harness also needs GStreamer's base and good plugins,
  whose absence fails its audio case (it is not skipped).
- For the app's process tests (`crates/ngterm-gpui/tests`): dbus-daemon, a private bus per test; the ones with a
  display also need weston, Xvfb and Mesa's lavapipe (`mesa-vulkan-drivers`, an unpacked one under the vendor dir, or
  `NGTERM_LAVAPIPE_ICD`), the keyboard layout tests `setxkbmap` (`x11-xkb-utils`). A missing one skips the test
  (`SKIPPED:`, shown with `--nocapture`) unless `NGTERM_REQUIRE_DISPLAY_TESTS` is set, as CI sets it, which fails it
  instead. The nested-niri checks (niri inside a headless weston: the activation check, the keyboard-layout lock's niri
  backend) are `#[ignore]`d, since CI has no niri: `cargo test -p ngterm-gpui --test instance --test input_source --
  --ignored`. The external texture
  test (`--test external_texture`, dmabufs drawn by GPUI's headless renderer) runs on lavapipe and on the first render
  node a Mesa Vulkan driver runs; its synthetic dmabufs need read-write access to `/dev/udmabuf` (a kernel built with
  `CONFIG_UDMABUF=y`, never a module; systemd's udev rules give the device to the `kvm` group; skipped without, failed
  under `NGTERM_REQUIRE_DISPLAY_TESTS`), its hardware case GBM and EGL.
- For the OpenCode plugin's tests (`crates/ngterm-app/tests/opencode_status.rs`): Node.js 22.7+ (or 20.19+ on the 20.x
  line), which imports the plugin's bare ES module. A missing or older one skips them (`SKIPPED:`) unless
  `NGTERM_REQUIRE_NODE_TESTS` is set, as CI sets it, which fails them instead.
- For the conformance suite: Python 3.11+ with venv, weston and dbus-daemon (each instance gets its own headless
  weston and bus); zsh or fish as the login shell for the Live tests of that shell. Its ui layer also needs Xvfb (an
  X11 display per ui instance), at-spi2-core (`at-spi-bus-launcher` and `at-spi2-registryd`, found in the
  distributions' libexec directories too), the `Atspi-2.0` typelib (Arch's `at-spi2-core`, Debian and Ubuntu's
  `gir1.2-atspi-2.0`) and the suite's `linux-ui` extra (PyGObject and python-xlib; pip builds PyGObject against the
  gobject-introspection (`libgirepository-2.0-dev`), cairo and Python development files with a C compiler). A missing
  one fails a run that selects the ui layer (exit 2, naming it).
- For the suite's code coverage (`run-linux.sh --coverage`): cargo-llvm-cov (`cargo install cargo-llvm-cov`) and the
  toolchain's `llvm-tools` component (`rustup component add llvm-tools`).

On Ubuntu 26.04, the `linux` CI job's packages (the last two lines, with meson and ninja, build weston 15, which
the GPUI runs need and 26.04 lacks; fish and zsh run the replay scripts' tests in their shells) plus `zstd` and
`python3-venv`. Ubuntu has no WPE WebKit, so the app builds there without its HTML overlays ([WPE WebKit](#wpe-webkit)):

```sh
sudo apt-get install pkg-config libfontconfig-dev libfreetype-dev \
  meson ninja-build libwayland-dev wayland-protocols libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev \
  curl xz-utils zstd patch ncurses-bin fish zsh fontconfig fonts-dejavu-core \
  dbus-daemon weston xvfb x11-xkb-utils libgl1-mesa-dri libegl-mesa0 mesa-vulkan-drivers python3-venv \
  g++ libpixman-1-dev libinput-dev libevdev-dev libudev-dev libdrm-dev libegl-dev libgles-dev libgbm-dev \
  libcairo2-dev libpango1.0-dev libpng-dev hwdata
```

On Arch, the `conformance` CI container's packages but git and rustup (WPE WebKit and lavapipe from the distribution;
zsh and fish for the Live tests of those login shells; at-spi2-core and gobject-introspection for the ui layer):

```sh
sudo pacman -S --needed wpewebkit bubblewrap meson ninja wayland wayland-protocols libxkbcommon libxcb \
  curl xz zstd patch ncurses fontconfig ttf-dejavu procps-ng weston dbus xorg-server-xvfb mesa vulkan-swrast \
  pixman libinput libevdev libdrm cairo pango libpng hwdata seatd lcms2 python zsh fish \
  at-spi2-core gobject-introspection
```

Run-time dependencies of every `ngterm` binary:

- a Vulkan driver (GPUI renders through wgpu), Wayland or X11;
- WPE WebKit for HTML overlays (`ngterm-gpui`'s default `html` feature), linked by the `ngterm-web` worker beside the
  app and not by `ngterm` itself. Without the feature (`--no-default-features`) the app refuses every page. On Ubuntu
  23.10+ see
  [WebKit sandbox on Ubuntu (AppArmor)](#webkit-sandbox-on-ubuntu-apparmor). A page's audio and video play through
  GStreamer's plugins, which WPE WebKit leaves optional: base for any media (playbin, Ogg, Opus, Vorbis), good, bad
  and libav for the other formats;
- for Live sessions, `ngterm-session`, the live-session server the workspace builds, staged next to the binaries with
  its build id. libghostty-vt is linked statically; the quick terminal's layer-shell panel is GPUI's own Wayland
  surface, a window where the compositor offers no layer-shell.

### WebKit sandbox on Ubuntu (AppArmor)

WebKit runs the web process of every HTML overlay inside a bubblewrap sandbox, which needs an unprivileged user
namespace. Ubuntu 23.10 and later (`kernel.apparmor_restrict_unprivileged_userns=1`) grant one only to a program whose
AppArmor profile allows `userns`, as the profiles Ubuntu ships for its own WebKit apps (epiphany, devhelp) do.
Without one, `bwrap` fails (`setting up uid map: Permission denied`) and WebKit would abort the whole app, so ngterm
refuses every HTML page instead and names this fix in the error. The sandbox stays on either way.

The fix is ngterm's profile, [packaging/linux/apparmor/ngterm](../packaging/linux/apparmor/ngterm): Ubuntu's shape,
unconfined plus `userns`, attached to the `ngterm` binary by its real path (the `ngterm-web` worker it starts and that
worker's `bwrap` inherit it, so neither needs a profile of its own):

```sh
scripts/install-apparmor-profile.sh /path/to/bin/ngterm   # writes /etc/apparmor.d/ngterm (sudo) and loads it
scripts/install-apparmor-profile.sh --uninstall           # unloads and removes it
scripts/install-apparmor-profile.sh --print [BINARY]      # shows the profile, changes nothing
```

Without a path it covers the conformance stage's `target/conformance-stage/bin/ngterm`. A moved binary needs the profile installed again for its new path. On
a host without AppArmor, or one that does not restrict user namespaces, the script installs nothing.

## Native dependencies

The native dependencies are fetched, prepared or built outside cargo, each into its own vendored prefix
`${NGTERM_VENDOR:-~/.cache/ngterm/vendor}/<name>-<key>/`. The key hashes the build's pins in `build/pins.env` and
its patches in `build/patches/`, so a complete prefix is reused as is and a pin of one build never rebuilds another.

```sh
scripts/build-ghostty-vt.sh    # the pinned, patched libghostty-vt source (Ghostty's vt C API) with its Zig packages,
                               # and Ghostty's themes, shell integration and terminfo at the same pin
scripts/fetch-wpe.sh           # the pinned WPE WebKit binary package, only where the system has none (below)
scripts/fetch-fonts.sh         # the fonts Ghostty embeds, which the GPUI frontend embeds too (below)
scripts/fetch-lavapipe.sh      # Mesa's lavapipe for headless GPUI runs: the system's, else the pinned Arch package
scripts/build-weston.sh        # weston 15 for headless GPUI runs where the system's is older (Ubuntu 26.04: 14)
```

The app's headless instances (the conformance suite's, `ngterm-gpui`'s window tests') render on Mesa's software
Vulkan driver lavapipe only, never on the machine's GPU. The render-path harnesses also render offscreen on a hardware
render node, each GPU in a child process of its own: `ngterm-gpui`'s `external_texture` and `html_overlay` tests (on
lavapipe and on the first render node a Mesa Vulkan driver runs) and `ngterm-wpe`'s engine tests (on the first DRM
render node and in shared memory, [WPE WebKit](#wpe-webkit)).
`scripts/fetch-lavapipe.sh` prints its ICD
manifest: `NGTERM_LAVAPIPE_ICD` when set, else the distribution's (Ubuntu/Debian/Fedora `mesa-vulkan-drivers`, Arch
`vulkan-swrast`), else it verifies the pinned Arch `vulkan-swrast` package (`LAVAPIPE_*` in `build/pins.env`) and
unpacks it into the vendored prefix, refusing a host whose libLLVM or glibc it cannot load (`--print-icd` finds without
fetching); a manifest whose driver is missing is never used. Both the suite and the process tests ask it
(`tests/support/lavapipe.rs`), so they share one rule.
They also need a headless weston with a seat, which only `--fake-seat` gives (weston 15.0 and later): where the system's
is older, `scripts/build-weston.sh` builds the pinned weston (`WESTON_*`, with the wayland-protocols and
libdisplay-info it needs as subprojects) into the vendored prefix; `run-linux.sh` takes it when
the weston on PATH has no `--fake-seat`, and the process tests take the first weston on PATH that has it (put the
prefix's `bin/` first, as CI does).

Each script also takes `--print-prefix` and `--verify`. No cargo build reaches the network: `ngterm-ghostty-vt-sys`'s
build script runs `zig build -Demit-lib-vt` over the libghostty-vt prefix's tree for cargo's target (as herdr's does),
with the pinned zig the prefix's stamp names, installing and caching under its `OUT_DIR` and sharing Zig's global
cache in the vendor root (`zig-cache-<version>`), and fails with "run scripts/build-ghostty-vt.sh" when the prefix is
missing; `ngterm-gpui` fails with "run scripts/fetch-fonts.sh". A fresh `OUT_DIR` (a new target dir or profile) builds
the library again, a few minutes.
libghostty-vt builds the same way on macOS and Windows ([macOS and Windows](#macos-and-windows)).
`NGTERM_GHOSTTY_VT_PREFIX` names a prepared prefix and skips the script on any OS.
The patches `scripts/build-ghostty-vt.sh` applies to the pinned source (`build/patches/ghostty-vt/`, in name order,
with no fuzz) are recorded in [that directory's README](../build/patches/ghostty-vt/README.md): one exposes data the
library stores as C API the app needs, the others change its logic for a fix or a parity behavior Rust cannot reach
from outside, each with its reason, upstream status, remove-when condition and verification. A pin bump rebases them
all and re-runs every verification, and drops a patch only once the pinned source has its behavior.
The pin is Ghostty's `v1.4.0` release (`LIBGHOSTTY_VT_REV` holds its commit), built with Zig 0.17.0, whose
optimize names changed (the build passes `-Doptimize=fast`, Zig 0.16's `ReleaseFast`) and whose `zig build` takes
the vendored global cache from `ZIG_GLOBAL_CACHE_DIR` only. Its patches: 0002 (hosted Wuffs builds, the
allocator-stub half), 0003 (the cursor hyperlink across a page replacement), 0004 (skipped cells unstyled, HTML only:
the VT formatter steps over gaps itself since this release) and 0005 (clipboard callback metadata); 0001, the
placement getters, is retired by the release's render-state image API. What this pin brought to the app: Kitty
placements, placeholders included, are drawn from that render-state API; animations run on the library's
`ghostty_kitty_graphics_animation_tick`; the session server cuts its Kitty reply barriers at `vt_write_until`
boundaries, and cancelled or aborted APCs leave none; binary terminal snapshots carry Kitty images, placements and
pixels, so an update handoff between builds of the same pin keeps them (the session server still loads no
file/temp/shm media); and the Kitty clipboard protocol (OSC 5522) runs at full fidelity
([The selection clipboard](#the-selection-clipboard), [Live sessions](#live-sessions)).
`LIBGHOSTTY_VT_*` is the one Ghostty pin (the source archive's sha256 and the theme package's included): it follows
the latest Ghostty release, not upstream agterm's `GHOSTTY_REV`, whose vt API lacks what the app needs. libghostty-vt is linked
statically; staging takes the prefix's `share/` (the resources) only. `.cargo/config.toml` makes `scripts/pkg-config.sh`
the pkg-config of every cargo build (it picks the WPE WebKit, below).
`scripts/gen-ghostty-vt-bindings.sh` regenerates the bindings of `ngterm-ghostty-vt-sys` (with bindgen's layout
assertions; `--check` compares a regeneration with the checked-in file) after a `LIBGHOSTTY_VT_REV`
or ghostty-vt patch change, `scripts/gen-ghostty-config-keys.sh` ngterm-core's ghostty config key table after a
`LIBGHOSTTY_VT_REV` bump, `scripts/gen-wpe-bindings.sh` the gir crates `ngterm-wpe-sys`, `ngterm-wpe-platform-sys` and
`ngterm-wpe-javascriptcore-sys` after a `WPE_*` or `GIR_*` bump.

The GPUI frontend's terminal embeds the fonts Ghostty embeds, from the package deps of the Ghostty at
`LIBGHOSTTY_VT_REV` (`FONTS_*` in `build/pins.env`, each archive's sha256 pinned): JetBrains Mono 2.304 (its static
faces; SIL Open Font License 1.1), the face when `font-family` is unset, and Symbols Nerd Font Mono 3.4.0 (MIT), every
family's symbol fallback. Their licenses are in the prefix's `share/doc/fonts/`; a package of the binary ships them. The terminal's procedural sprites (box drawing, blocks, powerline, braille, legacy computing) are a port of
Ghostty's sprite font through qwertty-term (both MIT); `crates/ngterm-gpui/tests/golden/` holds Ghostty's sprite test
images with Ghostty's license.

Some GPUI dependencies are patched in the tree rather than built: `vendor/calloop` and
`vendor/gpui-pre{,-wgpu,-linux,-windows}` (GPUI's dmabuf external surfaces for the HTML overlays, the font metrics
accessors and the coverage masks the terminal's sprites paint with, the native key behind each key event on Linux and
Windows, the HTML clipboard offer, per-MIME clipboard access, xdg-output geometry and X11 window state) are verified crates.io sources plus `build/patches/<crate>`, recreated by
`scripts/vendor-calloop.sh` and `scripts/vendor-gpui.sh` (`--check` compares, as CI does; `vendor-gpui.sh --diff`
turns edits under `vendor/` back into the patch files). `vendor/portable-pty` (the PTY and child under `ngterm-pty`:
ConPTY from kernel32 only, herdr's fixes for malformed Windows environments and directories on `PATH`, the child
created in its job, a ConPTY session's handles exported and imported for the session server's update handoff) is
the same, through `scripts/vendor-portable-pty.sh` (`--check`, `--diff`), and so is
`vendor/accesskit_atspi_common` (AccessKit's AT-SPI mapping: a disabled control without Enabled, a menu button's
role), through `scripts/vendor-accesskit-atspi.sh`, and `vendor/accesskit_windows` (AccessKit's UI Automation
mapping: a highlighted menu item keeps its Invoke pattern), through `scripts/vendor-accesskit-windows.sh`.

### macOS and Windows

The `cross-platform.yml` workflow builds and tests the terminal/session libraries and services on all three OSes,
and the native frontend with and without HTML overlays on macOS and Windows. [macOS](macos.md) covers the app
bundle and native services; [Windows](#windows) covers its installer and tests. Prepare the pinned Zig source and
then run Cargo:
```sh
scripts/build-ghostty-vt.sh    # stock bash 3.2 on macOS, Git Bash on Windows
cargo test -p ngterm-session -p ngterm-proto -p ngterm-host -p ngterm-core -p ngterm-ghostty-vt-sys \
  -p ngterm-ghostty-vt -p ngterm-pty -p ngterm-term -p ngterm-server -p ngterm-ctl -p ngterm-app \
  -p ngterm-headless -p ngterm-mobile
```

A Mac, a Windows machine or a VM of either needs, besides Rust from rustup (`rust-toolchain.toml`):

- macOS: Xcode's command-line tools (`xcode-select --install`), the C toolchain and SDK that rustc links with and
  Zig's macOS build of libghostty-vt reads (highway's `apple_sdk`). curl, tar, patch, `shasum` and `tic`/`infocmp`
  are stock.
- Windows: Visual Studio Build Tools with the C++ workload (MSVC and the Windows SDK): rustc's `*-pc-windows-msvc`
  targets link through it, and Zig builds libghostty-vt for `windows-msvc` only against it. Git for Windows: its Git
  Bash runs the script and brings curl, tar, patch, `sha256sum`, `unzip` (Zig's archive), `tic`/`infocmp` and
  `cygpath`. PowerShell 7 (`pwsh`): `ngterm-session`'s PowerShell session tests require it on Windows and run
  elsewhere wherever one is installed. The app, its installer and the suite need a little more
  ([Windows](#windows)).

The script picks the pinned Zig of the host's OS and arch (`LIBGHOSTTY_VT_ZIG_SHA256_<OS>_<ARCH>`). On Windows the sys
crate's build script runs it with Git for Windows's `bin\bash.exe`, found beside `git.exe` or under the install
locations (never `bash` from `PATH`, which can be WSL's), takes the MSYS path it prints through `cygpath -m`, and
links `ghostty-vt-static` through MSVC; on macOS it links the static archive from a directory of its own, built for
the deployment target rustc links for. `tic` on macOS and Windows writes the terminfo in its hashed layout
(`share/terminfo/78/`), which every reader takes beside `x/`; the terminfo source (`share/terminfo/ghostty.terminfo`)
is staged everywhere. Targets no machine runs natively are cross-built with `cargo build -p ngterm-ghostty-vt-sys
--target …`, as CI does: aarch64 Linux and x86_64 macOS from Linux, aarch64 Windows from Windows (Zig finds no MSVC
headers from Linux).

The live-session server works the same on all three ([Live sessions](#live-sessions)), each OS with its own detach and
endpoint. A live session on Windows runs the pane's selected Git Bash, PowerShell, CMD or WSL profile. With no profile,
it uses a supported `command` from `ghostty.conf`, then PowerShell (`pwsh.exe` from `PATH`, else Windows
PowerShell's `powershell.exe`). PowerShell uses ngterm's integration; Git Bash uses Ghostty's Bash integration,
loaded through ngterm's shim (`resources/shell-integration/git-bash`) so that its directory reports carry Windows paths.

### Windows shell profiles

Settings ▸ General ▸ Default shell selects the profile for new local sessions and windows. The sidebar's **+**
menu selects a profile for one new session. Discovery checks `PATH`, standard installation directories and Git for
Windows' registry entries, without executing discovered programs. It offers PowerShell 7, Windows PowerShell,
Git Bash and Command Prompt when their executables exist. Registered WSL distributions are read from the user's
Lxss registry keys and offered separately; discovery never starts a distribution.

`%LOCALAPPDATA%\ngterm\settings.json` accepts `shellProfiles` for portable or custom installations. Each entry has a
stable `id`, display `name`, absolute `executable`, separate `args`, optional `env` and optional `startingDirectory`:

```json
{
  "shellProfiles": [{
    "id": "portable-git",
    "name": "Portable Git Bash",
    "executable": "D:/Tools/PortableGit/bin/bash.exe",
    "args": ["--login", "-i"],
    "env": {"CHERE_INVOKING": "1", "MSYSTEM": "MINGW64"},
    "startingDirectory": "D:/Projects"
  }]
}
```

Merge these keys into the existing settings file while the app is closed. Custom entries override detected ones
with the same id. Installed shells are detected once per app launch. The settings picker saves the complete selected object as `defaultShellProfile`; `null` selects
Config / system default. The optional starting directory applies to sessions created from the sidebar and New
Session action; an explicit CLI directory, a split's directory or a restored session's directory remains its own.

Each primary/split pane saves a copy of its profile in its session snapshot (`shellProfile`/`splitShellProfile`).
Splitting and duplicating preserve it; changing the default affects only new sessions. Removing a catalog entry or
an executable does not silently switch existing panes to another shell: a pane whose profile's executable is gone
shows why it could not start above Ghostty's "failed to launch" report, and closes on a key. Old snapshots without profiles retain the
config/system behavior. A malformed profile in a snapshot or `settings.json` (a relative `executable`, an `env` name
holding `=`) is dropped when read, and the pane starts the config/system shell. A `cmd.exe` profile whose `args`
end in `/C <line>` or `/K <line>` passes that line to `cmd` as written, with or without `/S`. The line is one `args`
element, for example Windows Terminal's VsDevCmd line: `"args": ["/k", "\"C:\\…\\VsDevCmd.bat\" -arch=x64"]`.

An explicit creation command (`ngtermctl session new --command`) runs through a CMD or WSL profile, with or without
Live: `cmd <args> /S /C "<line>"` with the line as written, or `/bin/sh -lc <line>` inside the distribution. With
any other profile it keeps the platform's command shell (`cmd /D /S /C` on Windows, `/bin/sh -c` elsewhere) without
Live, and the profile's own shell under Live, as with no profile. A profile's `env` applies whenever its program
runs, not to a command the platform's command shell runs instead.

Live uses the same executable and environment as the selected profile. Git Bash and PowerShell use injected
integration and accept login/interactive flags (plus PowerShell's banner/stay-open flags). Git Bash integration
paths use forward slashes, including paths containing spaces; `CHERE_INVOKING=1` keeps the pane's directory. Its
directory reports name the directory as `cygpath -m` does (`C:/Users/me`, never MSYS's `/c/Users/me`), so a split,
a duplicate or a restarted shell opens in it.

CMD and WSL run directly in the Windows session server's ConPTY. Reattaching preserves the running process,
its directory and variables, including WSL's Linux shell. CMD preserves startup switches such as `/D`, `/Q`
and `/V:OFF`. Any CMD profile whose switches before `/C`/`/K` turn on delayed expansion (`/V:ON`, also with `/K <line>`),
with or without Live, replays no captured command (after a rerun relaunch or a lost server), and one Live can run
directly does not re-run a Duplicate as Companion copy's program: the pane starts the plain profile shell, because
delayed expansion would read a `!NAME!` in their arguments as a variable. Other CMD profiles run a copy through the
platform's command shell. WSL preserves `--distribution`/`-d`, `--user`/`-u` and `--cd` argument pairs. WSL's `--cd` controls
its Linux starting directory; otherwise WSL translates the Windows launch directory normally. Profiles with
one-shot commands or management options (for example CMD `/C` or WSL `--shutdown`) open ordinary sessions,
as the menu/settings indicate, rather than silently dropping their options.

Live CMD adds an OSC 7 directory report to `PROMPT`, preserving the inherited or profile-defined prompt text.
Replacing `PROMPT` inside CMD replaces that integration too. This reports the directory at each prompt; it does
not add command start/end markers. CMD has no equivalent of Bash/Zsh's command hooks, so complete command
tracking would need a separate mechanism.

Live WSL loads the bundled Ghostty integration into the distribution user's Bash or Zsh. Resource paths are
translated with `wslpath` inside that distribution, without assuming `/mnt/c`, and are passed as separate argv
values. Startup files, custom prompts, `ENV`, `ZDOTDIR`, and history settings remain in effect. Additional local
OSC 7 reports identify Linux directories even when the distribution hostname differs from Windows. The last
reported Linux directory is restored inside WSL when a fresh shell starts; an unavailable restored directory
does not prevent startup. Other Linux shells, or missing/inaccessible integration resources, start normally
without injected markers. No user startup files are edited and no helper is installed in the distribution.

WSL foreground Linux processes are not captured by the Windows process table for replay after the server ends.
Live preserves processes while the server and distribution remain running; it does not survive a Windows reboot
or `wsl --shutdown`. Reopening after a lost server starts a fresh shell, without its old variables or process
memory. Explicit WSL creation commands run through `/bin/sh -lc` in the selected distribution; explicit restart
commands run in a login `/bin/sh -lc` (reading the profile's `PATH`) before entering the integrated shell. A surviving daemon always reattaches to
its original process.

`python conformance/tools/check_windows_shell_profiles.py STAGE --shell cmd` (or `--shell wsl --distribution Ubuntu`)
checks real ConPTY sessions, reported cwd, host reattachment, Ctrl+C, quoted restart commands, and fresh-shell
recovery after stopping only its private server. `python conformance/tools/check_wsl_integration.py GHOSTTY_RESOURCES`
checks Bash command markers, exit codes, startup files, history, prompt preservation and fallback in a WSL PTY;
pass `--shell /bin/zsh` to check Zsh. Both use private test state; neither shuts down a distribution.

Focused checks: `cargo test -p ngterm-core --lib profile`, `cargo test -p ngterm-core --lib live`, and
`cargo test -p ngterm-term --lib profile_tests`. Windows GUI smoke tests belong on CI or a throwaway VM: choose
Git Bash, split/duplicate, change the default to PowerShell, then quit/reopen with Live enabled and verify that the
existing Bash process and directory survive. Repeat with CMD and WSL, checking the session leader PID and a shell
variable across host restarts; in WSL also check the Linux PID and distribution. Include a custom path with spaces.

A portable MSVC/SDK layout can be used without registering it system-wide: put its compiler and SDK tools on the
build process's `PATH`, set `INCLUDE`/`LIB`, and set `ZIG_LIBC` to an absolute Zig libc description file naming its
headers and libraries. Start from the pinned Zig's `zig libc` output so all required fields are present (Zig 0.17
requires `darwin_sdk_dir=`, even when empty on Windows). The Ghostty build forwards the file as both `--libc` for
compilation and `-Dlibc-file` for Wuffs' C translation, so both use the same SDK. Unset or empty `ZIG_LIBC` adds
no extra build arguments. Cargo tracks changes to the variable and the referenced file. No native-library
overrides or stubs are needed.

### WPE WebKit

The GPUI frontend's HTML overlays run WPE WebKit offscreen (WPEPlatform API). Its bindings are generated by gtk-rs
`gir` from the `.gir` files of the pinned package (`WPE_*` and `GIR_*` in `build/pins.env`, WPE WebKit 2.54.1) and
need WPE WebKit 2.54 or newer (`WPE_API_VERSION`) at build time, found through pkg-config (`wpe-webkit-2.0`):

- Arch: the distribution's `wpewebkit` (`sudo pacman -S wpewebkit`). A distribution build has its helper processes'
  path compiled in (`/usr/lib/wpe-webkit-2.0`), so the worker depends on that package; a system `wpe-webkit-2.0` of the
  bound API wins over anything vendored.
- No root, Arch-compatible userland (glibc 2.44+ and the package's libraries): `scripts/fetch-wpe.sh` verifies the
  pinned Arch package (`WPE_ARCH_PACKAGE_*`) and unpacks it into the vendored prefix, which `scripts/pkg-config.sh`
  offers when the system has no WPE WebKit of the bound API. Building and linking need nothing more; running pages
  from that prefix needs the helper path overlaid (`bwrap --ro-overlay /usr/lib` with the prefix's `lib/` on top),
  since nothing relocates it. That overlay is a user namespace with `PR_SET_NO_NEW_PRIVS`, inherited by everything
  started in it (no `sudo` or setuid program, root's files shown as `nobody`'s), so the app never enters it: the
  engine runs in the sibling `ngterm-web` worker under it, driven over a socket by `ngterm-wpe-client`, and the app, its
  terminals, their live sessions and everything they start keep the user's privileges (`cargo test -p ngterm-gpui --test
  privileges`). `ngterm-wpe`'s engine tests (`cargo test -p ngterm-wpe`) run every case through a worker and again in
  process inside the overlay (`ngterm_wpe::launcher::command`). Pick the distribution package whenever you can.
- Ubuntu packages no `wpewebkit` at all (none since 22.04's 2.36, 26.04 included), and 24.04 cannot load the Arch
  package (glibc 2.39): Ubuntu builds the workspace without what links WPE (`WORKSPACE_WITHOUT_WPE` in
  `scripts/lib/native.sh`: the WPE crates and the GPUI frontend) and the GPUI frontend without its HTML overlays
  (`GPUI_WITHOUT_HTML`: `--no-default-features`, the core then refuses every page). The app with its pages (feature
  `html`, the default) needs a WPE WebKit: `run-linux.sh` and `scripts/run-gpui-dev.sh` build it so, and fail
  naming this section where there is none.
- CI: the `wpe` job installs `wpewebkit` from a date-pinned Arch Linux Archive snapshot in a digest-pinned `archlinux`
  container, checks the WPE crates there and that the checked-in bindings equal a regeneration, then the GPUI
  frontend with its HTML overlays (clippy, unit tests, the headless page harness on lavapipe); the `conformance` jobs
  run in the same container.

`ngterm-wpe-client` is the frontend's socket client, with no WebKit link or build dependency. Build
`cargo build -p ngterm-wpe --bin ngterm-web` alongside the frontend; staging keeps the worker beside `ngterm`.
The client pins its image at app startup and verifies its version and recorded commit during the handshake.

Release builds link a relocatable WPE WebKit built from source on Ubuntu 24.04 (`scripts/build-release-wpe.sh`) and
ship none of it. `scripts/package-web-engine.sh` packs that prefix, the GStreamer plugins in
`packaging/linux/gstreamer-plugins.txt`, the GIO modules and their library closure into the runtime archive;
`--print-key` prints the key, which changes with the engine, the packaging scripts, the architecture or a copied
Ubuntu package. The release workflow compiles the runtime's `NGTERM_WEB_ENGINE_URL`, `_SIZE`, `_SHA256` and `_KEY` into
`ngterm-update`. At run time the app picks the source as [Downloads and releases](#downloads-and-releases) describes,
and only the worker's command gets a runtime's library, GIO, GStreamer and injected-bundle paths: the app, its panes
and its session servers never see them.

`ngterm-wpe` is the engine worker over those bindings: WPE WebKit on an engine thread with its own `GMainContext`, a
`WPEDisplay`/`WPEView` subclass rendering on the render node the GPUI side names with the formats it imports, frames
out as dmabufs (SHM as a last-resort fallback), and the WebKit surface the HTML overlay uses: stores, scripts and the
bridge, policies, schemes, the clipboard, `<select>` popups, the context menu's hit test and input methods; in this
process or in a worker (`Host::start_worker` asks for one with the system's package too, as the engine tests do). The
app runs its engine in a worker with either package, whose environment alone its web processes inherit: it names a
least `PulseAudio` request size (`PULSE_PROP`) that keeps a page's Web Audio playing on a Bluetooth sink, and no
terminal sees it. Its
engine tests render headless on this machine's first DRM render node (Mesa; `WPE_TEST_RENDER_NODE` picks another) and
in shared memory, each frame mode on each backend in a child process of its own. The GPUI side
(`ngterm-gpui`'s `html_overlay`) has a headless harness of its own (build `ngterm-web` first, then
`cargo test -p ngterm-gpui --test html_overlay`): the real engine on the GPU GPUI's headless renderer opened, its frames
read back through the external texture element, on lavapipe (SHM) and on a Mesa render node (dmabufs).

Cargo does not notice the provider changing: after installing or removing either, run
`cargo clean -p ngterm-wpe -p ngterm-wpe-sys -p ngterm-wpe-platform-sys -p ngterm-wpe-javascriptcore-sys`.

### Android toolchain

The Android app needs JDK 21, the Android SDK and NDK, the Rust Android targets and cargo-ndk, at the `ANDROID_*` and
`CARGO_NDK_VERSION` pins of `build/pins.env`. `scripts/setup-android.sh` (Linux and macOS) installs them into
`ANDROID_HOME` (else `~/Android/Sdk`): the command-line tools, platform, build-tools, NDK, platform-tools, emulator,
an x86_64 system image with a phone and a tablet AVD, `rustup target add` and `cargo install cargo-ndk`. It
doesn't install JDK 21 or `sshd` (the mobile e2e test), which are system packages: a missing one is reported with the
command that installs it, and the script exits 1. It is idempotent, reports what it did
and could not do, and prints the `JAVA_HOME`, `ANDROID_HOME` and `ANDROID_NDK_HOME` lines to export.

### iOS toolchain

The iOS app needs a Mac with the `IOS_*` and `SWIFTLINT_*` pins of `build/pins.env`: Xcode, the iOS simulator runtime
(Apple Silicon only, no x86_64 slice), the Rust iOS targets and SwiftLint. `scripts/setup-ios.sh` (macOS only) finds
the pinned Xcode and exports it as `DEVELOPER_DIR` (the selected one stays as it was), adds `aarch64-apple-ios` and
`aarch64-apple-ios-sim`, downloads the runtime when it is missing (about 8 GB), creates the `ngterm-phone` and
`ngterm-tablet` simulators on it, installs SwiftLint into the vendor root and checks for the libghostty-vt build's
pinned Zig. It doesn't install Xcode, which needs an Apple ID. It is idempotent, reports what it did and could not do
(exit 1), and prints the `DEVELOPER_DIR` and `PATH` lines to export; `--check` changes nothing and fails when a
piece is missing. Its header has the notes for a Mac reached over ssh (rustup's `PATH`, a host that can't reach
`ziglang.org`).

## Build and run

```sh
cargo build --workspace --release      # without WPE WebKit: "${WORKSPACE_WITHOUT_WPE[@]}" and "${GPUI_WITHOUT_HTML[@]}"
scripts/stage.sh /tmp/ngterm-stage     # bin/{ngterm,ngtermctl,ngterm-session}, share/{ghostty,terminfo,ngterm}
scripts/run-gpui-dev.sh [--release] [--state DIR] [-- NGTERM_ARGS...]   # an isolated dev instance
```

`stage.sh` (into a new or empty directory; `--profile debug` for a debug build) assembles the layout the suite, the
dev script and an install run from.
`run-gpui-dev.sh` (`scripts/lib/dev-instance.sh`; on Windows from Git Bash, [Windows](#windows)) builds and stages
the app into a fresh `/tmp/ngterm-dev-*` root and launches an isolated instance: a unique `NGTERM_APP_ID`, its own
`NGTERM_STATE_DIR` and `NGTERM_CONTROL_SOCKET`, and `NGTERM_*`/`AGTERM_*` inherited from a live terminal
session dropped (all but `NGTERM_LOG`). It prints the `ngtermctl <command> --socket ...` line that drives the instance
and the instance's pid, by which to stop it.

The build uses the installed build's app id, state dir and socket: while an installed ngterm runs, launch a checkout's
build only through `run-gpui-dev.sh`. A plain launch is forwarded to the running instance over the session bus (the
D-Bus name above). One that is not forwarded (no bus between them, or another `NGTERM_APP_ID`) and finds its state
dir owned by a running instance (an exclusive lock on `ngterm.lock` there, held for the owner's life) exits 1 before
it reads or writes the state dir (it only appends its refusal to `ngterm.log` there), with "ngterm: another instance
is already running on <dir>; control it with ngtermctl" on stderr. The control socket's own lock (`<socket>.lock`) stays for instances on different state dirs
that share one socket.

`NGTERM_LOG` sets the log level (`tracing` filter syntax); the log is `ngterm.log` in the state dir.

### The selection clipboard

Selecting text with the mouse copies it to a private selection clipboard, which a middle-click pastes, and leaves the
system clipboard alone; Ctrl+Shift+C (⌘C on macOS) puts the selection on the system clipboard. To have every selection
land on the system clipboard as well, set `copy-on-select = both`; `clipboard` puts it on the system clipboard only,
leaving the selection clipboard (and so a middle-click's paste) as it was, and `false` copies nothing on select. Per OS
(`crates/ngterm-gpui/src/terminal/selection_clipboard.rs`):

- Linux: the selection clipboard is the primary selection (Wayland's primary selection, X11's PRIMARY), which other
  apps share.
- macOS: a private pasteboard named for the app id (`<app id>.selection`), as upstream's is for its bundle id.
- Windows, which has no selection clipboard: an app-wide buffer inside ngterm, so a middle-click pastes the last
  selection made in any of its terminals.

Ghostty's own default copies nothing on select on macOS and Windows; ngterm's built-in defaults set
`copy-on-select = primary` there, for upstream's behavior. A program's OSC 52 write to the primary selection (`p`)
goes to the system clipboard on macOS, as upstream's, and to the selection clipboard on Linux and Windows.

A program using the Kitty clipboard protocol (OSC 5522) reaches the same clipboards per MIME type: text, HTML, images
and any other type it writes land as themselves (the Wayland/X11 offer, `NSPasteboard` types, Win32 clipboard formats),
and its reads get each requested type the clipboard holds. X11 rejects writes whose representations exceed its
server's single-request payload limit, preserving the current selection; outgoing INCR transfers are not yet
implemented. Under `clipboard-read`/`clipboard-write = ask` its prompt
previews what would be disclosed or written (one page per type on Linux and Windows, the text and first image on
macOS) and offers a grant for the program's password in that terminal; OSC 52's "Don't ask again this session" stays
OSC 52's.

The Android and iOS apps take a program's reads and writes the same way, per MIME type, on the device's one clipboard:
it keeps text, HTML and images (Android's clip with its image behind the app's content URI, iOS's pasteboard item),
the read's prompt previews what would be disclosed (a page per type on Android, the text and first image on iOS) and
offers the same grant, and a write completes only once the clipboard took it. Each app's design system has the
details (`mobile/android/DESIGN.md`, `mobile/ios/DESIGN.md`).

The values mean what they mean in Ghostty at `LIBGHOSTTY_VT_REV`, not in the older libghostty upstream links, so two
differ from upstream's: upstream's docs give `copy-on-select = clipboard` for "as well", which there meant both
clipboards and here means the system clipboard only (`both` is the "as well"); and an explicit `copy-on-select = true`
is taken by Ghostty's compatibility handler, the selection clipboard on Linux and the system clipboard only on macOS
and Windows, where upstream's macOS meant its selection pasteboard (`primary` is that).

## Live sessions

Under the restore mode `live` (Settings, or `ngtermctl restore mode`) each main and split pane runs its program in a
live session that outlives the app, held by the session server `ngterm-session` (`crates/ngterm-session`, taken from
[herdr](https://github.com/herdrdev/herdr)'s background session server at `HERDR_REV` in `build/pins.env`). Plain
panes keep a PTY of their own, and an instance with live sessions off starts no server.

- One server per instance. Its namespace is a directory named by the FNV-1a 64 hash of the state dir's resolved path:
  `/tmp/ngterm-live-<hash>` on Linux and macOS, created owner-only, holding the server's socket (`server.sock`) and
  the lock (`server.lock`) the server holds while it listens; `%LOCALAPPDATA%\ngterm\live\<hash>` on Windows, whose
  server listens on a named pipe named after that directory, owner-only and closed to remote clients, and a client
  refuses a pipe whose server runs as another user. A private state dir gives a private namespace, and with it a
  private server.
- Started on demand: the first attach that finds no server (a live pane's, or `ngterm-session attach`'s) runs
  `ngterm-session server <namespace>` detached from itself and waits for it to answer; clients that race to start one
  end up on the same server. Detached per OS: a session of its own on Linux; on macOS also made responsible for
  itself, so it and its sessions keep their own privacy permissions rather than the app's; on Windows a detached
  process outside every job of the launcher's, run from a private copy of the binary in the namespace directory, so
  a new build can be staged over the running one.
- Stopped by itself: never while a session lives, and a few seconds after it holds no session and no client.
  Sessions end with their programs, or through `ngtermctl live kill` and `live reset`.
- The app's live panes attach in process through the server's client library: the session's snapshot, then its raw
  output, into the pane's own libghostty-vt terminal. `ngterm-session attach` is the attach contract's terminal
  client (the `NGTERM_LIVE_*` variables), which the remote path and a terminal outside the app run;
  `ngtermctl clipboard set` writes through `ngterm-session print`. `ngterm-session --help` lists the rest (`list`,
  `kill`, `screen`, `type`, `version`).
- The update handoff: a launch that finds a server of another build than the staged one (its id in
  `share/ngterm/session-BUILD`) asks it to hand every session over to the staged binary. The programs keep running
  with their terminals: the PTYs pass to the new server (descriptors on Linux and macOS; on Windows the ConPTY handles,
  only on the Windows builds the handoff was proven on). A failed handoff rolls back, the sessions stay with the old
  server, and the app offers the Live sessions reset.
- Versions: a connection opens with a JSON control frame whose operations are stable (a new field is optional,
  unknown ones are ignored), then speaks postcard messages of one protocol version (`PROTOCOL_VERSION` in
  `crates/ngterm-session/src/protocol.rs`, which lists each bump: 5 since the Kitty clipboard's lazy attach paste
  bridge). API and attach connections need the exact version and refuse any other with `protocol_mismatch`, so a
  viewer or `ngterm-session attach` of another build upgrades to the server's; a control connection is taken at any
  version, which is how a new build asks an older server for the handoff. The handoff sends binary terminal
  snapshots only between equal libghostty-vt pins, else the formatter's VT replay.
- Across the rename: a server from before the build took the name ngterm runs as `agterm-session`, its sessions named
  `agterm-<id>`; the handoff to `ngterm-session` renames them `ngterm-<id>`, so the panes find them. Their programs
  keep the environment they started with (`AGTERM_*`), and the shells among them keep reporting agent status
  through the old wrappers a reinstall leaves in place. A one-time migration
  (`ngterm_core::live::LEGACY_NAME_PREFIX`).

### A server without a display

File ▸ Attach Remote attaches the live sessions of another machine running ngterm. On a machine with no display (a
server reached over ssh) that is `ngterm-headless` (`crates/ngterm-headless`), staged beside `ngtermctl` and
`ngterm-session`: the core, the control socket and the session server of the app, with no window and no GPUI, Wayland
or Vulkan. It runs every session pane of its state dir in a live session (restore mode `live` whatever the settings
hold, which keep the user's own mode for an app on the same state dir; it refuses to start where the live probe
rejects the host), each in a terminal nobody draws, so every session is attachable. `ngtermctl` drives it as it drives the app; the agents' hooks and the skill work in its panes.

```sh
# on the server: the staged layout (scripts/stage.sh), ngtermctl on PATH (~/.local/bin or /usr/local/bin)
ngterm-headless --detach [--grid 120x40]   # one per state dir, as the app; returns once the process is started (the
# server takes a few seconds to answer, and its own failures go to its log); installs or
# updates the agent skill and status hooks first (--skip-agent-setup leaves them alone)
# SIGTERM, SIGINT or SIGHUP saves and ends it (on Windows the console's Ctrl+C, Ctrl+Break or close); SIGINT or SIGHUP
# ignored at its start stays ignored, so under nohup it outlives the ssh session that started it. --detach starts it
# outside the session and job that ran it (a Windows ssh session takes every other process it started down with it);
# a detached server on Windows has no console to get Ctrl events, so `taskkill` ends it, without the quit save
ngtermctl session new --name build      # sessions to attach; they outlive ngterm-headless, the next start re-attaches
# on the desktop: the server in remotes.conf (one `destination [label]` per line), key-based ssh to it, then
# File ▸ Attach Remote (Attach All takes every session not attached here yet, New Session creates one there), or
ngtermctl live tree SERVER && ngtermctl live attach SERVER SESSION-ID
```

Agent status indicators need the hooks installed on the machine running the agents; installing them on the viewer
alone does not install them on the server. So each start of a staged `ngterm-headless` (a package, a release archive,
`scripts/stage.sh`) runs the desktop's Help installers before any pane's shell starts: the agent skill into each coding
agent's skills directory and the agent-status hooks with every agent's and shell's wiring, installing them the first
time and bringing them up to this build's bundle after an upgrade. Both preserve user settings and leave a
same-named skill or plugin the user wrote alone; the results, and any config they could not safely merge, go to the
log (`ngterm.log` in the state dir). `--skip-agent-setup` starts without touching the configs (a relaunch for a Live
sessions reset keeps it and `--opencode-version`), and a build run straight from cargo's target directory never runs
them, so a dev run leaves the real home's hooks on the installed `ngtermctl`. `--install-agent-hooks` runs the same two installers
and exits without starting or contacting an instance, printing their results, so it also works while the service is
running. If OpenCode's version cannot be detected, its hooks are skipped; pass `--opencode-version v1` or `v2` (to a
start or to `--install-agent-hooks`) to choose it explicitly. After installation, start a new agent process; shells
that were already running need to source `~/.config/ngterm/agent-status/shell/integration.sh` (fish:
`source ~/.config/ngterm/agent-status/shell/integration.fish`) or start a new shell to load the shell integration.

The attached row is as one from a desktop instance: each pane an ssh into its live session (the viewer claims the pty
size), the status, context, split, HUD and notifications streamed over `ngtermctl live present`, asks, program
overlays and pages shown on the viewer. What would be drawn on the server (banners, sounds, an overlay with no viewer)
has no display there: banners go to its log. A session closes when its program ends, as in the app.
It builds and runs on macOS and Windows too. On a Unix origin a viewer's remote commands go through `/bin/sh -c`.
A Windows origin (Windows' OpenSSH server, whichever of `cmd.exe` and PowerShell it runs) has neither `/bin/sh` nor one
quoting both shells read, so there each is `ngtermctl origin-run WORD`, with what to run base64-encoded in the word,
carried out by the origin's `ngtermctl` (`crates/ngterm-core/src/remote/origin.rs`). A viewer tells the two apart once
per host: a `live tree` in the POSIX form refused by a Windows shell, then a shell-neutral probe; the endpoint's paths
say the same to an attach. `ngtermctl` must be on the PATH of that account's ssh logins: the MSI adds it, and a staged
`ngterm-headless` adds its own folder to the user's PATH (not with a private `NGTERM_STATE_DIR`). A full-screen program
reaches a viewer through the origin's ConPTY, which re-renders what it draws, as on any ssh login there.

### An agterm origin

A viewer attaches the live sessions of a Mac running upstream agterm too (`crates/ngterm-core/src/remote/peer.rs`). That
Mac speaks the protocol this remote layer was ported from under its own names, so the viewer forms each remote command
for the app that answers: `agtermctl zmx tree|present|list`, `agtermctl session …`, and for each pane that Mac's bundled
zmx under zmx's attach and lead variables, its role reports read under zmx's title prefix. Nothing of agterm runs on the
viewer: a Unix desktop's pane runs ssh from its `/bin/sh` script, a Windows desktop's through `ngtermctl viewer-run`, the
mobile core over its own ssh channels, and each only sends that Mac the command strings.

- Which app: a listing runs the far side's `ngtermctl live tree` when it is installed and answers, else its
  `agtermctl zmx tree` under a marker line, in one ssh call. The viewer stamps the answer on the endpoint
  (`endpoint.peer`, in `live tree HOST --json` too). A row keeps the app its endpoint names: its later lines, the
  listings its own flows wait on (split, companion, New/Duplicate Session) and its relaunch go to that app only, so an
  app that starts or stops on that Mac later moves none of them, and a saved row records its app (`peer` in
  `remoteSessions`) for a relaunch listing pinned to it. A line for no row (New Session from the picker) follows what
  the host's last listing found. A destination may pin the app, `agterm:HOST` or `ngterm:HOST` (`remotes.conf`,
  `live tree`, `live attach`); the pin stays in the row's host, and ssh gets only `HOST`. The mobile core lists a fresh
  connection's machine before its first control line, so that line is never guessed at the app.
- What agterm lacks (`peer::Capabilities`): the agent chat view (the mobile core refuses its calls), the Kitty clipboard
  bridge (no `attachBridge`), the stream's `name` and `companion.page` frames (its hello never offers them). The title
  bar shows an `agterm` badge beside the host, with that list as its tooltip; `live tree HOST` prints it as a header; the
  mobile core gives each listed session its `peer` and `peer_capabilities` the apps' line.
- Checked against agterm v0.41.1's bundled zmx on a Mac (an attach through these lines reports `zmx-role;<nonce>:leader:<generation>`
  and runs what is typed); the listing and stream are pinned by upstream's own fixtures, as a locked Mac's agterm
  answers no control socket.

## Android

`mobile/android` is a native Android app (Kotlin, Jetpack Compose, phones and tablets) over the Rust core
`crates/ngterm-mobile`, which Kotlin reaches through UniFFI. It is remote-only: it runs no sessions of its own, and
reaches machines running ngterm (`ngterm-headless` on a server, or a desktop instance) over ssh, as File ▸ Attach Remote
does, and Macs running upstream agterm ([An agterm origin](#an-agterm-origin)): their machine card, sessions screen and
session bar carry an "agterm" badge (on the sessions screen and the session bar a tap says what agterm lacks), and their sessions offer no chat view. It lists a machine's live sessions, makes, duplicates and ends them, and attaches them: their panes (one at a time on a
phone, the origin's split on a tablet), status, context, HUD and notifications. Asks show as dialogs, or as
notification actions while the app is in the background; a foreground service keeps attached sessions connected. Its
design system is `mobile/android/DESIGN.md`.

```sh
scripts/setup-android.sh [--check]    # the pinned toolchain (Android toolchain), then export the lines it prints
scripts/build-android.sh [--profile release|debug] [--no-gradle] [GRADLE_TASK...]   # default: assembleDebug
scripts/build-android.sh check assembleDebug   # lint, JVM/screenshot tests and coverage floors, as CI runs it
scripts/build-android.sh :app:koverHtmlReportDebugJvm :app:koverXmlReportDebugJvm # one module's debug JVM coverage
scripts/build-android.sh :koverHtmlReportDebugJvm :koverXmlReportDebugJvm   # both modules, aggregated
scripts/build-android.sh installGithubDebug    # onto the device or emulator adb sees
scripts/test-mobile-e2e.sh            # the mobile core against a real ngterm-headless behind a private sshd (Linux; python3 runs its fake agent)
scripts/test-mobile-e2e.sh --coverage DIR   # the headless app's e2e line coverage into DIR, as CI's headless-e2e job (cargo-llvm-cov)
scripts/test-android-e2e.sh [TEST...]  # the app on the device adb sees, against a real ngterm-headless behind a private sshd, and conformance's stand-in agterm Mac on a second port (Linux)
```

`build-android.sh` builds `libngterm_mobile.so` for arm64-v8a and x86_64 through cargo-ndk, generates the Kotlin
bindings from it and runs Gradle; everything it generates stays under `mobile/android/core/build/`. The debug APK is
`mobile/android/app/build/outputs/apk/github/debug/app-github-debug.apk`.

The app comes in two distributions (product flavors): `github`, the APK GitHub releases carry and the one every debug
task builds, and `play`, Google Play's, which has only a release build (`assemblePlayRelease`, `bundlePlayRelease`).
The GitHub build updates itself from GitHub's releases (`app/src/github/`): at each launch and every twelve hours in
the background (WorkManager, on a network connection, battery not low) it looks for the newest release above the
installed version, skipping drafts, and prereleases unless the installed build is one, and notifies once per version
unless the user said Not now to it. Install downloads the APK with the release's `SHA256SUMS` and
`SHA256SUMS.minisig`, and hands it to Android's installer (a `PackageInstaller` session) only once the signature
verifies with the key desktop builds trust (the mobile core's `release_checksum`, over
`crates/ngterm-update/trust/minisign.pub`), the APK matches its checksum, and it is this app, newer and signed with the
installed app's key. Settings > Updates checks by hand and turns the automatic checks off. Google Play forbids an app
updating itself, so the Play build has none of it: no code, no `REQUEST_INSTALL_PACKAGES`, no WorkManager; `check`
fails if its merged manifest holds any of them.

`check` verifies each module's Kover line, instruction and branch coverage floors and writes HTML/XML reports for
the debug JVM tests (including Robolectric screenshots), per module and aggregated at the root. Reports are under
`mobile/android/{app,core}/build/reports/kover/` and `mobile/android/build/reports/kover/`; CI uploads both as
`android-coverage`. Use the coverage command above for per-class and per-file results, replacing `:app:` with `:core:`
for the bindings module. Only generated code is excluded; handwritten UI, including lambda bodies in Compose's
`ComposableSingletons` classes, stays in the denominator. The whole-percent floors in
`mobile/android/build.gradle.kts` only rise as tests are added, toward 90% for each metric.

On its first run the app creates an ed25519 key, sealed by the Android Keystore, or imports an OpenSSH or PEM key. It
shows the key's `authorized_keys` line (Copy, Share) and the command that appends it: add that line to
`~/.ssh/authorized_keys` of the user the app signs in as, on each machine. A machine needs the staged layout with
`ngtermctl` on `PATH`, where `~/.local/bin` or `/usr/local/bin` will do
([A server without a display](#a-server-without-a-display)).
A machine is a host, port, user and key; its host key is pinned on first use, and a changed one stops the connection
with both fingerprints shown.

Not in v1: HTML pages and program overlays (refused, as a viewer that can't show them refuses them), Kitty images
(the pane reports no support), passwords and jump hosts (public-key auth to the machine itself only), Nerd Font
symbols, a GPU renderer (the terminal is drawn on a Compose canvas), editing the layout from the phone, and Play Store
publishing. A Windows machine can't be reached, as from Attach Remote.

### Agent chat view

A pane running Claude Code or Codex shows as a chat instead of its terminal (Settings ▸ Agents ▸ Agent chat view, on
by default; a session's More ▸ Chat view sets its own). The agent stays a regular TUI in its pane, which the origin and
every other viewer see as before; the chat is a second view of it, read from the agent's transcript on the origin
(`ngtermctl agent chat` on an exec channel, open only while the pane shows as a chat):

- prompts, Markdown replies, tool calls with their output and diffs, subagents folded under their task, and older
  messages paged in as the list scrolls up;
- a composer: Send pastes the prompt into the pane and enters it, Stop sends Esc while a turn runs, and `/` opens a
  dropdown of the agent's slash commands and skills (built-ins, user and project skills and commands, enabled
  plugins'; `ngtermctl agent commands`), narrowed as you type with the desktop palettes' ranking; the paperclip
  attaches a photo, a video or any file (the system's pickers, no storage permission): it goes to the origin over the
  same ssh link (`ngtermctl agent upload`, at most 32 MiB, kept a week under the state directory's `uploads/`) with
  its progress and Cancel shown, and its stored path goes into the prompt for the agent to read;
- a permission prompt, Claude's `AskUserQuestion` or a Codex Plan-mode question, as a card pinned above the composer
  and answered with its buttons;
- the account the agent is signed in with and how much of its weekly limit is used, over the rows: Claude Code's
  email from its global config and its limit from the usage endpoint its `/usage` asks, with its own sign-in
  (`.credentials.json`, or the macOS Keychain); Codex's from its `auth.json` and the rate limits its rollout records.
  What the origin can't read is left out;
- the session bar's Show terminal / Show chat switch for the focused pane: picking the terminal holds for that pane
  until its agent's next conversation, and the agent ending returns it to the terminal. A notification about the pane
  opens its chat at the newest message.

**Answering takes control.** A chat action (a prompt, Stop, an approval, an answer) from a device that does not lead
the pane first takes the lead, as Take control does, waits for the agent to redraw at the phone's grid, and checks
with the agent's screen matcher that the pane shows what the action answers; only then do the keys go. Opening or
reading a chat claims and resizes nothing. An approval goes only to the dialog that shows the card's call: its
command or URL, or a file tool's path relative to the agent's working directory. A card the pane can't confirm (one
answered elsewhere, or an MCP tool's, whose dialog shows nothing the card names) sends nothing and stays up with
Answer in terminal until the agent moves on. Two calls a dialog shows alike (the same command twice) can't be told
apart on screen (`docs/backlog/a-phone-approval-can-answer-a-later-dialog-that-shows-the-same-call.md`).

**The origin** needs this build's `ngtermctl` and its agent-status hooks reinstalled (Help ▸ Install Agent Status
Hooks… on a desktop; on a server, a start of this build's `ngterm-headless` updates them), then a new agent process: the hooks'
`ngtermctl agent hook` call tells the origin which agent a pane runs and where its transcript is. An origin restarted
under a running agent learns it again at the agent's next prompt. An origin without these pieces shows the terminal,
as before, with no error. A Windows origin can't be reached at all yet, as above.

## iOS

`mobile/ios` is a native iOS app (SwiftUI over local Swift packages, with a UIKit terminal view; iPhone and iPad, iOS
26 and later) over the same Rust core, which Swift reaches through UniFFI. It does what the Android app does (see
[Android](#android) for the machines, the key and what isn't in v1), with these differences:

- Its [agent chat view](#agent-chat-view) is the same, in the session's toolbar (Show terminal / Show chat) and its More
  menu (Chat view); the paperclip offers the photo picker and the document picker, and a hardware keyboard sends with
  ⌘Return.
- It reaches Macs running upstream agterm too ([An agterm origin](#an-agterm-origin)): their machine row, sessions
  screen and session title carry an "agterm" badge, the sessions list's footer says what agterm lacks, and their
  sessions offer no chat view. Its UI tests reach one on the fake core (`FakeAgtermWorld`), as it runs no sshd.
- It is connected only while it is in the foreground: iOS can't keep the ssh links up, so there is no counterpart of
  Android's foreground service. Leaving the foreground, it disconnects after a short grace, and its open asks go back
  to the machine; coming back, it reconnects. Asks show as sheets, or as notifications with answer actions until the
  grace ends.
- Background App Refresh lists each machine's sessions and notifies the agents that turned waiting or done, without
  attaching.
- On an iPad each session opens in a window of its own.
- Its key is kept in the Keychain; its design system is `mobile/ios/DESIGN.md`; it ships through TestFlight to the
  App Store ([docs/ios-release.md](ios-release.md)).

```sh
scripts/setup-ios.sh [--check]    # the pinned toolchain (iOS toolchain); build-ios.sh takes its exports itself
scripts/build-ios.sh [--profile release|debug] [--no-xcode] [lint|build|test|coverage|archive...]   # default: build
scripts/build-ios.sh lint test    # the lint gate, then the Unit and UI test plans on both simulators, as CI runs them
scripts/build-ios.sh coverage     # the Unit plan alone, its line coverage summary and floor check, as test measures it
eval "$(scripts/setup-ios.sh --check)"   # then, after test or coverage, a source file's lines no Unit test ran:
scripts/ios-coverage.py uncovered mobile/ios/Packages/NgtermCore/Sources/NgtermCore/CoreInfo.swift
scripts/build-ios.sh archive      # a Release archive for devices (unsigned without the release secrets), checked
eval "$(scripts/setup-ios.sh --check)"   # then the core's tests in the phone simulator, as CI's lint job runs them:
CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUNNER="$PWD/scripts/ios-sim-runner.sh" \
  cargo test -p ngterm-mobile --lib --target aarch64-apple-ios-sim
```

- `build-ios.sh` builds `libngterm_mobile.a` for the device and the simulator, generates the Swift bindings, wraps
  them in `NgtermCoreFFI.xcframework` and runs its actions. Everything it generates stays under the packages'
  gitignored `Generated/` and `mobile/ios/build/` (derived data, the `.xcresult` bundles, the archive). It holds a
  per-user lock, so one `xcodebuild` runs at a time on a Mac, and strips `NGTERM_*` from everything Xcode starts.
- `lint` is SwiftLint (`--strict`, with its analyzer rules), `swift format lint --strict` and a check that every
  `swiftlint:disable:next` gives its reason; the compiler adds Swift 6 with complete concurrency checking and
  warnings as errors. A finding is fixed in the code: there are no baselines.
- `test` runs on the pinned `ngterm-phone` and `ngterm-tablet` simulators, one after the other. The UI tests drive
  the app on a fake core (Debug builds only; `archive` checks the Release build doesn't hold it). A UI test that fails
  runs once more, for XCTest gives up on the app now and then on CI's virtual Macs ("Failed to terminate", "does not
  have a process ID", "Timed out while evaluating UI query"). A test that passed on its retry is named after the run
  with what failed it, red runs included: as a warning when its first attempt failed on those alone, and otherwise
  as a flaky test that fails the run. CI keeps the result bundles and attachments of any run that retried a test.
- The Unit plan measures line coverage of the app's own code: the app target and the packages' library targets
  (`Unit.xctestplan`'s code coverage targets), not the tests, the fake core or the generated bindings; the UI plan
  is not measured. `test` and `coverage` write its summary per target and per file, least covered first, beside the
  result bundle (`mobile/ios/build/Unit.coverage.txt`, and `.json` for scripts), and fail when a target is below its
  floor in `mobile/ios/coverage-floors.txt`. `scripts/ios-coverage.py` measures with Xcode's `llvm-cov` over both
  simulators' profiles and each target's objects (its header says why not `xccov`). Floors only rise, toward 90%.
- Snapshot goldens live in each package's `Tests/*/__Snapshots__/`, one per simulator (`-on-phone`, `-on-tablet`),
  recorded on the pinned runtime, which CI verifies them on. A missing golden is recorded by the run that first
  meets it, which fails; the next run checks it. To re-record the ones that fail, run
  `TEST_RUNNER_SNAPSHOT_TESTING_RECORD=failed scripts/build-ios.sh test` (that run fails too), check the new images
  and run `test` again. A failed snapshot's image is in `mobile/ios/build/snapshot-failures/`, its diff in the
  result bundle.

## Windows

The same app, CLI and session server, built with MSVC: GPUI renders through DirectX, the HTML overlays are WebView2
pages through `wry` (`crates/ngterm-gpui/src/html_overlay/webview2.rs`) in place of WPE WebKit, notifications are
toasts. The application id is `com.nickalie.ngterm` here too, and so are the binaries' and variables' names; what
differs is where things are kept and how they are reached:

- The state dir is `%LOCALAPPDATA%\ngterm` and the config dir `%APPDATA%\ngterm` (the local and roaming application
  data known folders, never the XDG variables; `<state dir>\config` when `NGTERM_STATE_DIR` is set, as on Linux), the
  home `HOME`, else the profile folder (`crates/ngterm-core/src/paths.rs`).
- The control socket's path stays the identity (`<state dir>\ngterm.sock`, `NGTERM_CONTROL_SOCKET`, `NGTERM_SOCKET`,
  `ngtermctl --socket`), and the app serves the named pipe that path names, `\\.\pipe\ngterm-control-<hash>` (the
  FNV-1a 64 hash of the absolute path, case-folded with `\` separators: `ngterm_core::paths::control_pipe_name`),
  owner-only and closed to remote clients; nothing is created at the path itself. `ngtermctl` derives the same name
  and refuses a pipe another user's process serves.
- One instance per application id and state dir: the first serves a named pipe of its own (`ngterm-instance-…`), and
  a later launch forwards its activation over it, lets the primary raise its window and exits before GPUI starts or
  the state is read (`crates/ngterm-gpui/src/instance/windows.rs`); the state dir's `ngterm.lock` refuses a second
  instance on one state dir as on Linux. The quit SIGTERM runs on Linux comes from the console's Ctrl events and the
  session's end (`WM_QUERYENDSESSION`, `WM_ENDSESSION`).
- Toasts go out under the AppUserModelID, which is the application id: every launch sets it on the process and
  registers it (`HKCU\Software\Classes\AppUserModelId\<app id>`, the name and icon the toasts show), and the
  installer's Start-menu shortcut carries it.
- The window hides the system title bar: ngterm's own bar carries minimize, maximize and close, moves the window and
  opens its system menu, as on a client-decorated Linux window (`crates/ngterm-gpui/src/titlebar.rs`). `ngterm.exe`
  embeds the app icon (`resources/icons/ngterm.ico`), which the windows, the taskbar and Alt+Tab show.
- The live-session namespace is `%LOCALAPPDATA%\ngterm\live\<hash>` ([Live sessions](#live-sessions)).
- Attach Remote runs `ssh` by name, without a console window, where Unix goes through `/usr/bin/env`. A remote pane's
  attach and a remote overlay's job, a `/bin/sh` line on Unix, are `ngtermctl viewer-run WORD` here (the pane's
  command line goes through `cmd.exe`): the `ngtermctl` beside the app, named by its path, runs the same steps, the
  keep-alive probe and the reconnecting bar included (`crates/ngterm-core/src/remote/viewer_run.rs`).
- Install Command Line Tool… links nothing: it puts the bundled `ngtermctl.exe`'s folder on the user's `PATH`
  (`HKCU\Environment`) unless it is there already (the installer's is). Install Agent Skill… installs into the home's
  `.claude\skills\ngterm` and `.codex\skills\ngterm`.

Besides the toolchain [macOS and Windows](#macos-and-windows) lists (Visual Studio Build Tools, Rust, Git for Windows,
PowerShell 7):

- the terminal fonts, `scripts/fetch-fonts.sh` from Git Bash, as on Linux;
- the Microsoft Edge WebView2 Runtime for the HTML overlays (part of Windows 11; Windows 10 may lack it): the app
  checks for it at start, and without it refuses every page with a reason that says where to get it;
- for the installer, a .NET SDK, through which `scripts/build-msi.sh` installs the pinned WiX Toolset (`WIX_VERSION`
  in `build/pins.env`, whose comment says why that version) into the vendor root on first use (`NGTERM_WIX` names a
  `wix` of that version instead), and the Visual C++ redistributable files of the Visual Studio the build links with,
  which it puts beside the executables;
- for the suite, Python 3.11 or later with venv (CI runs 3.14); its ui layer's UI Automation bindings (comtypes) come
  with the suite's install; its `--coverage` run needs cargo-llvm-cov and the `llvm-tools` component, as on Linux.

Build and stage, from Git Bash (`stage.sh` takes a destination in either form and stages
`bin\{ngterm,ngtermctl,ngterm-session}.exe` beside `share\`, the layout the suite, the installer and an install run
from):

```sh
scripts/build-ghostty-vt.sh && scripts/fetch-fonts.sh
cargo build --release -p ngterm-gpui -p ngterm-ctl -p ngterm-session    # ngterm-gpui's default `html`: WebView2
scripts/stage.sh 'C:\ngterm-stage'
```

A dev build beside an installed ngterm runs through `scripts/run-gpui-dev.sh` from Git Bash, as on Linux
([Build and run](#build-and-run)): it builds and stages the debug app (`--release` the release one) into a fresh
`%TEMP%\ngterm-dev-*` root and launches it with `NGTERM_*` and `AGTERM_*` stripped from the environment, a
unique `NGTERM_APP_ID` (its own instance pipe, so the launch is not forwarded to the installed app) and a private
`NGTERM_STATE_DIR` (its config dir, its live-session namespace and its control socket, `<state dir>\ngterm.sock`,
follow it). It prints the `ngtermctl <command> --socket <state dir>\ngterm.sock` line that drives the instance and the
app's Windows pid, by which to stop it (`taskkill /F /PID <pid>`), and waits for the app. The smoke checks below and
the suite's harness launch their instances the same way. Its window opens on the desktop it runs on.

The installer is a per-user MSI from `packaging/windows/ngterm.wxs`, whose header says what it installs and why per
user:

```sh
scripts/build-msi.sh dist/ngterm-x64.msi [--staged DIR] [--version X.Y.Z]   # builds and stages first without --staged
msiexec /i dist\ngterm-x64.msi                                               # from cmd or PowerShell; no elevation
```

It installs into `%LOCALAPPDATA%\Programs\ngterm` with the staged layout and the licenses under `share\doc`, puts
`bin\` on the user's `PATH`, and adds a Start-menu shortcut carrying the application id. The version is the staged
`ngterm`'s Cargo version unless `--version` gives one; an install of a higher version replaces the installed one (a
major upgrade). The uninstall removes its files, shortcut, `PATH` entry and the application id's registration, and
leaves the state and config dirs. The session server's build id
(`share\ngterm\session-BUILD`) is not installed, so an installed session server hands no sessions over to a new build
after an upgrade (the .wxs header says why). `scripts/check-msi.ps1` installs an MSI for the current user, checks
what it installed, upgrades and uninstalls it, on a throwaway machine or account only, as CI does.

Tests, from Git Bash. The crates as CI runs them ([CI](#ci)), then the suite, which builds the release app, stages it
into `target/conformance-stage` and runs the protocol, cli, behaviors and ui layers against it (the ui layer drives the
app's windows on this desktop through UI Automation and `SendInput`: a CI runner or a throwaway VM only), held by its
own ratchet,
`conformance/rust-passing-windows.txt` (`check_ratchet.py --list` names it; see
[conformance/README.md](../conformance/README.md), Rust runner and ratchet):

```sh
cargo test -p ngterm-session -p ngterm-proto -p ngterm-host -p ngterm-core -p ngterm-ghostty-vt-sys \
  -p ngterm-ghostty-vt -p ngterm-pty -p ngterm-term -p ngterm-server -p ngterm-ctl -p ngterm-app \
  -p ngterm-headless -p ngterm-mobile --all-features
cargo test -p ngterm-gpui --no-default-features && cargo test -p ngterm-gpui   # without and with the HTML overlays
python -m venv .venv && .venv/Scripts/python.exe -m pip install -e conformance
.venv/Scripts/python.exe -m pytest conformance/selftests/test_transport.py conformance/selftests/test_client.py \
  conformance/selftests/test_windows_launch.py conformance/selftests/test_uia.py   # pipe, launch, UIA mappings
conformance/tools/run-windows.sh [--ratchet-only] [--stage DIR] [--coverage] [-- PYTEST_ARGS...]
```

- `ngterm-gpui`'s tests that need Xvfb, weston, lavapipe, a D-Bus bus or WPE WebKit are Linux-only, and those whose
  panes run POSIX `sh` scripts are ignored on Windows with that reason.
- Windows has no headless desktop: the suite's instances (each on a private root, `NGTERM_*` and `AGTERM_*`
  stripped, its processes ended by pid) open their windows on the desktop it runs on, and `session.reveal` or a
  clicked link reaches the system's handlers. Run it on a CI runner or a throwaway VM only, never beside a live
  ngterm. Its panes run Git for Windows' `sh.exe`; the conformance README's Safety guards have the rest.
- `scripts/smoke-windows-app.ps1` (the app, its panes, quick terminal, single instance, quit, hotkey, toasts and
  layout lock) and `scripts/smoke-windows-pages.ps1` (the HTML overlays) check a debug build on a desktop with a
  private state dir, through `ngtermctl --socket`; their headers say how to run them in a VM.

## Test

The gates' checks. CI runs them on the whole workspace, with the full suite, on every PR and push to main or a `ci/**`
branch that changes more than Markdown nothing builds or tests (docs, plans, READMEs). Locally, a change runs them only
on the part
it changes (these are Linux's; Windows' are in [Windows](#windows), and its and
macOS' gate is [CI](#ci)'s run on a `ci/**` branch or a PR):

- `cargo fmt`, `cargo metadata --locked` and `scripts/lint-report.sh` (a source scan, seconds) stay on the whole tree.
- clippy (both lines) takes `-p` for each crate the change touches and each workspace crate
  that depends on one (`cargo tree -i CRATE --workspace -e normal,build,dev --prefix none` lists them). The
  `--no-default-features` line runs only when `ngterm-gpui` is among them.
- `cargo test` takes `-p` for the touched crates only.
- the tracing-features check runs for a change to dependencies or `Cargo.lock`; a vendor script's `--check` runs for a
  change to its `vendor/` tree or `build/patches/`; a bindings check runs for the pin changes named below.
- the selftests run for a change under `conformance/`.
- `run-linux.sh` runs partially (`-- -k EXPR`, see [conformance/README.md](../conformance/README.md); node ids after `--`
  add to the three layers instead of narrowing them), on the ids that cover the behavior the change touches, and not at all for a change with no app behavior (docs, CI, the
  suite's selftests). It still builds the whole release workspace, since the staged app needs it.

```sh
cargo fmt --all --check
cargo metadata --locked --format-version 1 >/dev/null   # Cargo.lock is current
scripts/check-tracing-features.sh           # no dependency turns on tracing's `log` fallback
scripts/vendor-calloop.sh --check           # vendor/ equals its crates.io sources plus build/patches/
scripts/vendor-gpui.sh --check
scripts/vendor-portable-pty.sh --check
scripts/vendor-accesskit-atspi.sh --check
scripts/vendor-accesskit-windows.sh --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy -p ngterm-gpui --no-default-features --all-targets -- -D warnings   # the app without HTML overlays
scripts/lint-report.sh                      # complexity #[expect]s with their reasons; refuses inner #![allow]
GDK_BACKEND=x11 xvfb-run -a dbus-run-session cargo test --workspace
.venv/bin/python -m pytest conformance/selftests   # the suite's own tests, host-free
conformance/tools/run-linux.sh [--ratchet-only] [--stage DIR] [--coverage] [-- PYTEST_ARGS...]
```

A change to the WPE or gir pins also runs `scripts/gen-wpe-bindings.sh --check`, as the `wpe` CI job does; one to
the `LIBGHOSTTY_VT_*` pins or their patches, `scripts/gen-ghostty-vt-bindings.sh --check` (it needs `bindgen-cli`).

- The app's window tests need a display and a session bus; under `xvfb-run` and `dbus-run-session` they get their
  own and never touch the desktop's. The process tests (`crates/ngterm-gpui/tests`) start their own private weston,
  Xvfb and bus besides, from an environment built from scratch, and render on lavapipe
  (`tests/support/lavapipe.rs`).
- The selftests, and the pytest runs some of them start, import the harness the venv's editable install points at: a
  venv installed from another checkout (a worktree sharing the main checkout's) needs this checkout's `conformance/`
  first on `PYTHONPATH` (`run-linux.sh` puts it there itself).
- `--workspace` includes the WPE crates (`ngterm-wpe` and its -sys crates) and the GPUI frontend's HTML overlays,
  which need WPE WebKit ([WPE WebKit](#wpe-webkit)); without it, pass `"${WORKSPACE_WITHOUT_WPE[@]}"` and then
  `"${GPUI_WITHOUT_HTML[@]}"` (from `scripts/lib/native.sh`) instead, as the Ubuntu CI jobs do.
- `run-linux.sh` builds the release workspace, stages it into `target/conformance-stage`, runs the protocol, cli,
  behaviors and ui layers with `--impl rust` and exits with the ratchet's status. It runs the suite with `$PYTHON`,
  else `.venv/bin/python`, else `python3`, with the suite installed:
  `python3 -m venv .venv && .venv/bin/pip install -e "conformance[linux-ui]"`. See
  [conformance/README.md](../conformance/README.md) and [conformance/docs/rust-build.md](../conformance/docs/rust-build.md).
  It renders on lavapipe from `scripts/fetch-lavapipe.sh`; the ui layer's instances run on X11, each on a private
  Xvfb, their accessibility trees on an a11y bus of the run's own, never the desktop's. A failed test leaves a trace
  (`--trace-mode`) under the report's `traces/`.
- `run-linux.sh --coverage` runs the suite on an instrumented build and writes the desktop app's e2e line coverage
  report (`coverage/lcov.info`, `summary.md`: the total and its distance to 90%, per crate, the excluded share), which
  `conformance/tools/check_coverage.py` holds against `conformance/coverage/e2e-floor-desktop-linux.txt`
  (conformance/README.md > Code coverage). It is CI's job; run it locally only
  for a change to the coverage tooling.
- `scripts/test-mobile-e2e.sh --coverage DIR` does the same for the headless app: `ngterm-headless`, `ngtermctl` and
  `ngterm-session` built instrumented and staged, the mobile e2e, `ngterm-headless`'s integration tests and
  `ngterm-session`'s e2e test run on that build (the tests themselves uninstrumented, the two crates' tests taking the
  build's programs through `E2E_BIN_DIR`), the report in `DIR`, held against
  `conformance/coverage/e2e-floor-headless-linux.txt`. The report waits until no process the tests left (a session
  server in its idle grace) still runs and would write its profile into the pool: one read mid-write is corrupt and
  fails the merge. Also CI's job.
- `scripts/unit-coverage.sh OUT_DIR [-p CRATE]... [-- TEST_ARGS...]` measures what `cargo test` covers: one
  instrumented build and run of the workspace's tests (the -sys crates left out, and `ngterm-wpe` too outside Linux;
  default features), or of the `-p` crates' alone, with test code out of the denominator (`#[cfg(test)]` and `#[test]`
  items, the module files they declare and test files by name, read from the sources by
  `conformance/agterm_conformance/rust_test_code.py`) and `vendor/` too.
  It writes `OUT_DIR/summary.md` (lines, regions and functions % per crate), `OUT_DIR/summary-files.md` (lines,
  functions, regions and uncovered lines per file) and `OUT_DIR/lcov.info` (the uncovered lines by number), holding
  only the `-p` crates' files when given; it exits 1 when a test fails, with the report still written. Arguments after
  `--` go to every test binary's harness (for example, `-- --test-threads=1`); the build still includes every selected
  test. The `harness = false` binaries read a bare argument as a name filter and take no `--skip`: a `--skip` there runs
  only the cases its name matches, so their code reads as uncovered.
  The report needs Python 3.11+ (for `tomllib`): `$PYTHON` when set, else the first of `python3`, `python`,
  `python3.13`, `python3.12` and `python3.11` that can import it; the script checks this before the build and exits 2
  when `$PYTHON` cannot import it or no candidate can.
  `check_coverage.py` holds the summary against `scripts/unit-coverage-floor-linux.txt` (`crate lines functions
  regions`, the same 0.5 pt tolerance; floors only rise, toward 90 per crate). The gate is CI's job (linux.yml's
  `unit-coverage`, about an hour); locally, run it to see what a crate's tests leave uncovered while writing them, or
  for a change to the tooling:

  ```sh
  GDK_BACKEND=x11 xvfb-run -a dbus-run-session scripts/unit-coverage.sh target/unit-coverage -p ngterm-core
  ```

  Raise the floors from the job's own reports, never a local run (a desktop runs what the runner skips, such as the
  paths that need a DRM render node): the `rust-linux-unit-coverage` artifacts of two CI runs of one commit, then

  ```sh
  .venv/bin/python conformance/tools/check_coverage.py --floor scripts/unit-coverage-floor-linux.txt --add \
    --summary A/summary.md --summary B/summary.md
  ```
- Lint rules: `[workspace.lints]` in `Cargo.toml` (clippy `all` + `pedantic`, the complexity and robustness lints
  denied) with the thresholds in `clippy.toml`; the only exception is a reasoned outer `#[expect]` on one item.

### Git hooks

`.githooks/` holds the repository's Git hooks; `git config core.hooksPath .githooks` turns them on once per clone, for
every worktree of it. The pre-commit hook refuses a commit whose staged Rust `cargo fmt` would change, prints the diff
and the fix, and leaves everything else alone; its header says what it checks, and `scripts/test-pre-commit-hook.sh`
(run by `scripts/lint.sh`) tests it.

## CI

Every workflow that runs on a push to main runs on a push to a `ci/**` branch too, under the same path filters, so a
`ci/<task>` run is the check main's run will be. Main keeps what a run leaves behind: only its runs publish
(`zig-packages.yml`, `release-wpe-cache.yml`) and, `ios.yml` aside, save the cargo and vendor caches, which a branch's
and a PR's runs only read, as theirs would crowd main's out of the repository's cache space. A push cancels the older
run of the same workflow on the same `ci/**` branch and no other branch's. A branch made at a commit another branch
already holds changes no file, so the path-filtered workflows don't start on it.

- `release.yml`: tag-triggered and manual release packaging, with Linux x86_64/aarch64, Windows x86_64, and macOS
  Intel/Apple Silicon builds; see [Downloads and releases](#downloads-and-releases).
- `zig-packages.yml` (pushes that change libghostty-vt's pins, patches or preparation, and by hand on main): keeps a
  mirror of the Zig packages libghostty-vt's build needs and of Ghostty's theme archive, as a `zig-packages-<key>`
  prerelease, so preparing libghostty-vt doesn't depend on those upstream hosts being up. It builds with no write
  access; a separate job, which runs nothing it built, publishes, on main only (a `ci/**` run stops after the
  build). `scripts/build-ghostty-vt.sh` reads the mirror first
  and checks each package's hash; a key with no mirror falls back to the upstreams. Nothing pins these releases, so
  deleting one is safe.
- `release-wpe-cache.yml` (pushes that change the WPE build's inputs or its store, and by hand on main): keeps
  `release.yml`'s portable WPE WebKit prefix, for both Linux arches, in the public GHCR package
  `ghcr.io/<owner>/ngterm-release-wpe`, one tarball per runner image and `scripts/build-release-wpe.sh --print-key`
  (`scripts/release-wpe-store.sh`). It builds only a missing one, with no write access to the package; a separate job
  pushes it, attests its provenance and only then tags it, and never rewrites a tag; a `ci/**` run stops after the
  build, so a key that is new there is built again on main. `release.yml`'s `linux` job
  unpacks it only after `gh attestation verify` ties it to this workflow on main (a failed check fails the release);
  with none for its key, or a store it can't read (a failed login, an auth or registry error), it warns and builds
  WPE from source (hours). A package version outlives an actions/cache
  entry, which a tag couldn't read anyway.
- `linux.yml`: `linux` (format, clippy, the lint report into the job summary, `cargo test` with node for the
  OpenCode plugin's tests, all on the workspace without the WPE crates and the app without its HTML overlays), `wpe`
  (clippy, tests and the bindings' regeneration check of the WPE crates on Arch's `wpewebkit`, then the app with its
  overlays, see [WPE WebKit](#wpe-webkit)), then the staged build (HTML overlays on) under the suite in the `wpe`
  job's Arch container, on its lavapipe, held by `rust-passing.txt`, once per login shell (`conformance`, a matrix:
  bash, fish and zsh), each with `--must-pass live-any-shell`, `--must-pass agent-chat` and `--must-pass agterm-peer`
  (OpenSSH in the container for its loopback `sshd`), the zsh and fish ones
  with `--must-pass live-sessions` and `--must-pass pages-survive`
  too; the bash one runs the app's process tests with the overlays on first, and a `coverage` entry runs the suite on
  an instrumented build, judged by `conformance/coverage/e2e-floor-desktop-linux.txt` only; `unit-coverage`, the workspace's
  `cargo test` under cargo-llvm-cov in the `wpe` job's Arch container (`scripts/unit-coverage.sh`), judged by
  `scripts/unit-coverage-floor-linux.txt`; and `headless-e2e`, the headless app's e2e on an instrumented build
  (`scripts/test-mobile-e2e.sh --coverage`, behind the runner's `sshd`), judged by
  `conformance/coverage/e2e-floor-headless-linux.txt`.
- `cross-platform.yml`: the crates that build on all three OSes (`ngterm-session`, `ngterm-proto`, `ngterm-host`,
  `ngterm-core`, `ngterm-ghostty-vt-sys`, `ngterm-ghostty-vt`, `ngterm-pty`, `ngterm-term`, `ngterm-server`,
  `ngterm-ctl`, `ngterm-app`, `ngterm-headless`, `ngterm-mobile` and `ngterm-update` so far), on Ubuntu, macOS and Windows runners: the libghostty-vt
  prefix with each runner's pinned Zig (cached with Zig's global cache), format,
  clippy and `cargo test` (`crates`, a matrix), then the sys crate cross-built for the Zig targets no runner runs
  natively (aarch64 Linux and x86_64 macOS on Ubuntu, aarch64 Windows on Windows). The macOS leg also checks the frontend with and without WKWebView HTML overlays. The Windows leg also runs the
  vendored portable-pty's Windows patch tests and builds the app: `ngterm-gpui`'s clippy and tests without and with
  its HTML overlays (WebView2, which the runner image carries). Beside it, `windows`: the harness's Windows
  self-tests and the suite against the staged release build (`conformance/tools/run-windows.sh`, with `--must-pass
  live-any-shell`, as the runner's login shell is Windows PowerShell, `--must-pass agent-chat` and `--must-pass
  agterm-peer`), held by `rust-passing-windows.txt`; the report
  and the stage are kept as artifacts; and `coverage-windows`, the suite on an instrumented build, judged by
  `conformance/coverage/e2e-floor-desktop-windows.txt` only.
- `android.yml` (path-filtered to the mobile core, the crates under it, `mobile/android/` and the Android scripts and
  pins): `app` sets up the pinned toolchain (`scripts/setup-android.sh`), runs
  `scripts/build-android.sh check assembleDebug` (Spotless, detekt, Android Lint, the JVM unit and Roborazzi screenshot
  tests) and keeps the debug APK as the `ngterm-android-debug` artifact (the mobile core's e2e runs in `linux.yml`'s
  `headless-e2e`); `emulator` runs the instrumented tests on the phone and tablet AVDs (KVM), where the app's
  real-machine e2e tests (`e2e` package, `scripts/test-android-e2e.sh`) skip, having no machine.
- `ios.yml` (path-filtered to the mobile core, the crates under it, `mobile/ios/`, the shared fonts and the iOS
  scripts and pins), on the `xcode-27` image, three jobs side by side: `lint` (the core's Rust
  tests in the phone simulator, then `scripts/build-ios.sh lint`), `test` (`build-ios.sh test`, keeping the result
  bundles and the failed snapshots' images when it fails) and `archive` (`build-ios.sh archive`, unsigned).
- `ios-release.yml`: a release tag's signed iOS build, uploaded to TestFlight, and dry runs by hand or from `ci/**`
  pushes ([docs/ios-release.md](ios-release.md)).
- `windows-installer.yml` (the same triggers, and by hand): the release build staged, the MSI built from it with the
  pinned WiX and one of the next patch version, then `scripts/check-msi.ps1` installs, checks, upgrades and
  uninstalls it on the runner; the MSI is kept as the `ngterm-msi` artifact.
- `lint.yml` (path-filtered to the workflows, the scripts, the Git hooks, their linters' settings and
  `packaging/release/pins.env`): `scripts/lint.sh`, which runs actionlint over the workflows
  and shellcheck over `scripts/*.sh`, `scripts/lib/*.sh`, `.githooks/*` and every `run:` block, both pinned and
  checksum-verified, then the scripts' self-tests and the pre-commit hook's test. Run it locally the same way.
- `conformance-gate.yml`: the suite's self-tests and catalog coverage gate.
- `reference-macos.yml` (PRs that change `conformance/`, and by hand): the suite against upstream's macOS release.

## Layout

- `crates/`: the Cargo workspace (crate map in [AGENTS.md](../AGENTS.md)).
- `conformance/`: the suite, its catalog of upstream's commands, the ratchet lists `rust-passing.txt` and
  `rust-passing-windows.txt`; `coverage/`: the apps' programs for the e2e coverage report (`apps.toml`) and their
  e2e floors (`e2e-floor-<app>-<platform>.txt`).
- `build/`: `pins.env`, the ghostty-vt, gir, calloop, GPUI and portable-pty patches, the bundled skill's patches over
  upstream's.
- `vendor/`: the patched GPUI, calloop and portable-pty crates, recreated by the vendor scripts (an edit there goes back into its
  patch).
- `.githooks/`: the repository's Git hooks ([Git hooks](#git-hooks)).
- `scripts/`: native builds, staging, `run-gpui-dev.sh`, the lint report, the Linux web engine runtime's packaging, the
  AppArmor profile installer, the Windows installer's build and check, the Windows smoke checks.
- `mobile/android`, `mobile/ios`: the mobile apps over `crates/ngterm-mobile`, each with its `DESIGN.md`.
- `packaging/linux/apparmor/`: ngterm's AppArmor profile (WebKit's sandbox on Ubuntu).
- `packaging/windows/`: the Windows installer's WiX source.
- `resources/`: the agent-status hooks and agent skill the app installs (the skill at `resources/skills/ngterm`,
  upstream's `plugins/agterm/skills/agterm` plus `build/patches/skill-ngterm`), and ngterm's PowerShell integration
  (`shell-integration/pwsh`), which live sessions load; the app icon in every form (`icons/`, its README says where
  each goes).
- `docs/plans/`: implementation plans.
- `.upstream/agterm`, `.upstream/herdr`: read-only reference clones, git-ignored through `.git/info/exclude`. When
  missing: `git clone --depth 1 --branch v0.41.1 https://github.com/umputun/agterm .upstream/agterm` (the parity tag;
  the sync skill moves it), and `HERDR_REPO` checked out at `HERDR_REV` from `build/pins.env`.
