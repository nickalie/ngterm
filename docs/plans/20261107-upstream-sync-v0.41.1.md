# Upstream sync v0.40.0 -> v0.41.1

## Overview

- Bring ngterm from upstream `umputun/agterm` v0.40.0 to v0.41.1 (75 commits, releases v0.41.0 and v0.41.1,
  published 2026-11-06 and 2026-11-07). The mechanical part is on branch `upstream-sync-v0.41.1`: commit `80568dc5`
  regenerates the catalog and ui ids from v0.41.1 (commands `session.overlay.follow`, `session.overlay.search`,
  `session.overlay.changes`, `sidebar.density`; tree fields `sidebarDensity`, `snapshotUnsaved`; their four CLI
  leaves; ui id `settings-sidebar-density`), moves the parity target, the macOS reference level and the
  committed-catalog snapshots; commit `2bb467df` fixes the sync skill's `tag-mentions.sh` so the parity-claim lines
  stay out of the bump. Left out: the Rust claim (Task 24), the bundled skill (its first patch conflicts, Task 18),
  the Markdown renderer resources (new `text.js`/`text.css` need the sync script's rename, Task 2).
- This plan ports: the new wire/CLI shapes and their coverage (1); renderer resources and Markdown frontmatter (2);
  text pages, `path:line` links and the reveal chord (3); sentence punctuation after a path (4); clicked HTML
  documents (5); a click replacing a hidden clicked page (6); log pages in the model and control (7) and in the app
  (8); text-page search and Find routing (9); diff-page changes (10); content pages on a presenter (11); five status
  shapes (12); `snapshotUnsaved` (13); the Live shell from ghostty `command` (14); the quit-capture regression test
  (15); sidebar density (16); tint palette, hex tints and the Show On memory (17); the bundled skill (18); the
  Ghostty v1.4.0 review (19); workspace chevrons (20); palette Move Session (21); title admission and the OS title
  throttle (22); the layout lock over HTML pages (23); the v0.41.1 claim (24).
- Not ported: cookbook recipes and their author lines, upstream CI/watchdog/teardown scripts, CLAUDE.md, release
  metadata, upstream's Ghostty patch, `site/index.html`, `Package.swift` test-harness change, upstream design notes.

## Context

- Upstream: `.upstream/agterm` at v0.41.1 (`911345a`); diff `git -C .upstream/agterm diff v0.40.0 v0.41.1`.
  Upstream design notes worth reading for intent: `docs/plans/completed/20261102-text-file-link-overlay.md`,
  `20261104-remote-content-pages.md`, `20261105-log-follow-page.md` (in the upstream clone). Changes:
  - #874 (`9b8926a`, `b7884db`): a Cmd-clicked source/config/text/log file or `path:line` location opens a read-only
    highlighted text page; reveal moves to Cmd-Option-click; `session overlay open --text FILE [--line N]`.
  - #881 (`1ff85c6`): a path ending a sentence (`/tmp/review.md.`) opens without the punctuation.
  - #880 (`e608fd5`): a Cmd-click over a hidden page an earlier click opened replaces it; `htmlOverlays[].clicked`.
  - #917 (`0011d6c`): a clicked `.html`/`.htm` opens as a script-free, network-free document page;
    `htmlOverlays[].document`.
  - #897 (`2aeca70`): Markdown YAML frontmatter renders as a YAML code block.
  - #899 (`1156557`, `113db14`, `f91d14a`, `97de95c`, `e9e0c43`, `02ed526`, `082278b`, `dfbf1f3` + docs): log pages
    tail and follow the file (`--tail`, `--follow-file`, `session overlay follow`); text pages search
    (`session overlay search`); Find goes to the cover on top.
  - #916, #921 (`424fdd6`, `2caee70`, `669167a`): diff pages step between changes (`session overlay changes`,
    `text.changes`) and color each file's code by language over row bands.
  - #891 (`78cf140`): image, Markdown and text pages on a presented session show on the presenter (`page.content`).
  - #898 (`bb1a531`): triangle-down, play, seal, bolt, bell status shapes; older viewers get only shapes they
    decode (`status.shapes`).
  - #900 (`51bb8bc`, `9dc7fda`): `tree.snapshotUnsaved` while a window's state saves keep failing.
  - #895 (`2718ce5`): Live panes start the zsh/fish named in ghostty `command`.
  - #918 (`5cc5b22`): the quit-time Live listing gets 300 ms.
  - #884 (`ffae3b8`): compact sidebar density, `sidebar density`, `tree.sidebarDensity`.
  - #890, #910 (`0241973`, `d7d8378`): five more tint colors, `#rrggbb` hex tints, a "Custom #rrggbb" menu entry;
    a new tint starts on the last Show On pick.
  - #909 (`aa1201e`): a Workspace chevrons setting; Collapse/Expand Workspace in the workspace row menu.
  - #879 (`4a8b690`): palette Move Session rows act on the live selection.
  - #911, #817 (`87b4408`, `04e082b`): one queued title per pane; the OS window title coalesced to 2 Hz.
  - #904 (`3799efb`): the keyboard-layout lock holds while an HTML overlay page has the keyboard.
  - #912 (`8c9d782`): Attach Remote enabled from launch. #883 (`6511255`): upstream's Ghostty pin to v1.4.0.
- Baseline after the bump (scoped to what it touched):
  - `cargo test -p ngterm-proto --test catalog`: 8 passed, 2 expected failures
    (`command_wire_names_equal_catalog_commands`, `every_catalog_command_parses_to_its_own_name`: the 4 new
    commands). Task 1 fixes them.
  - `cargo test -p ngterm-ctl the_command_tree_is_the_catalogs_cli`: expected failure (the 4 new CLI leaves). Task 1.
  - `conformance/selftests`: 1152 passed, 34 skipped; the one failure is
    `test_coverage.py::test_committed_catalog_passes_the_ci_gate`, the coverage gate's 11 uncovered keys (the 4
    commands, 4 CLI leaves, `field:tree.sidebarDensity`, `field:tree.snapshotUnsaved`, `ui:settings-sidebar-density`).
    Task 1 covers them.
  - No runtime code changed, so no ratchet id can move; the ratchet run is CI's.
  - `scripts/sync-markdown-resources.py --check` fails (render.js, page.css drift; text.js, text.css missing). Task 2.
- Already in the Rust build:
  - #912 Attach Remote at launch: the core reads `remotes.conf` when built and `crates/ngterm-gpui/src/app.rs:158`
    loads remotes before any window; the key monitor consumes the chord even for a disabled item
    (`crates/ngterm-gpui/src/keymap_dispatch.rs:414-422`); tests
    `commands/tests/attach_remote.rs::the_menu_row_follows_the_file_once_it_is_read_again`,
    `keymap/linux.rs::attach_remote_ships_on_ctrl_shift_r_and_can_be_remapped`.
  - #895's ghostty include handling (CRLF, queued includes after every source, cycles, empty `config-file`, `?`,
    quotes, `~/`, symlinked includes): `crates/ngterm-core/src/ghostty_config/parse.rs:109,170,176,248,380,393` with
    tests in `ghostty_config/tests.rs`; two upstream cases lack a Rust test (Task 14).
  - #817's visible title width: the titlebar label sits in a `flex_1`/`min_w(0)` slot
    (`crates/ngterm-gpui/src/titlebar.rs:736-742`).
  - #918: no 100 ms per-call cap here; the quit capture reads leaders through one in-memory `SessionHost::list()`
    under `EXIT_CAPTURE_BUDGET` (500 ms, `commands/relaunch.rs:42`); a failed refresh keeps the cached leaders
    (`commands/live/launch.rs:44-46`). Task 15 pins it with a test.
  - e1b53a3 (workspace badge totals its sessions): `crates/ngterm-gpui/src/sidebar/rows.rs:240-252`, test
    `badges_count_unseen_notifications_when_enabled`. baafe4a is an AppKit doc comment: no counterpart.
  - #890's macOS 27 swatch fix: GPUI draws the swatches (`sidebar/menu.rs:423`), no NSMenuItem image.
- Upstream's pins: `ZMX_REV` unchanged (`8bab1f0`); zmx patches unchanged. `GHOSTTY_REV` 683d8db -> 83fb956
  (v1.4.0); `scripts/ghostty-patches/0001-drain-mailbox-during-surface-teardown.patch` changed. Task 19 records why
  neither needs a port and files the separate `LIBGHOSTTY_VT_REV` evaluation.
- Vendored copies: agent-status payload, ghostty defaults and upstream license unchanged.
  `build/patches/skill-ngterm/0001-live-group.patch` fails at `SKILL.md:160` (Task 18). Markdown resources: Task 2.
  The integrations header (`crates/ngterm-app/src/integrations/mod.rs:15`) stays at v0.40.0 until Task 18.

## Approach

- Each task follows AGENTS.md > Porting a command group: upstream's Swift sources and tests hold the exact strings.
  Read the task's upstream commits with `git -C .upstream/agterm show <sha>`.
- Behavior that upstream builds on a macOS API gets a form for macOS, Linux and Windows (AGENTS.md > Platforms), or
  a recorded reason why an OS has no counterpart. Cmd in upstream's chords is Ctrl (or Super where the keymap says
  so) on Linux and Windows; Finder is the platform file manager (`cx.reveal_path`).
- Upstream strings that name "another Mac" or "Finder" stay verbatim (the error-string rule).
- New conformance cases carry `since("0.41.0")` (v0.41.0 behavior) or `since("0.41.1")` (#916, #917, #921, #910,
  #911, #817, #918). They stay skipped on Rust until Task 24 raises the claim: do not count a skipped case as
  verification or add it to the ratchet before then. Feature tasks validate through Rust unit/integration tests
  and the cases executable at the current level; Task 24 unlocks, runs and ratchets the rest.
- A listed (ratcheted) test never gets a whole-test `since` and never gains assertions on behavior a later task
  builds: a new case goes into a new test, or into a listed parametrized test as
  `pytest.param(..., marks=pytest.mark.since(...))`, which makes a new, unlisted id. This keeps the ratchet green at
  every task boundary, Task 1's skeletons included.
- An existing ratcheted test whose upstream string changes (the one-source open refusal, the tint list, the shape
  list) changes in the same commit as the Rust string, so the ratchet stays green; a renamed parametrize id
  replaces its old line in `conformance/rust-passing.txt` in that commit.
- The bundled skill (`resources/skills/ngterm/`) is regenerated whole in Task 18; feature tasks do not edit it.
- Checks per task follow AGENTS.md > Choosing checks. A new conformance test must also pass on the macOS reference
  (`reference-macos.yml`), which the user runs.

## Implementation Steps

### Task 1: Register the new wire and CLI shapes and cover the catalog

**Upstream:** `agtermCore/Sources/agtermCore/{ControlProtocol,ControlProjection,ControlModes,ControlDispatcher}.swift`;
`agtermCore/Sources/agtermctlKit/{Session+OverlayFollow,Session+OverlaySearch,Session+OverlayChanges,MiscCommands}.swift`;
`site/commands.html` (`session overlay follow|search|changes`, `sidebar density`, the tree card).
**Swift tests:** `ControlProtocolSidebarTests.swift`, `ControlProtocolTests.swift`
(`controlTreeSnapshotUnsavedRoundTripsAndOmitsWhenNil`), `OverlayCommandsTests.swift`, `SidebarCommandsTests.swift`.
**Rust side:** modify `crates/ngterm-proto/src/{command,request,tree}.rs`, `crates/ngterm-core/src/dispatch.rs`,
`crates/ngterm-ctl/src/{main,session_overlay,sidebar}.rs`; conformance files named in Tasks 7-10, 13, 16.
**Conformance:** the 11 uncovered keys: `session.overlay.follow`, `session.overlay.search`,
`session.overlay.changes`, `sidebar.density`, `cli:session overlay follow|search|changes`, `cli:sidebar density`,
`field:tree.sidebarDensity`, `field:tree.snapshotUnsaved`, `ui:settings-sidebar-density`.

- [x] Register `session.overlay.follow`, `session.overlay.search`, `session.overlay.changes` and `sidebar.density`
  with their request args (`mode`, `text`, `to`) and the dispatcher's established not-implemented refusal until
  their tasks land. Add `tree.sidebarDensity` (after `sidebarFlaggedLayout`, before `flaggedOrder`) and
  `tree.snapshotUnsaved` (after `indexUnsaved`) as optional fields, omitted while unset; update the key-order tests
  (`crates/ngterm-core/src/projection.rs` ~757, `crates/ngterm-proto/src/tree.rs` ~714).
- [x] Add the CLI leaves `session overlay follow on|off|toggle [--pane]` (the mode is a required argument, as in
  upstream's `Session+OverlayFollow.swift`; only the wire request defaults an absent mode to toggle), `session overlay search [TEXT]
  [--next|--prev|--close] [--pane]`, `session overlay changes --next|--prev [--pane]`, `sidebar density
  [default|compact|toggle]`, with upstream's help and their parse-time refusals (Tasks 7, 9, 10, 16 list the
  strings); no success stubs.
- [x] Write the reference conformance tests that cover these keys (Tasks 7, 9, 10, 13, 16 give the cases and ids),
  `since`-marked, each asserting its named behavior or field, not merely labelled as coverage. Those tasks then
  make the tests Task 1 wrote pass on Rust and add the rest of their cases. The ui id gets
  `tests/ui/test_settings.py::test_sidebar_density_picker_persists` (Task 16).
- [x] Both catalog cargo tests, the host-free selftests and the coverage gate pass; existing ratchet ids unchanged.

### Task 2: Renderer resources and Markdown frontmatter (#897)

**Upstream:** `agterm/Resources/markdown/{render.js,page.css,text.js,text.css}`,
`agtermCore/Sources/agtermCore/MarkdownPage.swift` (holder `<pre>` -> `<div>`).
**Swift tests:** `agtermTests/MarkdownPageRenderingTests.swift` (`testLeadingFrontmatterRendersAsAYamlBlock`,
`testARuleAfterALeadingBlankLineIsNotFrontmatter`).
**Rust side:** modify `scripts/sync-markdown-resources.py`, `resources/markdown/*` (through the script only),
`crates/ngterm-core/src/html/markdown_page.rs` (line ~115) and its tests.
**Conformance:** existing `tests/ui/test_html_overlay.py::test_a_markdown_file_renders_and_the_tree_names_its_source`
(listed); none new (the rendered DOM is not reachable from the suite).

- [x] Extend the script's agterm -> ngterm rename set to `text.js` and `text.css` (classes, ids, functions, message
  handler names), then run it; `--check` passes. `text.js` and `text.css` are vendored but unused until Task 3.
- [x] `markdown_page::html` uses `<div id="ngterm-markdown" hidden ...>` so a leading blank line survives.
- [x] write tests: the holder keeps a leading "\n" and is a `div`; manual check on a `SKILL.md`: frontmatter as a
  wrapping YAML block, a later `---` as one rule; a file starting "\n---\n## H" shows no frontmatter block.
  (The manual check is automated: `ngterm-wpe`'s headless case
  `markdown_shows_leading_frontmatter_as_a_wrapping_yaml_block` ports upstream's two rendering tests.)

### Task 3: Text pages, `path:line` links and the reveal chord (#874, b7884db)

**Upstream:** `agtermCore/Sources/agtermCore/{TextPage,LinkPolicy,HtmlOverlay,ControlDispatcher+Overlay,ControlDispatcher+Companions,ControlProtocol,ControlProjection,AppStore+HtmlOverlay,AppStore+RemotePage}.swift`,
`agtermctlKit/SessionCommands.swift`, `agterm/Control/ControlServer+{TextPage,SessionActions}.swift`,
`agterm/Views/{TextProbe,MarkdownAssets,HtmlOverlayRegistry,HtmlOverlayView}.swift`,
`agterm/Ghostty/{LinkOpener,GhosttySurfaceView+Input,HudLinkClick,GhosttyCallbacks}.swift`; contract
`site/commands.html` (`session overlay open --text`, tree `text`), `.claude/rules/control-api.md` (link and text-page
bullets).
**Swift tests:** `TextPageTests.swift`, `LinkPolicyTests.swift` (`eligibleLocalFilesAreTextPages`,
`aTextFileFromAnotherOriginIsRevealed`, `anAbsoluteLocationNamesTheLiteralPathThenTheFileAtItsLine`,
`aRelativeLocationJoinsThePaneDirectoryBeforeAnySchemeParse`, `aRelativePathUnderAMissingOrRefusedDirectoryOffersNothing`,
`aRelativePathEscapingIntoAnAutomountRootOffersNothing`, `refusedAbsolutePathsOfferNothingToStat`,
`aRemotePaneOffersNothingToStat`, `aFoundLocationOpensAsTextAtItsLineFromAPaneOrTheScratch`,
`noFoundCandidateKeepsTodaysRoute`, `theChordDecidesTheClickIntent`, `theRevealChordWorksInEveryModeAndOrigin`, ...),
`ControlDispatcherOverlayTests.swift` (`textOpenRejectsInvalidInputsBeforeCallingActions`,
`textOpenRoutesTheStandardizedPathAndItsLine`, `pageCompanionRefusesATextFile`), `HtmlOverlayTests.swift`
(`textPagePolicyAllowsOnlyItsOwnBlankDocument`, `aTextPageIsNamedByItsFileNeverTheTitle`),
`AppStoreTreeProjectionTests.swift` (`aTextPageProjectsItsFileMappedLanguageAndRequestedLine`),
`OverlayCommandsTests.swift` (`textOpenSendsTheResolvedPathLiterallyAndItsLine`), `agtermTests/TextProbeTests.swift`,
`TextPageRenderingTests.swift`, `ControlServerLinkOverlayTests.swift`, `GhosttySurfaceViewInputTests.swift`,
`agtermUITests/ControlHtmlOverlayUITests.swift` (`testATextFileRendersAtItsLineAndTheTreeNamesItsSource`).
**Rust side:** create `crates/ngterm-core/src/html/text_page.rs` (+ tests); modify
`crates/ngterm-core/src/html/{mod,navigation}.rs`, `link_policy.rs`, `commands/{overlay,overlay_page,companion_page}.rs`,
`remote/{page,companions}.rs` (Text local-only for now), `projection.rs`; `crates/ngterm-proto/src/{tree,request}.rs`;
`crates/ngterm-app/src/documents.rs`; `crates/ngterm-ctl/src/session_overlay.rs`;
`crates/ngterm-gpui/src/html_overlay/` (documents, page, registry, policy, webview2/*, wkwebview), `state.rs`,
`terminal/{mouse,links}.rs`, `terminal.rs`, `pane.rs`, `hud.rs`, `quick.rs`, `overlay.rs`.
**Conformance:** existing `tests/protocol/test_document_pages.py::test_document_open_refusals[image]`,
`[markdownFile]`, `tests/cli/test_document_pages_cli.py::test_cli_document_sources_reject_incompatible_flags_before_connecting[--image]`,
`[--markdown]` (listed; their one-source string changes here), `test_local_document_pages_are_not_remote_pages`
(listed); new below.

- [x] `HtmlSource::Text { path, line }`: themed, no bridge, file grant, links or navigation; its policy arm runs ahead
  of the shared exits and allows only a main-frame `about:blank`. Browser hand-off refuses "a text page opens only
  in its overlay"; Finder "show in Finder requires a file page". Local-only until Task 11.
- [x] `text_page`: upstream's extension map, plain extensions (txt text log out csv tsv env lock), basename languages
  and plain basenames; Markdown, HTML and SVG are not text pages. Validate refuses NUL and invalid UTF-8;
  `MAX_BYTES` 4 MiB, highlight cap 256 KiB (over it `data-language` is dropped); the text sits in a `div` holder;
  CSP `default-src 'none'; style-src 'unsafe-inline'`.
- [x] `session.overlay.open` `textFile`/`line`, in upstream's order: `line` without `textFile` ->
  "session.overlay.open: --line requires --text" (first); the one-source refusal becomes "session.overlay.open
  takes one of a command, --html, --url, --image, --markdown or --text"; options -> "session.overlay.open: --text
  takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse"; "session.overlay.open: --text
  must be an absolute path"; "session.overlay.open: --line must be 1 or more"; "session.overlay.open: --text must be a
  source, config, text or log file"; then the probe: "session.overlay.open: unreadable text file: P", "... text file
  too large (over 4 MB): P", "... not a UTF-8 text file: P". `session.companion.page` with `textFile` -> "a page
  companion needs --html or --url".
- [x] Tree `htmlOverlays[].text = {file, language?, line?}` after `markdown`; `language` is the mapped one even past
  the highlight cap; `line` is the requested line.
- [x] `ngtermctl session overlay open --text FILE [--line N]`: FILE resolved to an absolute path and never parsed as
  `path:line`; "provide exactly one of COMMAND, --html, --url, --image, --markdown or --text", "--line requires
  --text", "--line must be 1 or more", "--text takes no --companion, --block, --wait, --navigation, --js,
  --chromeless, --persistent, --browse or --cwd"; help from upstream.
- [x] Link routing: an eligible local path from a pane or the scratch -> `Route::Text` at the mode's size (95 in
  browser mode); from a remote pane, HUD, program overlay or quick terminal, or on a refused/unreadable/binary
  open -> reveal. `link_policy::locations(raw, origin, cwd)`: the literal path, then the path without its
  `:line[:column]`; a relative link joins the pane's cwd before any scheme parse; every candidate passes the
  guarded-path checks before a stat; the frontend stats on a worker; none found keeps the typed route. This
  replaces `detected_relative_path` (`crates/ngterm-gpui/src/state.rs:42-61`). On Windows a drive colon stays.
- [x] Click intent: Ctrl/Super+Shift -> browser (today's rule); Ctrl/Super+Alt (+Shift under mouse capture) ->
  reveal the route's local file from any origin, HUD included, while a web link keeps its web route; a dragged
  reveal reveals nothing and keeps the block selection. Name the AltGr overlap on Windows and desktop grabs of
  Super+Alt on Linux in the module docs.
- [x] Renderer: `text_script` = highlight + `text.js`, `text_style` = page.css + `text.css`, in the isolated world on
  each backend (WKContentWorld, WPE named world, the CDP isolated world on WebView2); the bar stays pinned to the
  visible width (b7884db). Until Task 8's channel, text.js page->host posts are swallowed (add the WebView2
  `window.webkit` shim if needed).
- [x] write tests: Rust unit tests porting the Swift cases above (language map, eligibility, validation, inert
  escaping with a combining mark, leading blank line kept; every listed LinkPolicy case and the chord table; the
  dispatcher refusal table and round trip with `"textFile":"/tmp/a:12.go","line":7`; projection; ngterm-app probes:
  missing, FIFO, directory, oversize, NUL, non-UTF-8; navigation policy; gpui state: `path:line` opens at line N, a
  literal colon-named file wins, a missing location keeps its route, reveal never opens, shell metacharacters in the
  cwd stay data; mouse: Alt stripped, a drag keeps the block).
- [x] write tests: conformance (`since("0.41.0")`), updating the listed string tests in this commit:
  `test_document_pages.py::test_document_open_refusals` gains `textFile` (type refusal for x.svg, x.md, x.html and
  "/x/a.go:12"); `::test_text_page_reads_back_its_source` (open main.go at line 3: `text == {file, language: "go",
  line: 3}`, browser/finder refusals, reload keeps the source); `::test_text_open_line_and_file_refusals`;
  `::test_local_document_pages_are_not_remote_pages` gains `("textFile", "x.go")`; `test_document_pages_cli.py`
  gains `--text` and `::test_cli_text_source_is_taken_literally`; `tests/ui/test_html_overlay.py::test_a_text_file_renders_at_its_line_and_the_tree_names_its_source`.
- [x] manual GUI checks: Ctrl-click `./main.go:42:7` opens at the line; Ctrl+Alt-click reveals; Ctrl+Alt-drag
  still selects a block. (Automated instead of a manual run: `state::tests::a_relative_location_opens_its_file_as_a_text_page_at_the_line`,
  `the_reveal_chord_reveals_and_never_opens`, `terminal::input_tests::the_reveal_chord_lights_the_link_and_asks_for_a_reveal`,
  `a_dragged_reveal_chord_keeps_the_block_selection_and_opens_nothing`; the renderer in real WebKit is
  `ngterm-wpe`'s headless text cases.)

### Task 4: A clicked path that ends a sentence (#881)

**Upstream:** `agtermCore/Sources/agtermCore/LinkPolicy.swift` (`trailingPunctuation`, `trimmedSpellings`).
**Swift tests:** `LinkPolicyTests.swift` (`sentencePunctuationAfterAPathNamesTheLiteralThenEachShorterSpelling`,
`aLocationEndingASentenceFindsTheFileAtItsLine`, `atMostThreeTrailingPunctuationCharactersAreTrimmed`,
`aNameOfPunctuationAloneOrOtherTrailingCharactersOffersNoShorterSpelling`, `aTrimmedSpellingInsideARefusedRootIsNeverOffered`,
`aRemotePaneOffersNoShorterSpelling`); `GhosttySurfaceViewInputTests.swift`
(`testAMarkdownPathEndingASentenceOpensTheFileWithoutThePeriod`).
**Rust side:** modify `crates/ngterm-core/src/link_policy.rs`, `crates/ngterm-gpui/src/state/tests.rs`.
**Conformance:** none (clicks are not reachable from the suite).

- [x] `locations` iterates spellings: the literal, then up to three trims of one of `. ? ! ;` from the end, each
  followed by its `:line`-less form; stop before the last component is punctuation only (`/` and `\` separators);
  a comma, closing bracket or trailing colon offers no shorter spelling; every spelling passes the guards; a remote
  pane gets none; no candidate on disk keeps the typed route.
- [x] write tests: the exact upstream lists ("/tmp/review.md." -> [literal, "/tmp/review.md"]; "/a/b.swift:12." ->
  ["/a/b.swift:12.", "/a/b.swift:12", ("/a/b.swift", 12)]; "/a/b.swift.:12"; "/tmp/a.md..." gives 4; "/tmp/a.md...."
  never "/tmp/a.md"; the 10,000-'!' name; no automount or `//` spelling incl. "/tmp/../net/a.md."); a gpui state
  test where the probe sees both spellings and the Markdown page opens.

### Task 5: Clicked HTML files open as a document page (#917, local part)

**Upstream:** `agtermCore/Sources/agtermCore/{DocumentPage,LinkPolicy,HtmlOverlay,ControlProjection,ControlProtocol,AppStore+HtmlOverlay}.swift`,
`agterm/Control/ControlServer+DocumentPage.swift`, `agterm/Views/{DocumentProbe,DocumentRules,HtmlOverlayRegistry,HtmlOverlayView}.swift`,
`agterm/Ghostty/LinkOpener.swift`; contract tree card (`document?` between `markdown?` and `text?`),
`.claude/rules/control-api.md` (document-page bullet).
**Swift tests:** `DocumentPageTests.swift`, `LinkPolicyTests.swift` (`localHtmlPathsOpenAsDocuments`,
`documentOpensOnTheOwningSessionAtItsModeSize`, `sessionlessOriginsRevealDocuments`), `HtmlOverlayTests.swift`
(`documentPagePolicyHandsWebLinksOutAndKeepsEverythingElse`, `aDocumentPageIsNamedByItsFileNeverTheTitle`),
`AppStoreTreeProjectionTests.swift` (`aDocumentPageProjectsOnlyItsDocumentSource`), `DocumentProbeTests.swift`,
`HtmlOverlayRegistryTests.swift` (`testAClickedDocument*`, `testAnUnreadableDocumentReportsAFailure`),
`ControlServerLinkOverlayTests.swift` (`testAClickedHtmlFileOpensAsADocumentAndAnUnreadableOneOpensNothing`).
**Rust side:** create `crates/ngterm-core/src/html/document_page.rs` (+ tests); modify `html/{mod,navigation}.rs`,
`link_policy.rs`, `commands/overlay_page.rs`, `remote/page.rs` (local-only until Task 11), `projection.rs`,
`crates/ngterm-proto/src/tree.rs`, `crates/ngterm-app/src/documents.rs`, `crates/ngterm-gpui/src/state.rs`,
`crates/ngterm-gpui/src/html_overlay/*`, `crates/ngterm-wpe/src` (a wrapper over the existing
`WebKitUserContentFilterStore` bindings: the store functions from `crates/ngterm-wpe-sys/src/lib.rs:3761`, the
`WebKitUserContentFilter` type at ~2814).
**Conformance:** add `"document"` to the absent-field lists of `test_image_page_reads_back_its_source`,
`test_markdown_page_reads_back_its_source` and Task 3's text test (no socket path opens a document).

- [x] `HtmlSource::Document { path }`, from a click only: no file grant, no bridge, JavaScript off, the file page's
  theme, loaded as text with no base URL; identity is the file name. `document_page::page()` validates (no NUL,
  UTF-8, 4 MiB) and inserts `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:;
  style-src 'unsafe-inline'; font-src data:; media-src data:">` right after a leading doctype (only HTML ASCII
  whitespace before it, case-insensitive), otherwise first. Probe strings "unreadable html file: P", "html file
  too large (over 4 MB): P", "not a UTF-8 html file: P".
- [x] Routing: `.html`/`.htm` checked before text eligibility; a pane or the scratch -> `Route::Document` at the
  mode's size; sessionless origins and refusals reveal. Navigation shares the Markdown arm; browser hand-off
  refuses "a clicked html page opens only in its overlay"; Finder "show in Finder requires a file page". Tree
  `htmlOverlays[].document = path`.
- [x] Network blocking before load, or the page fails with "the network filter for a clicked html page could not be
  installed": macOS a WKContentRuleList blocking `^https?://`, `^wss?://`, `^ftps?://`; Linux the same rules through
  WPE's user content filter (verify WPE applies it to preconnect hints); Windows cancel http(s)/ws in the existing
  `WebResourceRequested` filter (`webview2/native.rs:603-645`) plus `x-dns-prefetch-control` off, and close or
  record the preconnect gap (research it in this task; strip `<link rel=preconnect|dns-prefetch|prefetch|prerender>`
  if no engine switch exists). (Linux: verified in real WPE that the filter stops the preconnect hint, which reaches
  a local listener without it. Windows has no per-page engine switch for link hints: every `<link>` is made inert
  instead, `document_page::chromium_page`.)
- [x] write tests: document_page placement and refusals; link_policy cases; navigation; projection; probes; gpui
  state and registry (JS off, no bridge, filter installed before load); manual check with a `<script>`, a remote
  `<img>` and a preconnect on each OS (a local listener sees nothing). (Linux automated instead of a manual run:
  `ngterm-wpe`'s headless `a_clicked_document_loads_nothing_from_the_network_behind_its_filter` and its unfiltered
  control; macOS and Windows not run by hand here: CI builds and tests their code, a device check stays open.)

### Task 6: A click replaces a hidden page an earlier click opened (#880)

**Upstream:** `agtermCore/Sources/agtermCore/{AppStore+ClickedPage,HtmlOverlay,ControlDispatcherOptions,ControlProjection,AppStore+HtmlOverlay}.swift`,
`agterm/Control/ControlServer+{ClickedPage,ImagePage,MarkdownPage,TextPage,SessionActions}.swift`; contract tree card
(`clicked?` after `replica?`), "session overlay hide", "browser links", `.claude/rules/control-api.md` (five click paths).
**Swift tests:** `AppStoreClickedPageTests.swift`, `AppStoreTreeProjectionTests.swift`,
`ControlServerLinkOverlayTests.swift` (`testALinkReplacesAHiddenPageAnEarlierClickOpenedAndShowsIt`,
`testALinkLeavesAHiddenPageNoClickOpened`, `testALinkLeavesAHiddenClickedPageWhenTheSavedStoreIsUnusable`,
`testEachClickedFileKindReplacesAHiddenClickedPage`, `testAClickedFileThatCannotOpenLeavesTheHiddenClickedPage`).
**Rust side:** modify `crates/ngterm-core/src/html/mod.rs`, `commands/overlay_page.rs` (`open_link_overlay`,
`open_document_link` at ~663-711), `projection.rs`, `crates/ngterm-proto/src/tree.rs`, `crates/ngterm-gpui/src/state.rs`;
create `crates/ngterm-core/src/commands/tests/clicked_page.rs`.
**Conformance:** new `tests/protocol/test_overlay.py::test_a_socket_page_is_never_marked_clicked`
(covers `field:tree.session.htmlOverlays`, `session.overlay.hide`).

- [x] `HtmlOverlay.clicked`, set only by the five click paths (web link, image, Markdown, text, document), never by
  the dispatcher nor carried in presentation frames; read back `clicked: true`, omitted otherwise.
- [x] `release_hidden_clicked_page(session)`: close the session-wide page only when the session is overlay-hidden,
  its page is clicked and `page_belongs_remote` is false; the released page's outcome is dismissed. Each click path
  releases only after every check that could still refuse (probe, HUD/zoom recheck, a usable saved store for a
  persistent web page; find ngterm's equivalent of upstream's `persistentStoreFailure`) and rechecks hidden
  before opening. A shown page, a hidden program or a socket page keep today's fallback.
- [x] write tests: the AppStoreClickedPageTests cases incl. the presenter case on the live fakes; projection
  (`true` once, never `false`, old node decodes None); gpui state per file kind and the refusal cases; the
  conformance test above.

### Task 7: Log pages: tail model, `--tail`/`--follow-file`, `session.overlay.follow` (#899)

**Upstream:** `agtermCore/Sources/agtermCore/{LogTail,TextPage,HtmlOverlay,AppStore+HtmlOverlay,ControlDispatcher+Overlay,ControlDispatcher,ControlProtocol,ControlProjection,ControlActions,ControlActionsDefaults,ControlDispatcherOptions,HtmlBridge}.swift`,
`agtermctlKit/{Session+OverlayFollow,SessionCommands}.swift`, `agterm/Control/ControlServer+{TextPage,SessionActions}.swift`,
`agterm/Views/{ImageProbe,TextProbe}.swift`; commits `1156557`, `113db14`, `e9e0c43`, `02ed526`, `dfbf1f3`; contract
`session overlay open --text ... --tail N [--follow-file]`, `session overlay follow`, control-api.md log-page bullet.
**Swift tests:** `LogTailTests.swift`, `TextPageTests.swift` (`onlyLogAndOutFilesAreLogs`,
`aLogPageCarriesItsStateInsteadOfALine`, `aPausedLogPageOmitsTheUnsetFlags`, `aRebuiltPageCarriesOnlyAFiniteAnchor`),
`HtmlOverlayTests.swift` (`aLogPage*`, `followMode*`, `aReport*`, `aPlainTextPageNodeOmitsTheLogFields`),
`ControlDispatcherOverlayTests.swift` (`textOpenCarriesTheLogRequest`, `overlayFollow*`), `ControlProtocolTests.swift`
(`logPageArgumentsRoundTripAndAreOmittedWhenUnset`), `OverlayCommandsTests.swift` (`textOpenSendsTheTailAndFollowFile`,
`follow*`, `openHelpTellsFollowFileFromFollow`), `ControlServerTextPageTests.swift` (log and follow cases),
`ImageProbeTests.swift` (`testATail*`).
**Rust side:** create `crates/ngterm-core/src/html/log_tail.rs`, `crates/ngterm-core/src/commands/tests/overlay_log.rs`;
modify `crates/ngterm-proto/src/{command,request,tree}.rs`, `crates/ngterm-core/src/html/{text_page,mod,bridge}.rs`,
`model/html_overlay.rs`, `commands/overlay.rs`, `dispatch.rs`, `projection.rs`, `model/page_snapshot.rs`,
`crates/ngterm-app/src/documents.rs`, `crates/ngterm-ctl/src/session_overlay.rs`, the gpui `HtmlHost` impl.
**Conformance:** new in `tests/protocol/test_text_pages.py` (create): `test_log_open_refusals`,
`test_log_page_reads_back_its_tail_and_follow_state`, `test_follow_answers_the_state_it_left`, `test_follow_refusals`;
rows in `test_overlay.py::test_overlay_pane_selector_refusals` and `::test_unknown_session_target`;
`tests/cli/test_cli_only.py::test_overlay_follow_and_tail_validation`.

- [x] `LogTail::snapshot(data, offset, max_lines)`: from offset > 0 drop the partial first line (unless nothing
  follows it, then keep the over-long line from its first whole character, marked clipped); hold back an
  unfinished UTF-8 tail; skip a leading NUL hole (copytruncate); keep the last `max_lines`; None for NUL or invalid
  UTF-8 in the kept lines; `start_offset` as upstream.
- [x] Constants `DEFAULT_TAIL` 5000, `MAX_TAIL` 100000, `MAX_TAIL_BYTES` 2 MiB; `is_log` = `.log`/`.out`.
- [x] `HtmlOverlay.text_log {tail, follow}` and `log_state {following, report}`; a follow change emits
  `tree.changed`, a report is stored only on change and never emitted.
- [x] Open validation: a text-only refusal first ("session.overlay.open: --line requires --text",
  "session.overlay.open: --tail and --follow-file require --text"); in the text branch "session.overlay.open: --tail
  and --follow-file cannot be combined with --line", then "session.overlay.open: --tail must be 1...100000". A log
  reads only its tail (no 4 MB refusal); a binary tail is refused as not UTF-8. A log page the placement hands to a
  presenter -> "session.overlay.open: a log page is not shown on another Mac".
- [x] `session.overlay.follow [mode]` (absent = toggle; "invalid overlay follow mode: X"), then pane, then session;
  a replica keeps upstream's replica string; a non-log text page -> "no log page"; answers
  `{id, text: "following"|"paused"}`. Bridge own-overlay commands += follow.
- [x] Tree text node: `file, language, line, tail, following, lines, clipped, waiting, unreadable` (the last six
  omitted on a plain page, present on a log page).
- [x] ngtermctl: `--tail N`, `--follow-file` (help says it is not `--follow`), the three parse-time strings;
  `session overlay follow` with "mode must be on, off or toggle".
- [x] Persisted pages (`model/page_snapshot.rs`, an ngterm extension): persist the request (tail, follow), not the
  live report, and recheck the range on restore. A headless origin's parked log page is refused or closed like the
  presenter case.
- [x] Tail reader (ngterm-app): a `FileStamp` of dev+ino+size+mtime(ns) on Unix and volume serial + file index +
  size + last-write on Windows (`GetFileInformationByHandle`); `read_at`/`seek_read`; non-regular files refused;
  default share modes so a writer's rename rotation is never blocked.
- [x] write tests: log_tail cases; html log attributes; model setters; dispatcher table; wire round trip;
  projection; CLI parse; tail reader (small file, large window + 1 byte, symlink measured at its target,
  FIFO/device refused, unchanged stamp reads nothing, stamp changes on grow/replace); the conformance tests above.
  (The conformance cases carry `since("0.41.0")` and stay skipped on Rust until Task 24; the tail reader's Windows
  branch, `GetFileInformationByHandle` and `seek_read`, is exercised by the same unit tests on the CI Windows job.)

### Task 8: Log pages follow the file in the app; `.log` clicks (#899)

**Upstream:** `agterm/Views/{LogFollower,TextProbe,ImageProbe,HtmlOverlayRegistry,TextPageBridge}.swift`,
`agterm/Resources/markdown/{text.js,text.css}`, `agtermCore/Sources/agtermCore/LinkPolicy.swift` (`opensAsLog`),
`agterm/Control/ControlServer+TextPage.swift`; commits `f91d14a`, `97de95c`, `02ed526`, `dfbf1f3`.
**Swift tests:** `LogFollowerTests.swift` (all), `TextPageRenderingTests.swift` (the log and scroll cases),
`LinkPolicyTests.swift` (`onlyAPlainClickOnALogOpensAsALogPage`), `ControlServerLinkOverlayTests.swift`
(`testAClickedLog*`), `agtermUITests/ControlHtmlOverlayUITests.swift` (`testALogPageFollowsAppendsAndFollowTogglesThroughControl`).
**Rust side:** create `crates/ngterm-app/src/log_follower.rs`, `crates/ngterm-gpui/src/html_overlay/text_page.rs`;
modify `crates/ngterm-app/src/{lib,documents}.rs`, `crates/ngterm-gpui/src/html_overlay/{documents,registry,page}.rs`,
the page->host handler registration (`wkwebview/native.rs`, `webview2/bridge.rs`, `webview2/runtime.rs`,
`crates/ngterm-wpe/src/spec.rs`), `crates/ngterm-core/src/link_policy.rs`, the Task 3 text-link opener.
**Conformance:** new in `test_text_pages.py`: `test_log_page_follows_appends_rotation_and_truncation`,
`test_a_paused_log_page_reads_nothing_until_resumed`, `test_a_log_reads_only_its_tail`.

- [x] `LogFollower` (frontend-neutral, injected reader and interval): while following and on screen, one stamp check
  per second (polling, as upstream: it works on network filesystems and through rotation and copytruncate on all
  three OSes); skip while the page left its slot or a document build is pending; only the newest read lands, never
  across a pause epoch; `shift(old, new)` treats a changed id, a shrunk size or a start offset that moved back as
  replaced, and counts CRLF and lone CR once each.
- [x] Results: a binary end keeps its stamp and sets `unreadable`; a read failure clears the stamp and sets
  `waiting`; a good read sets `{lines, clipped}` and pushes; every report goes to the model without an event; a
  theme rebuild reuses the cached snapshot and paused anchor; an explicit reload reads fresh; highlighting drops
  past the highlight cap.
- [x] The page channel: text.js posts `{ready}` (releases held pushes, resends following) and `{following, top}`
  (a user scroll); a model follow change calls `set_following` (a pause drops the anchor and reads in flight, a
  resume ticks at once). Define "on screen" in GPUI terms and stop the timer when the page closes or leaves the
  screen. Tasks 9 and 10 reuse this channel and its host->page calls with replies.
- [x] Links: a plain click on a `.log`/`.out` opens a following log page (tail 5000) after an off-thread check; a
  `path:line` click opens the static page; a presented session gets the static page (after Task 11).
  (Also fixed: on WPE a text page's own `about:blank` load was judged a subframe's and refused, so no text page
  finished loading on Linux; `policy::navigation_action` judges it the main frame's.)
- [x] text.js/css (ngterm names): the log bar ("Follow: on" / "Follow: paused - Resume", title "Follow or pause (F)",
  info "first line clipped · waiting for file · unreadable data"), no gutter; keys F, G/End, g/Home (none with
  Ctrl/Meta/Alt or in the find field); user-scroll intent within 600 ms or a held pointer; bottom within 4 px.
  (Already in the resources Task 2 synced; nothing to change.)
- [x] write tests: follower tests over a fake reader and a temp dir porting `LogFollowerTests`; `opens_as_log`; the
  conformance tests above; Windows share mode and file id on a CI runner or the VM. (CI `ci/upstream-sync-task8`:
  the Windows tail reader's unit tests and the macOS/Windows html builds green.)

### Task 9: Search in text pages; Find goes to the cover on top (#899)

**Upstream:** `agtermCore/Sources/agtermCore/{AppStore+HtmlOverlay,HtmlOverlay,TextPage,LogTail,ControlDispatcher+Overlay,ControlDispatcher,ControlProtocol,ControlProjection,ControlActions,ControlActionsDefaults,HtmlBridge}.swift`,
`agtermctlKit/Session+OverlaySearch.swift`, `agterm/Control/ControlServer+TextPage.swift`,
`agterm/Views/{TextPageBridge,TerminalSearchBar}.swift`, `agterm/{AppActions+PageSearch,AppActions}.swift`, text.js/css;
commits `082278b`, `dfbf1f3`, `93f2794`; contract `session overlay search`, control-api.md search bullet,
menu-actions.md "What is on top owns Command-F".
**Swift tests:** `HtmlOverlayTests.swift` (search cases), `TextPageTests.swift` (`aPageCarriesItsSearchWithTheQueryEscaped`,
`aClosedSearchCarriesOnlyItsRevision`), `ControlDispatcherOverlayTests.swift` (`overlaySearch*`),
`OverlayCommandsTests.swift` (`search*`), `ControlServerTextPageTests.swift` (search cases),
`AppActionsPageSearchTests.swift`, `TextPageRenderingTests.swift` (query and step cases), `agtermUITests`
(`testCommandFOverAFloatingTextPageSearchesThePageAndLeavesTheTerminalSearch`,
`testCommandFWithAPageOverTheOtherPaneSearchesTheFocusedTerminal`).
**Rust side:** modify `crates/ngterm-proto/src/{command,tree}.rs`, `crates/ngterm-core/src/html/{mod,text_page,bridge}.rs`,
`model/html_overlay.rs`, `commands/overlay.rs` (or create `commands/overlay_text.rs`), `dispatch.rs`, `projection.rs`,
`app_actions.rs` (`toggle_search` at ~595), `crates/ngterm-gpui/src/html_overlay/text_page.rs`, `window.rs`, the
web-view key paths (`webview2/keys.rs`, `wkwebview/keys.rs`, WPE), `crates/ngterm-ctl/src/session_overlay.rs`, text.js/css.
**Conformance:** new in `test_text_pages.py`: `test_text_page_search_sets_steps_and_closes`,
`test_overlay_search_refusals`, `test_a_search_step_pauses_a_following_log_page`; pane-selector and unknown-target
rows; `test_cli_only.py::test_overlay_search_validation`; `tests/ui/test_html_overlay.py::test_find_over_a_text_page_searches_the_page`,
`::test_find_with_a_page_over_the_other_pane_searches_the_terminal`. Existing listed
`tests/ui/test_titlebar.py::test_find_bar_counts_the_matches`, `tests/protocol/test_session_text.py::test_search_counts_the_matches`
stay green.

- [x] Model: `text_search {query, matches, current}` and `search_revision`; a set or close bumps the revision,
  clears the report and emits only on a change; an empty query closes; text sources only; a report lands only for
  the current revision and query and is never emitted.
- [x] Dispatch: exactly one of text/to: "session.overlay.search needs text, --next, --prev or --close",
  "session.overlay.search: text cannot be combined with --next, --prev or --close", "session.overlay.search: text
  must not be empty", "invalid overlay search direction: X"; then pane, session. A presenter's slot -> "remote page:
  search it on the Mac that shows it"; a non-text page -> "no text page"; a step without a query -> "no search
  query: set one first"; before load -> "page not loaded"; a step answers `{id, text: "N of M"}` and pauses a
  following log page. The step is a one-shot request to the frontend with a reply (a `Dispatched::Pending` like
  `browser.clear`, `commands/browser.rs:64`). (`commands/overlay_text.rs`; the frontend answers through
  `HtmlHost::step_text_search`, a renderer that never answers counts as `page not loaded` after 5 s.)
- [x] The page's own field writes the same model; typed revisions never get overwritten by their own pushes; a later
  control query replaces coalesced typing. Page html carries `data-revision` (+ escaped `data-query`/`data-current`).
  Tree `text.search {query, matches, current}`, omitted with no query. Bridge own-overlay += search. (The bridge now
  builds every local text page's documents, static ones included, and a rebuilt static page reopens where it was.)
- [x] Find routing: the quick terminal holding the keys, or a shown page/program overlay over the focused pane (full
  or floating), takes Find first; a text page focuses its field, any other cover swallows it; with no cover the
  scratch, then the focused surface. A HUD or a page over the other pane does not count. The Find chord (Cmd+F on
  macOS, Ctrl+Shift+F on Linux and Windows) reaches the app action while a web view holds focus on each backend.
- [x] ngtermctl `session overlay search` with "text cannot be combined with --next, --prev or --close", "text must
  not be empty" and upstream's other parse strings. (Task 1 landed it with its tests; nothing to change.)
- [x] text.js/css: the find box (placeholder "Find", "Previous match"/"Next match", counter "N of M"/"no matches"),
  `/`, n/N, Return/Shift-Return, Esc; matches across highlight spans; a hostile query stays text. (Already in the
  resources Task 2 synced; nothing to change.)
- [x] write tests: model and dispatcher rows; projection; page attrs; CLI parse; `app_actions` cover routing over the
  fakes; the conformance tests above. (Plus the bridge over a fake channel, the Find chord's routing, and a WPE
  harness case: the renderer counts the document's query and steps it.)

### Task 10: Diff pages step between changes and color by file language (#916, #921)

**Upstream:** `agtermCore/Sources/agtermCore/{TextPage,HtmlOverlay,AppStore+HtmlOverlay,ControlDispatcher+Overlay,ControlDispatcher,ControlProtocol,ControlProjection,ControlActions,ControlActionsDefaults,HtmlBridge}.swift`,
`agtermctlKit/Session+OverlayChanges.swift`, `agterm/Control/ControlServer+TextPage.swift`,
`agterm/Views/TextPageBridge.swift`, text.js/css; commits `424fdd6`, `2caee70`; contract `session overlay changes`,
control-api.md diff bullet.
**Swift tests:** `TextPageTests.swift` (`isDiffFollowsTheExtension`, `onlyAPlainDiffPageIsMarked`,
`aDiffIsMarkedPastTheHighlightCapWithTheChangeItReopensOn`, `aDiffMapsEachHeaderPathToItsLanguage`,
`aCRLFDiffMapsItsHeaderPaths`, `aDiffUnderTheHighlightCapCarriesItsFileLanguages`), `HtmlOverlayTests.swift`
(`aChangesStepAddressesOnlyALocalDiffPage`, `aChangesReportIsKeptClearedAndEmitsNothing`,
`aTextPageNodeReportsItsChangesOnlyOnceReported`), `ControlDispatcherOverlayTests.swift` (`overlayChanges*`),
`OverlayCommandsTests.swift` (`changes*`), `ControlServerTextPageTests.swift` (`testAChangesStepNeedsALoadedDiffPage`),
`TextPageRenderingTests.swift` (diff cases).
**Rust side:** modify `crates/ngterm-proto/src/{command,tree}.rs`, `crates/ngterm-core/src/html/{text_page,mod,bridge}.rs`,
`model/html_overlay.rs`, the Task 9 overlay text module, `dispatch.rs`, `projection.rs`,
`crates/ngterm-gpui/src/html_overlay/text_page.rs`, `crates/ngterm-ctl/src/session_overlay.rs`, text.js/css.
**Conformance:** new in `test_text_pages.py`: `test_diff_page_steps_between_changes`, `test_overlay_changes_refusals`;
pane-selector and unknown-target rows; `test_cli_only.py::test_overlay_changes_validation` (all `since("0.41.1")`).

- [x] `is_diff` (`.diff`/`.patch`); `diff_languages` from `---`/`+++` paths (up to a tab, quotes stripped, known
  languages only); html for a non-log diff: `data-diff` always, `data-change` when > 0, sorted-key `data-files`
  under the highlight cap only.
- [x] `TextChanges {count, current}`: stored on change, never emitted, cleared during a fresh read.
- [x] Dispatch: "session.overlay.changes requires to next or prev", "invalid overlay changes direction: X"; then
  pane, session; a presenter's slot -> "remote page: step it on the Mac that shows it"; not a plain diff page ->
  "no diff page"; before load "page not loaded"; no hunks "no changes"; answers `{id, text: "N of M"}`. Tree
  `text.changes` after `search`, omitted until reported. Bridge own-overlay += changes.
- [x] ngtermctl `session overlay changes --next|--prev`, "give exactly one of --next or --prev". (Task 1 landed it
  with its tests; nothing to change.)
- [x] text.js/css (vendored verbatim, never re-implemented): a change is one contiguous run of `-`/`+` rows inside a
  hunk, and the hunk headers' line counts decide which rows are inside a hunk (so `---`/`+++` within one never
  count; one `@@` hunk can hold several changes, `testADiffPageCountsItsChangesAndStepsWrapBothWays`); each file's sides highlighted in its language over row bands;
  past the cap bands and stepping only; the bar's "Previous change ([)"/"Next change (])" and "change N of M"; `]`/`[`;
  a theme rebuild keeps the change, a reload starts before the first. (Already in the resources Task 2 synced; the
  app side keeps the reported change for a rebuild and resets it after a fresh read.)
- [x] write tests: is_diff/diff_languages (CRLF, quoted paths, unknown language); html marking; model; dispatcher;
  projection; CLI parse; the conformance tests (diffs that fit on screen in the headless window). (Plus the
  pane-selector and unknown-target rows, and the GPUI bridge over a fake channel: the reopened change and a step.)

### Task 11: Image, Markdown, text and document pages on a presenter (#891, #917 remote part)

**Upstream:** `agtermCore/Sources/agtermCore/{PresentationFrames,PresentationHub,AppStore+RemotePage,AppStore+ReplicaPage,AppStore+RemoteOverlay,AppStore+HtmlOverlay,HtmlOverlay,ImagePage,MarkdownPage,RemotePageResources,ControlDispatcherOptions}.swift`,
`agterm/Control/ControlServer+{ImagePage,PageServe,RemotePage,CompanionPage,RemoteCompanions,SessionActions}.swift`,
`agterm/Views/{HtmlOverlayRegistry,HtmlOverlayView,ImageProbe,MarkdownProbe,TextProbe}.swift`; the #917 parts of
`AppStore+{RemotePage,ReplicaPage}`, `PresentationFrames/Hub` (`page.document`), `RemotePageResources`; contract
the presented-session paragraph in commands.html, control-api.md (`page.content`, `presenterSized`/`merge`).
**Swift tests:** `AppStoreRemotePageTests.swift`, `AppStoreReplicaPageTests.swift`, `PresentationFramesTests.swift`,
`RemotePageResourcesTests.swift`, `MarkdownPageTests.swift` (`lexicalPath*`, `remoteReferencesIgnoreSymlinksOnThisMac`),
`ControlServerPageServeTests.swift`, `ControlServerRemotePresentationTests.swift`, `HtmlOverlayRegistryTests.swift`
(replica cases), `MarkdownPageRenderingTests.swift` / `TextPageRenderingTests.swift` (replica cases),
`ImageProbeTests.swift` / `TextProbeTests.swift` (`testBytesInHandAreJudgedLikeAFile`), `MarkdownProbeTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/remote/{frames,hub,client,page,handoff,replica_page,page_serve,companion_page}.rs`,
`model/remote_overlay.rs`, `html/{mod,markdown_page}.rs`, `projection.rs`, `commands/overlay_page.rs`,
`model/page_snapshot.rs`, `crates/ngterm-app/src/documents.rs`, `crates/ngterm-gpui/src/html_overlay/*`,
`crates/ngterm-gpui/src/state.rs`, `crates/ngterm-headless/src` (a probe seam).
**Conformance:** existing listed `tests/protocol/test_live_sessions.py::test_a_presenter_without_page_kinds_keeps_the_page_refusal`,
`::test_a_page_goes_to_the_presenter_and_its_reports_reach_the_tree`, `::test_page_serve_serves_a_page_a_presenter_shows`,
`::test_origin_commands_on_a_remote_page_reach_the_presenter`; new `::test_content_pages_go_only_to_a_presenter_listing_page_content`,
`::test_page_serve_serves_a_content_pages_files_under_its_rules`, `::test_a_point_resize_of_an_auto_sized_image_merges_on_the_presenter`.

- [x] Frames: `PresentationPageSource::{Image{path,width,height}, Markdown{path}, Text{path,line?}, Document{path}}` in
  Swift's synthesized coding, `uses_channel`, `is_content`; `PresentationPageCommand.merge` (omitted when None).
  Hub kinds `page.content` (after `overlay.points`) and the document kind; `viewer_kinds(false)` drops them.
- [x] Origin: probe before reserving; an image travels only with its oriented pixels; without the presenter's kind ->
  `Unsupported`. An auto-sized session-wide image reserves 95% with `presenter_sized`; a point-only resize sends its
  axes with `merge: true`; a percent or full resize ends it. Claim expiry, navigate refusals and the hand-on of
  parked pages treat every channel page alike; a parked content page goes only to a presenter listing the kind.
  Decide on purpose what the `page_snapshot.rs:276` unpresentable sweep does now that images and Markdown map.
- [x] page.serve: an image, text or document page serves only its own file; a Markdown page its own file plus
  pictures in its folder (symlinks resolved on the origin); each under its kind's cap; "file is too large for the
  page", "file cannot be read", "not a png, jpg, gif or webp image", "this page has no bridge".
- [x] Viewer: `HtmlSource::Replica { path, content }` (Html, Image{w,h}, Markdown, Text{line}, Document); the tree
  reports image/markdown/text/document with `replica: true`; a merge resize keeps the axis it does not name.
  Markdown and text replicas themed, image replicas unthemed on a clear canvas, none bridged; a text replica stays
  static (no follow, no control search); a log page click on a presented session opens the static page there.
- [x] Companion attach refuses "a markdown, text or document page companion cannot be shown on another Mac".
- [x] `markdown_page::lexical_path` split out of `local_path`; the viewer lists references lexically.
- [x] write tests: every Swift case listed (frames, hand-over by capability per kind, presenter sizing and merge,
  expiry, resolve rules and caps incl. FIFO and a grown file, lexical_path, replica_source and projection, byte
  probes); the conformance tests above, green on the reference first.

### Task 12: Five more status shapes, gated for older viewers (#898)

**Upstream:** `agtermCore/Sources/agtermCore/{AgentStatus,PresentationHub}.swift`, `agterm/Views/SettingsView.swift`;
contract `session status --shape`, `.claude/rules/notifications.md`.
**Swift tests:** `AgentStatusTests.swift`, `PresentationHubTests.swift` (`oneStatusReachesAViewerWithoutTheShapesKindWithOnlyAShapeItDecodes`,
`aViewerWithoutTheShapesKindGetsTheSnapshotAndAHeldDeltaWithoutANewShape`, `aViewerAdvertisingTheShapesKindGetsTheSnapshotUnchanged`),
`ControlDispatcherPickTests.swift`, `agtermctlKitTests/CommandsTests.swift`, `SettingsUITests.swift`.
**Rust side:** modify `crates/ngterm-core/src/model/status.rs`, `remote/hub.rs`, `crates/ngterm-gpui/src/status_glyph.rs`,
`settings/agent_status.rs`, `crates/ngterm-ctl/src/session_meta.rs`; the mobile apps:
`mobile/android/app/src/main/kotlin/com/nickalie/ngterm/android/ui/theme/Status.kt` + drawables,
`mobile/ios/Packages/NgtermDesign/Sources/NgtermDesign/StatusShape.swift`, both `DESIGN.md` (frontend-design skill).
**Conformance:** listed `tests/protocol/test_session.py::test_status_shape`, `tests/protocol/test_ask_pick.py::test_pick_refusals[status-shape]`
(strings change in this commit); new `test_live_sessions.py::test_a_viewer_without_status_shapes_gets_only_shapes_it_decodes`.

- [x] Order: circle, square, triangle, triangle-down, play, diamond, capsule, seal, star, bolt, bell; display
  "Down Triangle" etc.; `ORIGINAL` (what a viewer without `status.shapes` decodes) is v0.40.0's set, upstream's
  `StatusShape.original`: circle, square, triangle, diamond, capsule, star (not the first six of the new order). "invalid shape: <v> (circle|square|triangle|triangle-down|play|diamond|capsule|seal|star|bolt|bell)",
  "invalid pick item shape (...)", ctl "shape must be one of: circle, square, triangle, triangle-down, play, diamond,
  capsule, seal, star, bolt, bell"; settings accept the new names.
- [x] Hub: advertise `status.shapes`; a subscriber without it gets `shape` cleared for a non-original shape in Status,
  Snapshot and held deltas. Viewer hellos list it only once they draw the shapes (desktop and both mobile apps in
  this change).
- [x] GPUI glyphs for the five shapes at the existing size; the Settings picker shows 11 options from `display_name`.
- [x] write tests: the Swift cases; pick refusal; ctl phrase; glyph coverage per shape; picker labels; the listed
  conformance strings and the new mirror test.

### Task 13: `tree.snapshotUnsaved` (#900)

**Upstream:** `agtermCore/Sources/agtermCore/{AppStore,AppStore+Persistence,ControlProjection}.swift`; contract tree
top-level fields, `.claude/rules/windows.md`.
**Swift tests:** `PersistenceTests.swift` (`aFailedSnapshotSaveShowsInTheTreeUntilASaveLands`,
`aFailedSnapshotSaveMarksOnlyItsOwnWindow`), `ControlProtocolTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/persist/library.rs` (`save_window` ~419-450), `projection.rs`,
`dispatch.rs`, `crates/ngterm-core/tests/persistence.rs`.
**Conformance:** new `tests/protocol/test_tree.py::test_snapshot_unsaved_while_a_windows_file_cannot_be_written`
(covers `field:tree.snapshotUnsaved`); add `snapshotUnsaved` to `::test_fresh_tree_omits_unset_top_level_fields` (listed).

- [x] A failed `windows/<id>.json` write (immediate or debounced) marks that window; the next landed write clears
  it; launch-time writes leave it alone; other windows unaffected; forgotten when the window closes. `tree`
  (scoped to its window) carries `snapshotUnsaved: true` while marked, independent of `indexUnsaved`.
- [x] write tests: the two persistence cases (`indexUnsaved` stays absent); the conformance test (a directory in place
  of the file; check the trick on Windows).

### Task 14: Live panes start the shell named by ghostty `command` (#895)

**Upstream:** `agtermCore/Sources/agtermCore/{ZmxSupport,GhosttyConfigText}.swift`, `agterm/Ghostty/{GhosttyApp,ZmxLaunch}.swift`;
contract `.claude/rules/settings.md`, `site/docs.html`.
**Swift tests:** `ZmxSupportTests.swift` (`commandShellAcceptsOnlyAnAbsoluteZshOrFishWithLiveFlags`,
`commandShellReplacesAnUnsupportedPasswordDatabaseShell`, `loginShellConfigurationCarriesNoCommandShell`, ...),
`ZmxLaunchTests.swift`, `GhosttyConfigTextTests.swift` (`aNestedIncludeResolvesFromTheSymlinkTargetDirectory`,
`includesLoadAfterEverySourceInQueueOrder`).
**Rust side:** modify `crates/ngterm-core/src/live/support.rs`, `crates/ngterm-session/src/launch.rs`,
`crates/ngterm-gpui/src/session_host.rs` (`wrap_inputs`, `claims`, `command_of` ~213), `crates/ngterm-headless/src/lib.rs`,
`crates/ngterm-core/src/restore/mode.rs`, `crates/ngterm-core/src/ghostty_config/tests.rs`.
**Conformance:** none (the suite cannot vary the password-database shell).

- [ ] `command_shell(command)`: optional `direct:`/`shell:`, an absolute path to a shell ngterm integrates (zsh,
  bash, fish; on Windows an absolute pwsh.exe/powershell.exe path, a whole value that is an existing file taken as
  one path), followed only by live flags (-l, --login, -i, --interactive, -il, -li; decide the pwsh flag set);
  otherwise None ("fish --login", "/usr/bin/env fish", "-C 'echo hi'", "tmux new" refused). Upstream's
  "/bin/bash -l" -> nil is Some here: record the divergence.
- [ ] The last raw `command` across sources and includes decides; latch it at launch beside the restore decision in
  GPUI and headless; a config reload never changes it; a pane whose daemon runs keeps its shell until Reset Live
  Sessions. `eligibility`/configuration use `command_shell.or(login_shell)` for `SHELL`, integration and probe;
  `live-sessions` is claimed only for an effective zsh or fish.
- [ ] The live-unavailable reason names the command setting too, in ngterm's wording; note it in
  `conformance/docs/rust-build.md` if a test reads it.
- [ ] write tests: the classifier table (upstream cases + bash, pwsh, Windows paths); configuration with and without
  a command shell; tcsh login + fish command is eligible; reload keeps the latch; the two missing ghostty include
  tests.

### Task 15: Quit-time Live capture with a slow listing (#918)

**Upstream:** `agterm/Ghostty/ZmxClient.swift`, `agterm/AppDelegate.swift`.
**Swift tests:** `agtermTests/AppDelegateCaptureTests.swift` (`testExitCaptureReadsALiveListingSlowerThanAHundredMilliseconds`).
**Rust side:** modify `crates/ngterm-core/src/commands/relaunch.rs` (doc of `EXIT_CAPTURE_BUDGET`), a test under
`crates/ngterm-core/src/commands/tests/`.
**Conformance:** none.

- [ ] write tests: an exit capture over a `SessionHost` whose `list()` takes 150 ms still fills every live pane's
  foreground (passes on the current code: the upstream failure has no counterpart here).
- [ ] Keep 500 ms (upstream's 700 ms answers zmx's per-daemon probes) and say why in the constant's doc; check
  whether the capture's leader refresh can exceed the budget through the 3 s client timeout, and bound it if so.

### Task 16: Compact sidebar density (#884)

**Upstream:** `agtermCore/Sources/agtermCore/{AppSettings,ControlModes,ControlDispatcher,ControlProtocol,ControlProjection,AppStore,ControlActions,ControlActionsDefaults}.swift`,
`agtermctlKit/MiscCommands.swift`, `agterm/Control/ControlServer+AppCommands.swift`, `agterm/SettingsModel.swift`,
`agterm/Views/{AppearanceSettingsView,WorkspaceSidebar,WorkspaceSidebar+RowRendering,SidebarRowViews,SidebarInfoCell}.swift`;
contract `sidebar density`, `.claude/rules/{control-api,settings,sidebar}.md`.
**Swift tests:** `AppSettingsTests.swift` (density cases), `ControlModesTests.swift`, `ControlDispatcherSidebarTests.swift`,
`ControlProtocolSidebarTests.swift`, `AppStoreTreeProjectionTests.swift`, `SidebarCommandsTests.swift`,
`ControlServerSidebarDensityTests.swift`, `SettingsModelTests.swift`, `SidebarDensityTests.swift`,
`agtermUITests` (`testSidebarDensityCommandResizesTheRenderedRowsAndReadsBack`, `testSidebarDensityPickerPersistsAndTheAppearanceTabFits`).
**Rust side:** modify `crates/ngterm-core/src/persist/settings.rs`, `commands/sidebar.rs` (pattern:
`sidebar.flagged-layout` at ~96-107), `projection.rs`, `settings_model.rs`, `crates/ngterm-ctl/src/sidebar.rs`,
`crates/ngterm-gpui/src/sidebar/{look,view,info}.rs`, `settings/appearance.rs` and tests.
**Conformance:** new `tests/protocol/test_sidebar.py::test_density_echoes_the_density_it_set`, `::test_density_is_app_wide`,
a row in `::test_unknown_mode_is_refused` ("invalid sidebar density: sideways"); `test_tree.py::test_fresh_tree_top_level`
gains `"sidebarDensity": "default"` (listed: the field and the assertion land in the same commit);
a new `since("0.41.0")` `tests/cli/test_cli_output.py::test_sidebar_density` covers `cli:sidebar density` (Task 1
writes it; not the listed `::test_sidebar`, which would go red or skip while the command is a skeleton); `tests/ui/test_sidebar_rows.py::test_density_command_resizes_the_rendered_rows`;
`tests/ui/test_settings.py::test_sidebar_density_picker_persists` and `"settings-sidebar-density": "popup"` in `test_appearance_tab`.

- [ ] `SidebarDensity` raw "default"|"compact" ("Default"/"Compact"); stored `sidebarDensity` absent for default,
  unknown raw resolves to default; not a ghostty key.
- [ ] `sidebar.density [default|compact|toggle]` (absent = toggle) echoes `text`, writes nothing when unchanged,
  needs no window; "invalid sidebar density: X"; the tree always reports it. ctl "density must be default, compact,
  or toggle", no `--window`.
- [ ] GPUI: compact trims row padding, nested indent, info lift and bottom padding, selection inset; name lift 1.
  ngterm's default row padding (13) differs from upstream's (15): apply upstream's deltas and record the choice.
  Settings ▸ Appearance "Sidebar density" picker under Sidebar font size, id `settings-sidebar-density`; the tab
  still fits.
- [ ] write tests: the Swift cases (resolve, metrics, row height at font bounds, mode parse, echo and no-write, no
  window, tree, ctl); the conformance tests above.

### Task 17: Tint palette, hex tints and the Show On memory (#890, #910)

**Upstream:** `agtermCore/Sources/agtermCore/{SidebarTint,AppStore+Tint,ControlDispatcher+SessionMetadata,ControlModes}.swift`,
`agtermctlKit/TintCommands.swift`, `agterm/Views/{TintColor+AppKit,WorkspaceSidebar+TintMenu}.swift`; contract
`session tint`/`workspace tint`, `.claude/rules/{control-api,sidebar}.md`.
**Swift tests:** `SidebarTintTests.swift`, `AppStoreTintTests.swift`, `ControlDispatcherSessionMetadataTests.swift`,
`SessionInfoMarkupTests.swift`, `SnapshotRoundTripTests.swift`, `TintCommandsTests.swift`, `ControlServerTintTests.swift`,
`WorkspaceSidebarTintMenuTests.swift`.
**Rust side:** modify `crates/ngterm-core/src/model/tint.rs`, `commands/tint.rs`, `session_info.rs`, `sidebar_actions.rs`,
`palette/{feeds,run}.rs`, `projection.rs`, `crates/ngterm-ctl/src/{tint,session_meta}.rs`,
`crates/ngterm-gpui/src/sidebar/{look,menu,view,info}.rs` and tests.
**Conformance:** listed tests whose "unknown" color becomes valid change in this commit: `tests/protocol/test_session.py`
(:693, :752 "teal" -> "maroon"), `tests/protocol/test_session_info.py` (:17 "pink" -> "maroon"),
`tests/cli/test_cli_output.py::test_tint_refusals_at_parse_time` (:551, :558; the parametrize ids embed the message,
so replace their ratchet lines); new `test_session.py::test_tint_takes_a_hex_and_reads_it_back_lowercase`,
`::test_tint_takes_the_new_palette_names`, hex cases in `test_workspace.py`, `tests/behaviors/test_relaunch.py`,
`test_session_info.py`, `test_cli_output.py::test_tint_commands`; `tests/ui/test_pointer.py` "Custom #3a7bd5" entry.

- [ ] `TintColor` adds teal, pink, brown, indigo, mint in that order; `TintValue` = a name (any case) or `#rrggbb`
  (with or without "#", stored lowercase; keep `SidebarTint: Copy` with an `[u8; 3]`); rejects "", "maroon",
  "#3a7bd", "#3a7bd5ff", "#3a7bdz", "##3a7bd5". Snapshot `{"color": "#3a7bd5", "on": [...]}`; decode of an unknown
  color fails "unknown tint color: <raw>" and the lossy caller drops it.
- [ ] Set and creation tints take a hex; "invalid tint: <raw> (red|orange|yellow|green|blue|purple|gray|teal|pink|brown|indigo|mint|#rrggbb)";
  the tree reads back the name or the lowercase hex. ctl "tint must be one of: <names>, a #rrggbb color, or clear",
  "--tint must be one of: <names>, or a #rrggbb color". `session.info` markup stays names only.
- [ ] GPUI: the five colors' light/dark values (check against the macOS system colors on the real Mac); a hex draws
  as fixed sRGB; the icon's accessibility value "#3a7bd5 tint"; the Tint menu adds the five colors and a checked
  "Custom #rrggbb" entry when every target shares one hex.
- [ ] The last Show On pick: in memory, app-wide, never persisted, default icon; a menu Parts pick records it; an
  untinted row shows it checked; a menu color pick on an untinted row starts on it; the palette and `session.tint`
  without `--on` keep the icon start.
- [ ] write tests: every Swift case listed; the conformance edits and new tests above.

### Task 18: Refresh the bundled skill

**Upstream:** `plugins/agterm/skills/agterm/{SKILL.md,reference.md,troubleshooting.md}` at v0.41.1.
**Rust side:** modify `build/patches/skill-ngterm/*.patch`, regenerate `resources/skills/ngterm/` with
`.claude/skills/agterm-upstream-sync/scripts/sync-resources.sh`; bump the release in
`crates/ngterm-app/src/integrations/mod.rs`'s header.
**Conformance:** none.

- [ ] Rebase `0001-live-group.patch` (fails at `SKILL.md:160`) and every later patch that conflicts, regenerating
  each as `diff -u` of upstream's files and the hand-merged copy; new `agtermctl` calls (`overlay open --text`,
  `follow`, `search`, `changes`, `sidebar density`) come out as `ngtermctl`, zmx wording as live.
- [ ] `sync-resources.sh --check v0.41.1 v0.41.1` passes; the integrations header names v0.41.1 for every payload.

### Task 19: Review upstream's Ghostty v1.4.0 move (#883)

**Upstream:** `scripts/setup.sh` (`GHOSTTY_REV` 683d8db -> 83fb956), `scripts/ghostty-patches/0001-drain-mailbox-during-surface-teardown.patch`,
`docs/backlog/ghostty-pin-bump-past-vt-1-0-breaks-swift-nullability.md`.
**Rust side:** `docs/backlog/` (a new item via the `workflow:backlog` skill); no pin change in this plan.
**Conformance:** none.

- [ ] Confirm and record: ngterm's `LIBGHOSTTY_VT_REV` (81598385) is an ancestor of v1.4.0 and already has the page
  memory reclamation upstream's "fixed-memory trims" names; the display-link pause is the macOS app renderer's and
  the drain-mailbox patch touches App/Surface/renderer threads, none of which libghostty-vt has.
- [ ] File a backlog item to evaluate a `LIBGHOSTTY_VT_REV` bump to the vt-1.0.0 tag as its own change: Zig 0.16 ->
  0.17, `vt_write_until_ground` -> `vt_write_until` (`crates/ngterm-ghostty-vt/src/terminal.rs:400-410`), the
  kitty_graphics.h rework (patch 0001), whether patch 0004 becomes removable, patch 0003, and the terminal fixes
  gained (#14761, #14762, #14814, #14812, #14832, #14856, #14858).

### Task 20: Workspace chevrons setting; Collapse/Expand Workspace in the row menu (#909)

**Upstream:** `agtermCore/Sources/agtermCore/AppSettings.swift`, `agterm/Views/{SidebarOutlineView,WorkspaceSidebar,WorkspaceSidebar+ContextMenu,SettingsView}.swift`;
contract `.claude/rules/{sidebar,settings}.md`.
**Swift tests:** `AppSettingsTests.swift` (`rowElementsAreShownByDefaultAndHideIndependently`),
`SidebarWorkspaceExpansionMenuTests.swift`, `SidebarRowIndentTests.swift`, `agtermUITests`
(`SettingsUITests`, `SidebarUITests.testHiddenChevrons*`).
**Rust side:** modify `crates/ngterm-core/src/persist/settings.rs`, `settings_model.rs` (+ tests),
`crates/ngterm-gpui/src/interface_glyph.rs`, `sidebar/{context,view,menu}.rs`, `sidebar.rs`, `settings/interface.rs` and tests.
**Conformance:** listed `tests/ui/test_settings.py::test_interface_tab_toggles_title_bar_buttons`; new
`tests/ui/test_pointer.py::test_the_workspace_row_menu_collapses_and_expands_it`,
`tests/ui/test_settings.py::test_hidden_workspace_chevrons_persist`.

- [ ] `InterfaceElement::WorkspaceChevrons` ("workspaceChevrons", sidebar section after session icons, shown by
  default, "Workspace chevrons", chevron glyph).
- [ ] Hidden: no workspace disclosure and no reserved strip; nested rows shift left by it; a companion parent keeps
  its whole disclosure ahead of its content (both densities); toggling re-lays out both ways.
- [ ] Workspace row menu: "Collapse Workspace"/"Expand Workspace" after "Open Directory…", folding the clicked
  workspace (not the active one), persisted, working with chevrons hidden and with row click off.
- [ ] Interface tab: the Session info picker and the active-window toggle share the Sidebar section's last row,
  the toggle labelled "Active window only".
- [ ] write tests: element default/section/name/order; menu entries per expansion and the clicked-row fold; row
  leads with chevrons hidden at both densities; the conformance tests above.

### Task 21: Palette Move Session rows act on the live selection (#879)

**Upstream:** `agterm/AppActions+Palette.swift`. **Swift tests:** `AppActionsPaletteTests.swift`
(`testMoveRowsMoveTheSessionSelectedWhenTheyRun`, `testAMoveRowGoesInertWhenTheSessionSelectedUnderItAlreadyLivesInItsTarget`,
`testMoveRowsAreListedButInertForACompanion`, `testAMoveRowGoesInertWhenItsTargetIsRemovedUnderIt`).
**Rust side:** modify `crates/ngterm-core/src/palette/{mod,feeds,run}.rs` (`move_session_entries` ~261-283, the run arm
~124-128, `palette_entry_enabled` ~27-50), `commands/tests/palette.rs`.
**Conformance:** none (Rust tests on the fakes).

- [ ] `PaletteAction::MoveSession { workspace }` captures no session; enabled = UI actions enabled, a selected session,
  its workspace differs from the target and `can_move_sessions` allows it; run moves the session selected then; an
  inert row does nothing and keeps the palette open.
- [ ] write tests: the four Swift cases.

### Task 22: One queued title per pane; the OS window title coalesced (#911, #817)

**Upstream:** `agtermCore/Sources/agtermCore/TitleAdmission.swift`, `agterm/Ghostty/{GhosttyCallbacks,GhosttySurfaceView}.swift`,
`agterm/Views/{WindowAccessor,WindowContentView+Titlebar}.swift`.
**Swift tests:** `TitleAdmissionTests.swift`, `WindowAccessorTests.swift` (`testRapidTitleChangesAreCoalescedIntoOneTrailingWrite`).
**Rust side:** create `crates/ngterm-core/src/title_admission.rs` and a pure title-write throttle; modify
`crates/ngterm-term/src/{effects,pane}.rs`, `crates/ngterm-gpui/src/window.rs` (`sync_title` ~937-959),
optionally `crates/ngterm-core/src/{model/session.rs,commands/panes.rs}`; a real-PTY test in `crates/ngterm-term/tests/pane.rs`.
**Conformance:** none (performance).

- [ ] TitleAdmission per pane: a title equal to the last queued one is skipped while that one is queued; a different
  title, or the same after the queue drains, is queued; completion happens where `Pane::take_events` drains (so
  GPUI, headless and mobile all benefit); lead and remote-link notice titles bypass it.
- [ ] OS window title (all three OSes, through GPUI's `set_window_title`): the first change after a quiet 500 ms writes
  at once, later ones collapse into one trailing write with the latest title, an unchanged title writes nothing. An
  unchanged reported title does not settle the window.
- [ ] write tests: the five TitleAdmission cases; the throttle with an injected clock; the real-PTY flood test (5,000
  identical titles give a few events, the last differing title arrives).

### Task 23: The layout lock holds while an HTML page has the keyboard (#904)

**Upstream:** `agterm/Commands/TerminalInputSource.swift`, `agterm/Views/HtmlOverlayRegistry.swift`; contract
`.claude/rules/settings.md`.
**Swift tests:** `TerminalInputSourceTests.swift`.
**Rust side:** modify `crates/ngterm-gpui/src/input_source.rs` (`refresh` ~285-294, module doc),
`html_overlay/view.rs`, `html_overlay/webview2/view.rs`, `html_overlay/registry.rs`, `webview2/registry.rs`,
`html_overlay.rs` (stub), `input_source/tests.rs`, `crates/ngterm-gpui/tests/input_source.rs`; README.md (~180).
**Conformance:** none (it switches the system layout).

- [ ] Held while a terminal (main, split, scratch, overlay, quick) or an HTML overlay page, its fields included, holds
  an active ngterm window's keyboard; a page focus change requests a refresh; Settings fields, the palette, ask
  dialogs, another app and no ngterm window release it. Make `page_holds_keyboard` reflect native web-view focus on
  Windows and macOS (check on a runner or the VM).
- [ ] write tests: the holds predicate (terminal, page, text field, palette, inactive window) and the refresh on a
  page focus change.

### Task 24: Claim v0.41.1 parity and update the docs

**Rust side:** `PARITY_LEVEL` and its mirrors (`tag-mentions.sh v0.40.0`, class `claim`: `crates/ngterm-proto/src/version.rs`,
`crates/ngterm-core/src/dispatch.rs`, `crates/ngterm-server/tests/server.rs`, `conformance/agterm_conformance/impls/rust.py`,
`conformance/docs/rust-build.md`, README.md's and conformance/README.md's claim lines); README.md,
`conformance/README.md`, `conformance/docs/rust-build.md`; `conformance/rust-passing*.txt`.
**Conformance:** every new id from Tasks 1-23 and the existing ratchet.

- [ ] Bump `PARITY_LEVEL` and its mirrors once every task above is done.
- [ ] Run the newly unlocked `since("0.41.0")`/`since("0.41.1")` cases and the existing ratchet; add actually passing
  ids from the reports to the right OS's ratchet; never remove an old id to hide a failure; no skip counts.
- [ ] README.md (Parity: what v0.41.x brought and how each OS does it: the reveal chord, file stamps, the Find
  chord), conformance/README.md and rust-build.md (the recorded divergences: bash/pwsh Live command shells, the
  Windows preconnect handling, the density metrics) describe what changed.
- [ ] manual (user): push, run `reference-macos.yml` on the branch, open the PR so the cross-platform jobs and the
  full suite run.
