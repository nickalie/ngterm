# Open pages survive a restart and a reattach

## Overview

- An HTML page overlay (`session overlay open --html|--url|--image|--markdown`, session-wide or `--pane left|right`,
  and the link-click overlays) lives until it is **closed**, not until the process or the viewer showing it goes:
  - an app (or headless) restart brings back every page that was open, in the slot it held;
  - on another machine's session (Attach Remote), a page shown on this viewer comes back when the viewer attaches
    again: after a detach, a lost link, closing the remote row, or a restart of either side.
- Closing the page itself ends it for good, wherever it was closed: its X, ⌘W/Ctrl+W, `session overlay close`,
  the page's own `window.close()`, a submit, `End Session on HOST`, or the session's close on the origin.
- An extension beyond upstream, which keeps every overlay ephemeral ("Losing the presenter ends its overlays for
  good: no later stream adopts one"; overlays never persisted). README's extension list gets an entry.
- Out of scope: page companions (session rows whose content is a page) are still not persisted; a page that was
  already open on a desktop origin before a viewer attached still stays on the origin (upstream's "one already open
  stays local"); a restored page reloads its original source, not the address the user navigated to, and loses its
  in-page state (JS memory; storage unless `--persistent`).

## Context (from discovery)

- Model: a session-wide page is `Session.overlay` (`OverlaySlot.html`, `size_percent`, `points`,
  `background_color`, `hidden`); a pane page is `PaneOverlays.{left,right}.overlay` (`PaneOverlay.html`,
  `background_color`, `hidden`, `holds_cover`). The value is `crate::html::HtmlOverlay` (`id`, `replica_of`,
  `source: HtmlSource`, `flags: HtmlPageFlags`, load state, `row`). Docs on both say "Ephemeral: never persisted".
- Opening: `crates/ngterm-core/src/commands/overlay_page.rs` (`open`, `PageOpening`, `register_page_outcome`),
  `crates/ngterm-core/src/model/html_overlay.rs` (`open_html_overlay_sized`). Outcomes: `html/outcomes.rs`; the
  commit's page sweep settles a page that left every slot as `dismissed`.
- Persistence: `crates/ngterm-core/src/persist/snapshot.rs` (`SessionSnapshot`, lossy optionals, `lossy_entries`),
  `crates/ngterm-core/src/model/snapshot.rs` (`session_snapshot`, `session_from_snapshot`, `restore`), bootstrap at
  `persist/library.rs` (`store.restore(&snapshot, true)`); closed-window and Recent Closed reopen go through the same
  `session_from_snapshot`.
- Origin side of a presented session: `crates/ngterm-core/src/remote/page.rs` (`open_remote_page`, page commands,
  `receive_remote_page_frame`, `close_remote_page`), `remote/handoff.rs` (`presenter_lost`), the model half in
  `model/remote_overlay.rs` (`RemoteOverlaySlot` with `owner` = presenter generation, `RemotePage` with token, claim
  deadline, hidden/sequence state; `remote_overlay_presenter_lost` ends every page), the page channel in
  `remote/page_serve.rs` (claim, helper loss and claim expiry end the page). `remote/hub.rs`: `PresenterGrant`
  (`acquire` on `Body::PresenterAcquire` bumps the generation; no event is emitted for a grant today, only
  `HubEvent::PresenterLost`), `SUPPORTED_KINDS`, `PAGE_KINDS`; `remote/mod.rs` dispatches hub events.
- Viewer side: `crates/ngterm-core/src/remote/replica_page.rs`. Every way a replica leaves its slot sends
  `page.closed` once (`replica_page_released`, from the commit's sweep): the origin's close, a close here, the
  helper's end, the stream's loss and the row leaving (`close_replica_pages`), and the quit path
  (`shutdown_replica_channels`) has to be checked for it.
- Headless origin: `crates/ngterm-headless` (`core.set_page_presenter_only()`): every page goes to the presenter.
- Frames: `remote/frames.rs`; an unknown kind decodes to `Body::Unknown` and is dropped, so a new presenter→origin
  kind is safe to receive, but the viewer must only send it to an origin whose hello answer lists it.
- Conformance: `conformance/tests/protocol/test_live_sessions.py` drives a fake presenter (`_page_presenter`,
  `wait_frame`); check whether any listed id there asserts that a page ends on presenter loss before changing it.

## Development Approach

- **testing approach**: Regular (code first, then tests); every task's checks are in the same task.
- Complete each task fully before the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests** for its code, on the `ngterm-host` fakes in
  `ngterm-core` (and `ngterm-headless` where it changes).
- **CRITICAL: all tests must pass before starting the next task.**
- **CRITICAL: update this plan file when scope changes during implementation.**
- Checks are sized to the change (AGENTS.md > Choosing checks): clippy and tests on the touched crates; the protocol
  conformance file for live sessions when Task 2 or 3 changes its behavior; no full suite locally.

## Testing Strategy

- Unit tests on the fakes:
  - snapshot round trip of every page kind and slot, the lossy decode of a bad entry, a file page whose file is gone
    dropped at restore, a replica never written;
  - restored pages are registered pending, so `overlay result --page` and `submit` work on them;
  - presenter loss parks pages; the next presenter that speaks pages gets a fresh `page.request` (new token, new
    owner generation) for each parked page, a hidden one followed by its hide command; a presenter that does not
    speak pages leaves them parked; `overlay close` on a parked page ends it and sends nothing;
  - viewer: a close here sends `page.closed`; the row leaving, the stream's loss, the helper's end and the quit
    send no `page.closed` to an origin that speaks `page.detached` (the helper's end sends `page.detached`), and
    keep the old `page.closed` toward an origin that does not.
- Protocol conformance (`test_live_sessions.py`): a fake presenter that disconnects and a second one that attaches
  receive the same page again; a page closed with `session overlay close` is not re-sent.
- No new UI, so no `conformance/tests/ui` work; the GPUI frontend builds web views for whatever page sits in a slot.

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Keep the plan in sync with the work.

## Solution Overview

1. **Local pages persist.** Each `SessionSnapshot` gets a `pages` list written from the session's page slots
   (session-wide, left, right), replicas excluded. `session_from_snapshot` puts them back with fresh page ids;
   the core registers each restored page's outcome pending. A file, image or Markdown page whose file no longer
   exists is dropped at restore, so no page outlives the artifact it showed.
2. **The origin keeps a presenter's pages while no presenter shows them ("parked").** Presenter loss, the page
   channel's helper loss and the claim timeout park the page instead of ending it: the slot stays reserved, the
   outcome stays pending, the read-back says `parked`. When a presenter that speaks pages is granted the session,
   every parked page is handed to it again, as a fresh `page.request`. Parked pages persist too, so a headless
   restart keeps them.
3. **The viewer tells a close from a departure.** Only a close of the page itself sends `page.closed`. A replica
   that leaves because its row, stream, helper or app goes sends nothing (the helper's end sends the new
   `page.detached`, so the origin parks it while the stream lives on), toward an origin whose hello answer lists
   `page.detached`; an older origin keeps getting `page.closed`, its old behavior.

## Technical Details

- **Snapshot entry** (`SessionSnapshot.pages: Option<Vec<PageSnapshot>>`, camelCase, lossy per entry through
  `lossy_entries`, omitted when empty):
  - `slot`: `session` | `left` | `right`; a `right` entry needs the restored session to have a split, else dropped;
  - `source`: `{kind: file, path, grantRoot?}` | `{kind: url, url}` | `{kind: image, path}` |
    `{kind: markdown, path}`; checked again at restore (`grant_error` for files, an absolute path, an http(s) URL);
  - the five `HtmlPageFlags`, `sizePercent`, `width`/`height` (session slot only), `backgroundColor`, `hidden`;
  - `viewer: true` for a page parked or shown on a presenter (written from `RemoteOverlaySlot.page`), which restores
    as a parked reservation instead of a local slot;
  - a page id is not written: every restore mints a fresh one, so a reopened window or Recent Closed entry never
    collides with an outcome already settled.
- **Restoring**: pane overlays get a fresh generation from `PaneOverlays::next_generation`; `holds_cover` is
  recomputed, not stored. A presenter-only origin (headless) restores every local page as parked.
  Outcome registration happens wherever the core learns of restored pages (bootstrap, window reopen, Recent Closed
  reopen); the commit sweep is a fine single place, as long as a page answered before stays answered.
- **Parking** (model: e.g. `RemotePage.parked`, or an `owner` that names no live generation; choose one and keep
  `send_remote_page_command`'s owner check meaningful):
  - `remote_overlay_presenter_lost` parks pages and still frees/cancels program jobs as today;
  - helper loss and claim expiry on the origin park the page and send `page.close` to the presenter so it drops its
    broken replica; a `page.closed`/`page.rejected` for a page that is parked is ignored;
  - re-hand: a new `HubEvent::PresenterGained(session)` (pushed when `acquire` grants a new holder) makes the core
    send each parked page as a fresh `page.request`: new token and claim deadline, owner = the new generation,
    `follow: false`, the stored size and points; then a hide `page.command` if the page was hidden. Pages go to the
    presenter only when it speaks every `PAGE_KINDS` kind, and a loopback URL only when it speaks the tunnel kind;
    otherwise they stay parked. The placement rule is not re-run: a parked page belongs to the session's viewer;
  - commands on a parked page: `close` ends it (outcome `dismissed`, nothing sent); `hide`/`show`/`toggle` record
    the visibility, delivered on re-hand; `resize` updates the stored size; `reload`, `navigate` answer
    `VIEWER_GONE`; `text`/`copy`/`result` answer as for any remote page;
  - read-back: `htmlOverlays[]` and `remoteOverlays[]` carry `parked: true` (omitted when false); a park or re-hand
    emits `tree.changed`.
- **`page.detached`**: presenter→origin, body `{page}` like `page.closed`; listed in `SUPPORTED_KINDS` so the
  origin's hello answer echoes it to a viewer that lists it; the hub forwards it from the current presenter only.
  Viewer: a replica record carries why it left; `replica_page_released` sends `page.closed` only for a close of the
  page (the user, the page, a submit, the origin's own close echo stays as today), `page.detached` for the helper's
  end, and nothing for the stream's loss, the row leaving (soft close included) and the quit — toward an origin that
  does not list `page.detached`, all of these keep sending `page.closed`.

## What Goes Where

- **Implementation Steps:** `ngterm-core` (persist, model, remote, commands), `ngterm-headless` if its restore or
  hub wiring needs it, `ngterm-gpui` only if its quit or row-close path bypasses the core's replica release,
  protocol conformance tests, README, the `ngterm` skill.
- **Post-Completion:** a manual check against a real headless server and a desktop viewer.

## Implementation Steps

### Task 1: Local pages persist across a restart

**Files:**
- Modify: `crates/ngterm-core/src/persist/snapshot.rs`, `crates/ngterm-core/src/model/snapshot.rs`
- Modify: `crates/ngterm-core/src/html/mod.rs`, `crates/ngterm-core/src/model/pane_overlay.rs` (the "never
  persisted" docs), wherever outcomes are registered for restored pages
- Tests: `crates/ngterm-core/src/persist/` and `crates/ngterm-core/src/model/tests/`

- [x] `PageSnapshot` and `SessionSnapshot.pages` as Technical Details describes, written from `session_snapshot`
      for local pages (no replicas, no page companion rows)
- [x] `session_from_snapshot` restores them into the session-wide and pane slots with fresh ids and generations;
      drops entries whose file is gone, whose source no longer validates, or whose pane does not exist
- [x] restored pages registered pending for every restore path (bootstrap, closed-window reopen, Recent Closed)
- [x] tests: round trip per source kind and slot; lossy decode of a malformed entry; gone file dropped; replica not
      written; restored page answers `overlay result --page` pending and accepts a submit
- [x] ➕ a persisted page that opens, closes or moves asks its windows for a debounced save (overlays asked for none
      before), so a crash neither loses an open page nor revives a closed one
- [x] clippy and tests on `ngterm-core`; must pass before Task 2

### Task 2: The origin parks a presenter's pages and hands them to the next presenter

**Files:**
- Modify: `crates/ngterm-core/src/remote/{hub,handoff,page,page_serve,mod}.rs`,
  `crates/ngterm-core/src/model/remote_overlay.rs`, the tree projection of `htmlOverlays`/`remoteOverlays`
- Modify: `crates/ngterm-core/src/persist/snapshot.rs`, `crates/ngterm-core/src/model/snapshot.rs` (`viewer` entries)
- Modify: `crates/ngterm-headless` if its event wiring or restore needs it
- Tests: `crates/ngterm-core/src/commands/tests/remote_page.rs`, `model/tests/overlay_events.rs`, hub tests;
  `conformance/tests/protocol/test_live_sessions.py`

- [x] `HubEvent::PresenterGained` on a new grant, dispatched in `remote/mod.rs`
- [x] presenter loss, helper loss and claim expiry park pages; a stale `page.closed`/`page.rejected` for a parked
      page is ignored
- [x] re-hand of parked pages to a page-speaking presenter, hidden state replayed
- [x] commands on a parked page; `parked` read-back; `tree.changed` on park and re-hand
- [x] parked pages persist (`viewer: true`) and restore as parked; a presenter-only origin restores local pages as
      parked
- [x] tests on the fakes for each item above; protocol conformance: disconnect and reattach re-sends the page, a
      closed page is not re-sent; check `rust-passing.txt` ids that asserted the old ending and update them with
      the reason
- [x] ➕ a Rust-only capability `pages-survive` (base.py, rust-build.md, `ngterm-gpui`'s `CAPABILITIES`, the zsh and
      fish legs' `--must-pass`), so the new conformance tests skip on the reference and the helper-loss test branches
      on it instead of failing upstream's reference
- [x] clippy and tests on `ngterm-core` and `ngterm-headless`, the live-sessions protocol file; must pass before
      Task 3

### Task 3: The viewer sends `page.closed` only for a close of the page

**Files:**
- Modify: `crates/ngterm-core/src/remote/{replica_page,frames,hub}.rs`
- Modify: `crates/ngterm-gpui` only if its quit or row-close path sends `page.closed` around the core
- Tests: the replica page tests in `ngterm-core`

- [ ] `Body::PageDetached` (`page.detached`) in frames, `SUPPORTED_KINDS`, the hub's presenter-frame forwarding,
      and the origin's handling (park, as for helper loss)
- [ ] the replica record knows why it left; `replica_page_released` sends `page.closed`, `page.detached` or nothing
      as Technical Details says, gated on the origin's hello answer
- [ ] the quit path and the row's soft close send nothing to a `page.detached` origin
- [ ] tests: each departure reason toward a new and an old origin; a viewer close ends the page on the origin end
      to end through the fakes
- [ ] clippy and tests on `ngterm-core` (and `ngterm-gpui` if touched); must pass before Task 4

### Task 4: Verify acceptance criteria

- [ ] every Overview item holds: restart restores pages (local and parked); detach/reattach, link loss, row close
      and either side's restart bring a page back; every close path ends it for good
- [ ] older peers: an old viewer (no `page.detached`) still ends pages by `page.closed`; an old origin keeps getting
      `page.closed`
- [ ] clippy and tests on the touched crates; the live-sessions protocol file

### Task 5: [Final] Documentation

- [ ] README: an "Another extension beyond upstream" entry (pages survive a restart and a reattach, close ends
      them, what stays out of scope)
- [ ] the `ngterm` skill (`resources/skills/ngterm/`): pages persist until closed; `parked` read-back
- [ ] crate and module docs that said pages are ephemeral or end with their presenter
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

- Manual check with a real headless server and a desktop viewer: open a page in a right pane, detach, reattach;
  quit the viewer and start it again; restart the headless server; close the page and confirm it stays closed.
