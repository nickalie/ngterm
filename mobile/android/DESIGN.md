# ngterm for Android: design system

The design system every screen of the app stays inside. A screen that needs something new extends this file on
purpose, in the same change. The code is `app/src/main/kotlin/com/nickalie/ngterm/android/ui/theme/` (tokens),
`ui/components/` (the inventory), `ui/navigation/` and `ui/AdaptiveScaffold.kt` (the frame). The goldens in
`app/src/test/screenshots/design/` show all of it at three widths, light and dark, and at font scale 2.0.

## Brief

- **Subject:** a remote control for terminals. ngterm runs on a server or a desktop, AI agents work in its sessions,
  and this app reaches them over ssh to watch, answer and type.
- **Audience:** developers who run agents on machines they own, checking in from a phone between other things, or
  working properly from a tablet.
- **Primary job:** see at a glance which machine and which session needs you, and get into its terminal fast. The
  terminal is the hero; everything else is the way to it.

## Direction: copper and patina

ngterm's mark is a copper gear with a `>_` prompt (`resources/icons/ngterm.svg`, `#cf6e31`). The app takes its
material literally: **copper**, the conductor, carries the brand and marks where you are; **verdigris**, the patina
copper grows, is the healthy state, connected and working. Around them is a cool **graphite** (dark) or **drafting
paper** (light) ground, the colors of a machine room and a schematic, never warm cream and never pure black.

The one loud element is the **bus bar**: a short, thick copper bar at the leading edge of the selected destination
or session (the top edge in the bottom bar). It is the copper strip that connects you to the thing you are on, and it
marks nothing else. Everything around it is quiet: flat tonal surfaces with no shadows, no gradients, no decorative
borders, and type that does the hierarchy.

### The plan, reviewed against the brief

The first pass and what the review changed:

| First pass | Why it was generic | Revised to |
|---|---|---|
| Near-black `#111` ground with a single copper accent | the "dark with one bright accent" default | a graphite with a green-blue cast (`#1B2226`), and a second, subject-born accent (verdigris) for health |
| Light theme on warm off-white | the cream-and-terracotta default, and copper on cream reads as that tell | a cool drafting grey (`#F2F4F5`); copper deepened to `#984A19` so it reads as metal, not clay |
| One 12dp radius on every surface | the uniform card kit | radii by rank: 4-6dp badges, 12dp cards and rows, 20dp dialogs, round only for buttons |
| Monospace for small labels as decoration | a template tell | mono only for what is compared character by character: hosts, ports, fingerprints, key lines |
| A pill indicator plus a tint plus the bus bar on the selected item | three marks for one fact | the bus bar and the primary color only |
| Status as colored dots | color alone carries meaning | a distinct glyph per status, always with its word |

## Color

Defined in `ui/theme/Color.kt`. `ContrastTest` checks every promise below, light and dark.

| Token | Dark | Light | Role |
|---|---|---|---|
| Graphite / Drafting | `#1B2226` | `#F2F4F5` | background and surface |
| containers (lowest to highest) | `#161C1F` `#1F272B` `#242D32` `#2A343A` `#313C42` | `#FFFFFF` `#ECEFF0` `#E6EAEB` `#E0E4E6` `#D9DFE1` | cards (low), bars (container), banners and dialogs (high) |
| on surface / variant | `#E3E8EA` / `#A9B4B9` | `#1B2226` / `#47535A` | text / secondary text |
| primary (copper) | `#E3884F` | `#984A19` | the bus bar, the selected destination, buttons, text buttons |
| secondary (verdigris) | `#5FBFAA` | `#1A6A5D` | healthy and attached |
| tertiary (brass) | `#D9B25A` | `#7A5A0A` | on its way |
| error (oxide) | `#F2867A` | `#A83226` | failed, destructive |
| outline / variant | `#7D8A90` / `#3A464C` | `#6B787F` / `#C3CCD0` | control edges (3:1) / dividers |
| brand copper | `#CF6E31` | `#CF6E31` | the mark's tile only: a fill, never text |

- Every text role is at least 4.5:1 on its container, and on every surface up to `surfaceContainerHigh`; primary,
  secondary and error are used as text and meet that too. Outlines are at least 3:1.
- **Dynamic color** is optional (`NgtermTheme(dynamicColor = true)`, Android 12+, Settings' Wallpaper colors). It
  replaces only Material's roles; the status colors, the type and the terminal palette stay the app's.

### Status

`ui/theme/Status.kt`. A status is a color, a glyph and a word, and the three always go out together.

| Tone | Glyph | Word | Dark | Light |
|---|---|---|---|---|
| Online | filled circle | Connected | `#5FBFAA` | `#1A6A5D` |
| Connecting | open ring | Connecting | `#D9B25A` | `#7A5A0A` |
| Offline | struck ring | Offline | `#9AA6AC` | `#535F65` |
| Attention | diamond | Needs you | `#F0904F` | `#984A19` |
| Error | triangle | Failed | `#F2867A` | `#A83226` |
| Idle | capsule | Idle | `#A9B4B9` | `#47535A` |

- Every status color is 4.5:1 on every surface up to `surfaceContainerHigh`, so it can be text.
- No two tones share a glyph.
- An agent's status comes from its origin as its wire name (`active`, `blocked`, `completed`, `idle`), with a hex
  color and one of upstream's eleven shapes (circle, square, triangle, triangle-down, play, diamond, capsule, seal,
  star, bolt, bell) only when the agent overrode them. Each has a 16dp vector drawable of its own, a solid
  silhouette like the rest, drawn within the same 12-13dp extent so no shape reads larger than another in a row. The
  app's hello lists `status.shapes`, so an origin sends it all eleven; a name this build doesn't know draws the
  circle. `AgentStatusBadge` says the name as a word and draws the origin's shape, lifting its color toward
  white or black (`readableOn`) until it reads on every surface up to `surfaceContainerHigh`, so it holds on a card
  or the sidebar as on the ground: the origin's color is the user's choice, the legibility is ours. What the origin
  leaves unset comes from `AgentState`, the app's tones in desktop's roles (orange blocked, green completed):

  | Wire name | Word | Glyph | Color |
  |---|---|---|---|
  | `active` | Working | filled circle | Online |
  | `blocked` | Waiting | diamond | Attention |
  | `completed` | Done | star | Online |
  | `idle` | Idle | capsule | Idle |

### Terminal palettes

`ui/theme/TerminalPalette.kt`: background, foreground, cursor, selection and the 16 ANSI colors.

- **Copper** (dark default): the Graphite ground, so the terminal and the chrome are one surface; a copper cursor,
  verdigris for cyan.
- **Drafting** (light default): the drafting ground with ink text, the ANSI colors darkened to read on it.
- **Ghostty**: desktop ngterm's starting point, Ghostty's `#282c34` ground with white text and libghostty-vt's
  built-in 16 colors.

The theme picks Copper or Drafting with the light or dark side, unless Settings pins one (`NgtermTheme(terminal =)`).
Every palette's text is 4.5:1 on its ground and on its selection, and its cursor 3:1. `TerminalView` hands the palette
to the pane (`Pane.set_colors`), so the terminal resolves ANSI colors with it and every reattach starts from it; colors
a program sets (OSC 4/10/11/12) win, as on desktop.

## Type

`ui/theme/Type.kt`. One family in two widths, both variable fonts (wght 200-800) bundled from Google Fonts under the
OFL (`licenses/`):

- **Atkinson Hyperlegible Next** for the chrome. The Braille Institute drew it to tell similar letters apart, which
  is the job here: hostnames, session names and statuses read at a glance, at any font scale.
- **Atkinson Hyperlegible Mono** for what is compared character by character: `user@host:port`, fingerprints, key
  lines (`NgtermTheme.mono`). It is not the terminal's font: the terminal's is JetBrains Mono (The terminal, below),
  which the chat also uses for what the terminal holds (`TerminalFamily`, `ChatType`: a tool's title at 13 sp, code at
  13 sp, output and diffs at 12 sp).

The scale is the classic one (11, 12, 14, 16, 18, 21, 24, 30, 36, 48 sp). The two largest display sizes are light,
headlines medium, titles and labels semibold, body regular. Sentence case everywhere; no all-caps labels, no eyebrows
above headings.

| Role | Size / line | Weight | Used for |
|---|---|---|---|
| headlineSmall | 21 / 28 | 500 | a state panel's title |
| titleLarge | 18 / 24 | 600 | the sidebar's wordmark, screen titles |
| titleMedium | 16 / 22 | 600 | a machine card's name, a cover's title |
| titleSmall | 14 / 20 | 600 | a session's name, section headers, sidebar rows |
| bodyMedium | 14 / 20 | 400 | body text, banners |
| bodySmall | 12 / 16 | 400 | a session's context line |
| labelMedium | 12 / 16 | 600 | status words, bar labels |
| labelSmall | 11 / 16 | 600 | the collapsed rail's labels |
| mono small / medium | 12 / 16, 14 / 20 | 400 | addresses / fingerprints, key lines |

## Spacing, shape and size

`ui/theme/Spacing.kt`.

- A 4dp grid: 2, 4, 8, 12, 16, 24, 32, 48. Gutters are 16dp on a phone, 24dp from the medium width up.
- Anything tappable is at least 48dp on its short side; rows are 56dp.
- Lists and cards stop growing at 640dp (`readableWidth`); text blocks at about 420dp, under 80 characters.
- Radii by rank: 4dp extra-small, 6dp badges and banners, 12dp cards and rows, 20dp dialogs and sheets, 28dp for the
  largest; buttons are fully round.
- Depth is tone, not shadow: a card is `surfaceContainerLow` on the ground, a bar `surfaceContainer`, a banner or a
  dialog `surfaceContainerHigh`.

## Motion

`ui/theme/Motion.kt`. Motion answers an action and shows what changed; nothing moves on its own, and nothing loops.

- Durations: 120ms quick (a selection, a tint), 220ms standard (the rail opening, a destination fading in), 320ms
  emphasized (a layout swap, a cover). Easing: Material's emphasized decelerate.
- Every tap gets the ripple at once, well inside 100ms.
- The bus bar grows from its middle when its item is selected.
- Reduced motion: Compose scales every animation by the system's animator duration scale, so "Remove animations"
  turns all of them into cuts. Nothing here depends on an animation finishing.

## Icons

Line icons drawn for the app in `res/drawable/ic_*.xml`: 24dp, 2dp strokes, round ends, tinted by `Icon`. Machines
(two racked servers), Sessions (a terminal window), Settings (sliders), Menu and Menu open, Add, Retry, Alert, Offline
(a struck signal), Close, Lock, Back (mirrored right to left), More (an item's menu), Send (the return key's arrow,
mirrored right to left, `ic_send`), Attach (a paperclip, `ic_attach`), Chat (a speech bubble with two
lines, `ic_chat`) and the extra keys' arrow
(`ic_key_arrow`, turned for each direction). The chat's lines have a glyph per tool kind (`ic_tool_*`): shell (a `>_`
prompt), edit (a pencil), read (a page with a folded corner), search (a magnifier), web (a globe), task (a branch,
a subagent), other (four tiles); and per notice: stop (a square, an interrupt), compact (two arrows meeting at a line);
thinking is three filled dots (`ic_thinking`). The status bar's small icon (`ic_notification`) is the mark's `>_` alone,
white on nothing, as Android masks it. The status glyphs are 16dp filled or stroked shapes (`ic_status_*`). The mark
(`ic_ngterm_mark`) is the launcher's `>_` cut out of a copper tile. No icon font or icon library.

## Components

`ui/components/`. Each takes its content as parameters; none reads app state.

| Component | What it is |
|---|---|
| `BusBar` | the copper bar marking where you are |
| `StatusGlyph`, `StatusBadge`, `AgentStatusBadge` | a status's glyph; glyph and word for an app tone; for an origin's status |
| `MachineCard` | a machine: name, mono address, connection badge, and a summary slot for its sessions' status |
| `SessionRow` | a session: name, context or companion line (a machine's list indents every companion; Sessions only one under its attached parent), status slot, Attached mark, the bus bar when on screen, and an optional menu at the end (Attach or Detach, Duplicate session, Chat view, End session) |
| `SectionHeader` | a group's heading with an optional count and one text action |
| `MessagePanel` with `EmptyState`, `ErrorState`, `OfflineState` | what a screen shows instead of content: icon, title, what to do, and the one action that does it (busy while it runs: a small spinner, no taps); at most one quieter text-button action under it, for a retry after the fix the main one points to |
| `Banner` | a strip about the whole screen while it stays usable, with an optional action and dismiss; announced politely |
| `ConfirmDialog` | the confirmation before what can't be undone; the button repeats the action's words, in the error color |
| `KeyPlate`, `CopyButton` | text compared or pasted character by character (a key line, a fingerprint, a command, a machine's sign-in message): mono, selectable, wrapping anywhere, on a `surfaceContainerLow` plate with an optional label above and Copy or Share under it; Copy says "Copied" for 2 s. A link in it is copper and underlined and opens on a tap; a long press still selects |
| `ScreenFrame` | a screen of reading and filling in: an optional top row (Back or Close, the title as a heading, one action such as Save), then a scrolling column no wider than 640dp (520dp in the first run), centered, with the phone's or the tablet's gutter, padded for the bars, the cutout and the keyboard |
| `Cover` | a veil over a pane that can't be used as it is (another device drives it, it is reattaching), with its action; it takes every touch, and the pane under it is hidden from TalkBack (`Modifier.underCover`); `icon = null` shows a spinner instead of a glyph |

The update panel (`ui/updates/UpdatePanel.kt`, the GitHub build's alone) is Settings' Updates plate and the update
sheet's content: one plate, `surfaceContainerLow` in Settings, the sheet itself in the sheet. Its state is a status
glyph and a headline in the app's tones, so it adds no color: up to date is Online (healthy), a release on offer is
Attention (it needs you), checking, downloading, verifying and installing are Connecting (on its way, the download's
bar in that brass too), a failure is Error, and not checked yet is Idle. The step between the installed version and the
offered one is mono (`0.2.3 → 0.3.0`), as versions are compared character by character; TalkBack reads it in words.
Install is the one filled button; Release notes and Not now are text buttons, and Not now leaves once said. A failure
says what failed in words (the signature, the checksum, the signing key, the installer's own message) and offers Try
again for that step.

Rules the components hold to:

- Every screen has empty, loading, error and offline states, each with a next action. An empty screen invites the
  first step; an error says what happened and how to fix it, without apology.
- Destructive actions confirm (End session, Remove pinned key); safe ones don't ask (Detach).
- An action keeps its name through the flow: the button that says "End session" is confirmed by "End session".
- TalkBack: decorative glyphs and icons have no description, the word next to them is read; a badge reads as its
  word; headings are headings; destinations are tabs in a group; banners and covers are polite live regions.

## Frame: `AdaptiveScaffold`

By the window's width class (`currentWindowAdaptiveInfoV2`), recomputed on rotation, folding, multi-window and
freeform resizing:

```
compact (< 600dp)        medium (600-839dp)              expanded (>= 840dp)
+----------------+       +--+-------------+  +-------+------+   +-----------+--------------------+
|                |       |= |             |  |<=     |      |   | >_ ngterm |                    |
|    content     |       |▌M|   content   |  |▌[] Mac|      |   |▌[] Machines|                   |
|                |       | S|             |  | >_ Ses| cont |   | >_ Sessions|      content      |
|                |       | ⚙|             |  | ⚙ Set |      |   | ⚙ Settings |                   |
+----------------+       |  |             |  |       |      |   |-----------|                    |
| ▔▔             |       |  |             |  |       |      |   | build-01 2|                    |
| [] Mac >_ Ses ⚙|       +--+-------------+  +-------+------+   |▌api-refac.|                    |
+----------------+        rail, collapsed    rail, labels open  +-----------+--------------------+
```

- **Compact:** a bottom bar, the content full width above it.
- **Medium:** an 88dp rail with icons over short labels; its menu button opens it to 248dp with the labels beside
  the icons.
- **Expanded:** a permanent 296dp sidebar: the mark and name, the destinations, a rule, then a list of its own (a
  machine's sessions) taking the rest of the height. The real split layout of panes is the content's.
- **Edge to edge:** the navigation pads for the system bars and the cutout and consumes those sides (the bottom bar
  its whole height); the content pads for the rest itself (`safeDrawingPadding`), so it meets the keyboard's top
  edge. The keyboard is not the navigation's to avoid. A rail's or sidebar's width is beside a start-side inset, not
  inside it.
- **State:** the content is movable content, so a layout change moves it rather than recreating it, and nothing in it
  is lost; the rail's open state and the destination are saveable.
- **Back:** predictive back first narrows the open rail with the gesture and closes it on release; then a destination
  other than the first goes back to the first; only from the first does back leave the app (the system's predictive
  back-to-home shows that). The manifest opts in (`enableOnBackInvokedCallback`), so Android 14 and 15 get it too, not
  only Android 16, where target SDK 36 makes it the default.

Destinations today: Machines, Sessions (what is attached, across machines, and the session view) and Settings. A
machine's own session list lives under Machines (tap a machine); tapping a session there attaches it and opens it under
Sessions. The rail (medium) and the sidebar (expanded) carry the attached sessions under the destinations on every
destination, so a session is one tap away wherever you are (The session view, below).

## Screens

### First run (`ui/onboarding/`)

Full screen, no navigation: there is nowhere to go without a key. A column of 520dp at most, left aligned (it is
reading, not a poster), the mark on top of the welcome.

- **Welcome:** "Set up a key", what the key is for, and a line with the lock glyph in verdigris that the private key
  is sealed by the device's keystore. Create a key is the filled button, Import a key the outlined one under it; while
  the key is made the button shows a spinner and its words say so. An import continues through Back: the welcome
  shows a spinner with "Importing a key", Create a key is disabled, and Import a key reopens the busy import screen.
  Success shows the public key even from the welcome; a failure leaves the welcome ready for another try.
- **Import:** the private key pasted into a mono field or read from a chosen file, and the passphrase (hidden, with a
  Show toggle). What went wrong shows under the field it is about: no passphrase or the wrong one under the
  passphrase, anything else under the key, and the text stays for another try. The key's text lives in memory only,
  never in saved state.
- **Public key:** the three steps, numbered because they are a sequence; the `authorized_keys` line on a `KeyPlate`
  with Copy and Share; its fingerprint; the one command that appends it, with Copy; Done. The machine form opens the
  same screen with Back.
- **States:** nothing while the keys are read; "Your keys couldn't be read", the store's reason and Retry when they
  can't be. No key is made or imported until a retry reads the store successfully.

### Machines (`ui/machines/`)

- **Home:** "Machines" with Add machine at the top, then a `MachineCard` per machine, 640dp at most and centered.
  The card's badge is the connection; under the address, what the state means and the one action it calls for at
  the end of the line: Not connected / Connect, The connection dropped / Retry now (the badge counts down "in 4 s"),
  the core's reason / Retry, Host key changed / Review, Waiting for sign-in (the attention diamond) / "Sign in at its
  link to connect" with no action, as the sheet over the app carries it. Connected shows the sessions' status instead: a badge per
  status the origin reports with how many have it ("2 working"), then "1 idle". Tapping a card opens the machine's
  sessions; its More menu holds Connect or Disconnect, Edit and Remove (in the error color).
- **States:** a spinner while the list is read; an error with Retry when it can't be; the empty state with Add
  machine; an offline banner over the list while the device has no network.
- **Remove** asks nothing: the machine goes at once and a snackbar offers Undo, which puts it back where it was, with
  its pinned key.
- **Add and Edit machine:** a `ScreenFrame` with Close and Save. Name, Host, Port and User (mono for what is compared
  character by character), the key (a radio choice only when there is more than one), Show public key, and when
  editing, the pinned host key. A field shows its problem once it has been left, or when Save is tried; the problem
  replaces its hint and says how to fix it ("Put the user in User, not in the host").
- **First connect:** a bottom sheet over whichever destination is shown, raised by the core's question: the attention
  diamond, "Trust build-01?", the fingerprint on a plate, the command that prints it on the machine, Trust and
  connect (filled) and Don't connect. Swiping the sheet away or back is Don't connect, which leaves the machine not
  connected rather than failed.
- **Sign-in:** a machine may ask you to sign in elsewhere before it lets the app in (Tailscale SSH's `check`: an ssh
  banner with a link, then authentication held until you have signed in in a browser). While its connect waits, a
  bottom sheet over whichever destination is shown, with the first-connect sheet's shape: the attention diamond,
  "build-01 asks you to sign in", the mono address, one line on what to do, the machine's message on a `KeyPlate`
  ("Message from build-01") with its link copper and underlined (a tap opens it, a long press selects) and **Copy
  link** under it, then a quiet "Waiting for you to sign in" with a 12dp spinner: the connect is alive and goes on by
  itself. **Open link** (filled) hands the link to the browser and leaves the sheet as it is, so coming back to the
  app shows the same wait; **Don't connect** (text), swiping the sheet away or back disconnects. The sheet goes when
  the connect ends: connected, failed (the card then gives the core's reason, "the sign-in was not completed in time"
  after ten minutes), or disconnected. A host-key question shows first when both wait. Leaving the app for the
  browser doesn't close the connection: the app keeps it while a sign-in waits, with nothing attached too.
- **Changed host key:** a hard stop, not a dialog: the error triangle, "build-01's host key changed", what that can
  mean, the key trusted until now and the one offered now, and what to do. Back to machines is the filled button;
  Remove pinned key is outlined in the error color and confirms. Removing it connects again, which asks about the new
  key.

### A machine's sessions (`ui/sessions/`)

Stacked on the Machines destination above the home (Back returns to it), so the bar, rail or sidebar stays. The
machine is the subject: the list is quiet rows on the ground, not cards, since a machine card already framed it.

```
+--------------------------------------+
| <-  build-01              + New session|
|     nick@build-01.lan:22             |
|  Attached 1                          |
|  docs-site                         : |
|  Rewrite the install guide           |
|  <> Waiting for input                |
|  On build-01 4                       |
|  api-refactor                      : |
|  o Working                           |
|      api-refactor review           : |
|      Companion of api-refactor       |
+--------------------------------------+
```

- **Top row:** Back, the machine's name as the heading with its mono address under it (wrapping at a large font scale
  rather than losing its end), and New session at the end while connected (a text button's look; a long press
  names the session).
- **List:** 640dp at most, centered. "Attached" (what this app has attached) first, then "On build-01" (the rest);
  each heading with its count, a group with nothing in it left out. A row is a `SessionRow`: the name, the context (or
  the working directory when it has none), the agent's status badge, and a More menu with Attach or Detach, Duplicate
  session, Chat view and End session (in the error color). Companions sit indented under their parent with "Companion
  of …" as their line. Tapping a row attaches it (the session view opens from there). A row being ended says "Ending",
  one being duplicated "Duplicating", with a small spinner in place of its status. Pull to refresh lists again; on a machine not connected it connects.
- **New session** takes one tap: the machine creates a session it names, the app attaches it and the session view
  opens on it, in control. Its pane takes the lead when its first attach reports that another device has it (Take
  control without the tap, the pane attaching meanwhile rather than flashing the cover); one that leads already is left as it is, and any later takeover is the user's to answer
  as usual. While the machine creates and lists it (up to 20 s) the button says Creating with a small spinner and
  takes no taps; a failed create or attach is a snackbar in the core's words, and the list stays (a session created
  but not attached shows in it). Only this device's own New session or Duplicate claims the lead: a session opened
  from a row keeps the Take control cover.
- **Duplicate session** (every session's More menu: a machine's list, the attached list, the session view's bar) asks
  nothing: the machine copies the session as desktop's Duplicate does (a fresh shell in its focused directory, right
  after it, named by the machine; a companion's copy is a session of its own, and a parent's companions are not
  copied), and the copy lands as a one-tap New session's does: attached, opened, in control. While it runs the source
  says "Duplicating" with a small spinner in place of its status (on its row in either list, or in the bar's state
  line), and every Duplicate session item and New session, on every screen, take no taps: one create at a time. A
  failure is a snackbar in the core's words and the screen stays. From the Sessions destination the copy replaces the
  source in the view, unless the view has moved on meanwhile (another session, the list, another destination): the
  copy then waits attached in the list and claims no lead.
- **New named session:** a long press on New session (TalkBack's action "New named session") opens a dialog over the
  list: one optional Name field ("Optional: build-01 names it when blank"), Create session and Cancel. While the
  machine creates and lists it the button says Creating with a spinner and the field holds still; Cancel closes the
  dialog and the create goes on. A failed create shows under the field, in the core's words, and the name stays for
  another try; once created, the session lands as a one-tap New session's does.
- **End session** confirms ("End docs-site?", "It stops on build-01, with everything running in it. This can't be
  undone.", End session in the error color). A failed End or attach is a snackbar in the core's words.
- **States**, each a `MessagePanel` in place of the list: connecting and reading are a spinner with what is under
  way (a host-key question or a sign-in waits as its sheet over it); not connected offers Connect; the device offline says so with Retry; a dropped connection counts down "Trying
  again in 4 s" with Retry now; unreachable gives the core's reason with Retry; a changed host key offers Review
  (the changed-key stop); no sessions invites New session (Creating with a small spinner while it runs); a listing that failed gives its reason with Retry. A
  machine without `ngtermctl` (the far shell's "not found") says "ngtermctl isn't on build-01", what to install and
  where, with Open the README (the server setup section) and Retry under it. A refresh that fails over a list keeps
  the list under an error banner with Retry; the offline banner shows over it as on the home.

### The terminal (`ui/terminal/`)

`TerminalView` draws a shown pane's screen; the session view (Task 18) places it. The terminal is the hero, so it is
plain: no frame, no padding of its own, no scrollbar, nothing over the text. The host gives it its room (gutters,
insets); the view fills that with whole cells from the top left and paints the slack at the right and bottom in the
terminal's ground, which is the chrome's ground in Copper and Drafting, so the two read as one surface.

- **Font:** JetBrains Mono 2.304 (OFL, `licenses/`, NOTICE), the static Regular, Bold, Italic and Bold Italic faces,
  the same release Ghostty and desktop ngterm embed: a session reads the same on the phone as on the desk. It covers
  box drawing, blocks and the common symbols on the grid, and keeps its programming ligatures (`=>`, `!=`), as Ghostty
  does by default; they take the columns of the characters they join. CJK and emoji fall back to the platform's fonts,
  centered in their two cells or squeezed into them. No Nerd Font symbols in v1: the symbols font is 2.5 MB.
- **Size:** 13 sp by default (`DEFAULT_TEXT_SIZE`, which `TerminalTextSize` derives from), or what Settings sets:
  about 50 columns on a phone held upright, 110 in landscape. It follows the system font scale like any text (at 2.0,
  about 25 columns); a pinch changes one pane's in whole sp steps between 8 and 32 (`TerminalTextSizeRange`, which
  `TerminalTextSizes` derives from), so the machine's pty is resized once per step, not per frame. A size set in
  Settings resizes every shown pane, pinched or not. Settings can swap the font for the device's monospace
  (`FontChoice`), which keeps the cell rules below.
- **Cell:** the font's advance wide, unrounded, so runs of the font's own glyphs sit on the grid; ascent to descent
  tall, rounded up to whole pixels. The pane's grid is what fits, and it goes to the pane as its size on every
  change (rotation, multi-window, folding).
- **Text:** bold and italic pick the face; faint is half alpha (Ghostty's `faint-opacity`); inverse swaps the colors;
  invisible draws nothing; strikethrough, overline and five underlines (single, double, curly, dotted, dashed), in
  the underline color when the program set one. Blinking text holds still.
- **Cursor:** a block in the palette's cursor color (copper in Copper, rust in Drafting) with the character under it
  in the ground's color; a bar, an underline, or a hollow block as the program asks. A terminal without focus shows a
  hollow block. It never blinks: nothing on screen moves on its own (Motion). On a wide character it covers both
  columns.
- **Selection:** a tonal fill in the palette's selection color under the text, which keeps its own colors (not
  Ghostty's inverse: an inverted block of copper text on a phone reads as an error).
- **Before the first frame** and while the pane has no terminal (hidden, between attaches) the view keeps what it
  last drew, on the ground; the covers for those states are the session view's (`Cover`).
- **TalkBack:** the view is announced as "Terminal". The session view asks the pane on screen to be read
  (`readAloud`): its text is then the screen's rows, refreshed at most once a second while it changes, never a live
  region (a busy program would talk over everything).

### Touch, mouse and selection (`ui/terminal/TerminalGestures.kt`, `TerminalMouse.kt`, `SelectionHandles.kt`)

The terminal keeps the gestures people already know from Android's text and from desktop terminals; nothing new to
learn, and nothing drawn until a gesture asks for it.

| Input | Does |
|---|---|
| a finger's drag | scrolls by whole rows, and a flick flings on and slows as a list does; the pane routes it as wheel turns: the scrollback, or the program when it asked for the mouse (or cursor keys under alternate scroll) |
| a long press | selects the word (a haptic tick), with handles and the toolbar; dragging on without lifting extends it |
| a handle's drag | moves that end of the selection |
| two fingers | pinch the text size |
| a tap | a click for a program tracking the mouse; otherwise it lets a touch selection go; unless it let one go, it gives the terminal the focus and opens the soft keyboard (Keyboard, below) |
| a mouse or trackpad | under mouse tracking its buttons, drags and moves go to the program (shift held selects instead); otherwise a click lets the selection go, a double click selects the word and a triple one the line, a drag selects; the wheel scrolls either way |
| the right button | the menu: Copy (while there is a selection), Paste, Select all |

- **Handles** are the terminal's own copper (the palette's cursor color, rust in Drafting): a 2dp stem on the
  selection's edge, as tall as its row, and a teardrop knob (22dp) hanging under the row, the start's to the left of
  its stem and the end's to the right, the square corner at the stem's foot, so the two point at what they hold. At
  the view's edge a knob hangs on the other side of its stem. A finger within 24dp of a knob (a 48dp target) takes
  it. A mouse selection has no handles: the mouse is precise, and the right button has the menu.
- **Toolbar:** Copy, Paste and Select all as text buttons in copper on a `surfaceContainerHigh` strip (12dp radius,
  an `outlineVariant` edge, since it floats over text of any color), 8dp over the selection, or under its knobs when
  there is no room above, centered on a one-row selection and kept 8dp inside the view. It hides while a finger
  moves the selection and comes back when it lifts. Copy copies and lets the selection go; Paste pastes and lets it
  go; Select all keeps the handles.
- **Menu:** Material's dropdown at the pointer, on `surfaceContainerHigh`; Copy is greyed with nothing selected.
- **Paste protection:** text the pane refuses as unsafe (a line break the program would take as Enter, or a control
  character) is not pasted until a dialog confirms it: "Paste text that can run a command?", why, the text itself
  in mono on a `surfaceContainerLow` plate (its first six lines), then Cancel and Paste (the toolbar's word). Safe
  text pastes without asking.

### A program reads the clipboard (`ui/terminal/TerminalPrompts.kt`, `ClipboardPreview.kt`, `data/ClipboardReads.kt`)

A program's read (OSC 52, or the Kitty clipboard protocol's OSC 5522) waits for the user, as Ghostty's
`clipboard-read = ask`. It is a sheet over whatever is on screen, the same shape as the first connect's: the attention
diamond, "Let docs-site read your clipboard?", from where ("A program in docs-site on build-01 asks for what you
copied. It sees this only if you allow it."), then what it would see, then **Allow** (filled), **Always allow on
build-01** (outlined) and **Deny**. Swiping the sheet away denies. Reads queue and show one at a time; a read the core
gives up on (its pane hidden, detached, lost or exited) closes its sheet.

- **The preview** is Ghostty's GTK confirmation in the app's terms: one page per type the read asks for and the
  clipboard holds, picked with a filter chip named by its MIME type in mono (no chips for one page), on a
  `surfaceContainerLow` plate. An image shows fitted, at most 200dp tall; valid UTF-8 shows as text in mono, scrolling
  inside the plate past 160dp (the first 64 KiB of its UTF-8, the rest counted); other data says "No preview for this kind of
  content"; an empty part says "Empty". Under the plate, the page's size ("73 bytes", "2.0 kB"), after its type when
  there is one page. The clipboard is read when the sheet comes up ("Reading your clipboard…" until then, with Allow
  and Always disabled), and Allow hands over exactly what was shown. A clipboard holding none of the types says so.
- **Don't ask again for this program in this terminal**, a switch row under the preview, shows only for a read that
  can keep a grant (the Kitty protocol's password); with it on, Allow (or Always) asks the core for that grant. A
  program holding one is answered unasked.
- A listing of the types, or a read asking for no type, discloses no data and never asks.
- "Always" is the app's to keep, per machine (`ClipboardTrust`, which Settings lists), and answers that machine's later
  reads at once. In the background Android lets no app read the clipboard, so an always-allowed read then gets an
  empty one.

A program's write isn't asked about (Ghostty's `clipboard-write = allow`): its text, HTML and first image land on the
clipboard as one clip (an image through the app's own content URI, replaced by the next write; HTML with its text, as
Android's clips carry it), and the program learns the outcome only once the clipboard took it. A write of none of those
types is refused as unsupported, and only a write with nothing in it clears.

### Keyboard (`ui/terminal/KeyMapping.kt`, `TerminalKeyboard.kt`, `TerminalInputConnection.kt`, `ExtraKeysRow.kt`)

Typing is the terminal's other half, and on a phone the soft keyboard is in charge of the screen. The row of extra
keys is the one piece of chrome that stays while typing, so it is the plainest thing in the app: a strip of legends on
the bar's tone, nothing drawn around a key until it holds something.

The plan, reviewed against the brief:

| First pass | Why it was generic | Revised to |
|---|---|---|
| Grey key caps with a shadow, ESC / CTRL / ALT in capitals | a skeuomorphic keyboard kit, and all-caps labels are a template tell | flat legends on `surfaceContainer`, sentence case (Esc, Ctrl, PgUp); a key's face appears only for a held modifier |
| A colored dot on a held Ctrl | color alone carries the state | shape carries it: an outline holds for the next key, a fill locks; TalkBack reads "On for the next key" or "Locked on" |
| Symbols in the chrome's mono | the chrome's mono draws `~` and `-` small and low, and they are what the terminal will show | the terminal's own face (JetBrains Mono) at 18 sp: the key shows the character as it will land |
| A pop-up preview over each pressed key, as soft keyboards do | decoration a row of large keys doesn't need | the ripple and a keyboard tick |
| Arrows as text glyphs | the chrome's font has none, and a fallback glyph breaks the icon set | the app's own line arrow (`ic_key_arrow`), turned |

```
phone, upright: two lines of eight

+-----+------+-----+-----+------+-----+-----+------+
| Esc |  /   |  |  |  ~  | Home |  ↑  | End | PgUp |
| Tab | Ctrl | Alt |  -  |  ←   |  ↓  |  →  | PgDn |
+-----+------+-----+-----+------+-----+-----+------+

sideways or wider (48dp a key or more): one line of sixteen

| Esc | Tab | Ctrl | Alt | / | | | ~ | - | Home | End | PgUp | PgDn | ← | ↑ | ↓ | → |
```

- **The soft keyboard:** a tap on the terminal gives it the focus and opens the keyboard. The terminal asks for plain
  text with no suggestions, autocorrect, automatic capitals or learning (it types commands and passwords), and Enter
  as a key. Every letter goes to the program as it is typed, so a shell's completion or a TUI's `/` menu follows each
  one: a word the keyboard composes (a swiped word, a CJK reading, voice) goes as it grows, and a change to it (a
  conversion, a correction, a backspace in it) is made on the program's line, with backspaces and the new text. Line
  editors take different amounts back on a backspace over a letter with its marks, a conjunct or an emoji sequence
  (Claude Code and Codex the whole character, readline the last letter with its marks, the terminal's own line editing
  one code point), so a composition goes only as far as its first such character; from there it waits at the cursor
  as a dead key's accent does, and goes when the word is done. It waits where the terminal will put it: from the
  cursor on, each character covering its own cells, onto the next row at the right edge (the screen moving up a row
  when that is past the last), never over the text before it. Backspace past it is the program's.
- **The row** sits under the terminal, over the keyboard, whenever no hardware keyboard is attached and Settings hasn't
  hidden it (it shows with the keyboard closed too: the arrows and Esc work a pager or a menu without it). Two lines of
  eight on a phone held upright: Ctrl and Alt by Tab under the thumb, the arrows an inverted T with Home and End over
  Left and Right, the page keys at the end. One line of sixteen once each key gets 48dp, unless Settings keeps two lines
  at every width. A key is its whole cell (no gaps between keys, 45dp at the narrowest 360dp phone), 48dp tall.
- **Legends:** words in Atkinson Hyperlegible Next semibold, all at one size, the widest's ("Home") fitted to its key
  (so a large font scale grows them only as far as they fit); symbols in JetBrains Mono 18 sp; arrows the line icon.
  `onSurface` on `surfaceContainer`.
- **A key acts as it goes down**, with a keyboard tick; the arrows and the page keys repeat while held (after 400ms,
  every 50ms). Esc, Tab and the symbols don't.
- **Ctrl and Alt are sticky:** a tap holds it for the next key (a 2dp copper outline, copper legend, a toggle tick); a
  second tap locks it (a copper face, `onPrimary` legend); a third lets it go. A held modifier joins the next key from
  the row, the soft keyboard (the character's key on a US layout: Ctrl and a typed `c` is Ctrl+C) or a hardware key,
  and a once-held one lets go after it.
- **A hardware keyboard** types straight into the focused terminal, and the row stands aside. It types what its
  layout types: AltGr's characters (German AltGr+Q is `@`) are text, not Alt chords, and a dead key's accent waits at
  the cursor (in the terminal's font, on its ground, underlined 2dp in the cursor's copper) until the next key takes it (`´` then `e` is `é`; a space, the same accent again, or a
  key it doesn't go on types the accent itself first). Its chords are desktop's Linux ones, by the character the
  layout types: Ctrl+= (or Ctrl++) and Ctrl+- step the text size a whole sp (8-32), Ctrl+0 puts it back to the size
  Settings sets (on Dvorak, Ctrl+[ and Ctrl+] stay the program's); Ctrl+Shift+Left and Right go to the previous and
  next pane, Ctrl+Shift+Up and Down to the previous and next session (the session view's to wire; a chord with nowhere
  to go is the program's).
- **Order:** a key from the row or a hardware key, a paste that goes in, or a tap, click or wheel the program takes
  while the soft keyboard is composing ends the composition, so the keyboard starts afresh after it rather than
  correcting text before it or where the cursor was. What waits at the cursor goes first, before any paste, tap, click,
  drag or scroll, a scroll of the view included (where the wheel goes is only known once the program has it), so it
  lands where it was typed. A mouse click gives the terminal the focus without opening the soft keyboard; a tap opens
  it.

### The session view (`ui/session/`)

An attached session's panes, live. Everything here exists to get out of the terminal's way: one bar of chrome above
it, the extra keys under it, and covers only when a pane can't be used as it is.

The plan, reviewed against the brief:

| First pass | Why it was generic | Revised to |
|---|---|---|
| A toolbar with the name, a status chip, and icon buttons for split, keyboard, detach and end | the "app bar of actions" every client draws, eating a terminal row per icon | one bar: the name over a line of status and context, the pane switcher, and one More menu (Detach, Duplicate session, End session) |
| Pager dots for the panes | dots say how many, not which, and are too small to tap | numbered pane tabs (the panes are a sequence: the origin's order), the bus bar under the one on screen |
| A tinted frame or an accent border around the focused pane of a split | a second loud mark, and a frame costs the terminal its edge cells | the bus bar again, in a 4dp gutter at the focused pane's leading edge; the other pane's cursor goes hollow |
| The HUD as a floating toast in the origin's colors | an overlay over the text, colored by someone else | a `Banner`-shaped strip under the bar, on `surfaceContainerHigh`, the origin's words and its spinner |
| One full veil for every reason a pane can't be used | hides the screen the user is trying to keep their place in | the full `Cover` where the pane can't be trusted (another device drives it, it is attaching, its program ended); a light scrim and a strip while it reconnects, so the last screen stays readable |
| A Chat / Terminal segmented control for a pane running an agent | 96dp and two labels in a bar that already holds the name, the pane tabs and More; it truncates at font scale 2.0, and its selected half would be a second "you are here" mark beside the bus bar | one 48dp icon button named for what a tap does: "Show terminal" (the Sessions glyph) over a chat, "Show chat" (`ic_chat`) over a terminal, and only while the focused pane runs an agent |

```
compact                            medium (rail open)               expanded
+------------------------------+   +-------+--------------------+   +-----------+---------------------------+
| <- docs-site          1 2  : |   | <=    | docs-site    1 2 : |   | >_ ngterm | docs-site            1 2 : |
|    <> Waiting  Rewrite the.. |   | ...   | <> Waiting         |   | Machines  | <> Waiting  Rewrite the.. |
|  Building the docs (HUD)     |   | ----- |--------------------|   | Sessions  |---------------------------|
|                              |   |▌docs-.|                    |   | --------- |▌             |            |
|        pane 1 (swipe)        |   |  api-.|  pane 1  | pane 2  |   | build-01  |  pane 1      |  pane 2    |
|                              |   |       |          |         |   |▌docs-site |              |            |
|------------------------------|   |       |--------------------|   |  api-refa.|              |            |
| Esc / | ~ Home ↑ End PgUp    |   |       | extra keys         |   | nas       |---------------------------|
| Tab Ctrl Alt - ← ↓ → PgDn    |   +-------+--------------------+   +-----------+ extra keys               +
+------------------------------+
```

- **Where it lives:** under Sessions. With nothing open, Sessions lists what is attached, grouped by machine (a
  `SectionHeader` per machine, a `SessionRow` per session with its status and a menu: Detach, Duplicate session, Chat
  view, End session), or the
  empty state pointing to Machines. The rows are the tree a machine's sessions shows, kept to what is attached: in the
  machine's order by family, an attached companion indented under its attached parent with "Companion of …" as its
  line. A companion whose parent isn't attached keeps that line but stands unindented in its family's place, so it
  never reads as another session's. There is nothing to expand or collapse, as on a machine's list. Tapping a session opens its view. On a phone the view is a screen of its own over
  the list (Back returns to it) and the bottom bar steps aside, so the terminal has the whole height; from the medium
  width up it is the content beside the rail or the sidebar, which list the attached sessions (below).
- **The bar** (`surfaceContainer`, under the status bar): Back on a phone; the session's name (titleMedium) over its
  agent status (`AgentStatusBadge`, or the stream's own state while it isn't live: Connecting, Reconnecting, or Can't stream) and its
  context in one line; the pane switcher; the view switch; More. The bar is the one place the session's state shows;
  nothing in the terminal repeats it.
- **The pane switcher** (`PaneSwitcher`): only with two panes. A tab per pane, numbered in the origin's order, 48dp
  square, the pane's title as its TalkBack label ("Pane 1 of 2, vim"). The pane on screen (a phone) or with the
  focus (a split) has the copper number and the bus bar under it. On a phone a swipe across the terminal pages
  between the panes too: a mostly sideways drag of 64dp or more, or a fast sideways fling, goes to the next or
  previous pane; other drags scroll the terminal.
- **The view switch** (`SessionBar`, `ui/chat/PaneChats.kt`): while the focused pane runs Claude Code or Codex (its
  origin announces the agent) and its session's chat view is on (Chat view, below), the pane shows as a chat and the bar has one more
  48dp icon button, between the pane switcher and More. It names what a tap does: "Show terminal" with the Sessions
  glyph over a chat, "Show chat" with `ic_chat` over the terminal; TalkBack reads those words. Picking the terminal
  holds for that pane until its agent starts its next conversation (or another agent starts there); picking the chat
  lifts it. When the agent ends, or its chat ends or can't open (an origin without the chat view), the pane shows its
  terminal and the switch offers the chat back while the agent is still announced. A pane without an agent, or every
  pane of a session with its chat view off, has no switch. Switching never resizes the pane or moves its lead: the terminal stays
  attached at its grid behind the chat, and only a chat action takes control. The chat itself is The chat, below.
- **The split** (`SplitLayout`, medium and expanded): the origin's layout as it is, two panes side by side
  (`vertical`) or stacked (`horizontal`), halves split by a 1dp `outlineVariant` rule, since the terminal's ground is
  the chrome's. Each pane of a split keeps a 4dp gutter at its leading edge where the focused one shows the bus bar
  (32dp, from its top); a touch on a pane selects it (even one that already has the input focus), and the other's
  cursor goes hollow. A pane tab toggles the split on this device alone, the origin's untouched: a tab in the
  split shows its pane alone and selected, the shown pane's own tab brings the split back, and the other tab shows
  that pane alone instead; TalkBack names the tap ("Show this pane alone", "Show both panes"). The choice is kept per
  session. A hidden split (the origin zoomed a pane) shows the focused pane alone, and the switcher still moves
  between the two. On a phone every layout
  is one pane at a time. A pane alone keeps 4dp at both sides, so its text stays off the screen's edge.
- **The HUD** (`HudBanner`): a strip under the bar while the origin shows one, `surfaceContainerHigh`, 6dp radius,
  the message in bodyMedium semibold and its detail under it, a small spinner when the origin runs one; announced
  politely. The origin's colors and position are desktop's: on a phone the strip is where it can't cover text. Under
  320dp of height (a phone on its side with the soft keyboard up) the strip steps aside, so the terminal keeps its
  rows; it comes back with the room.
- **Covers** (`LeadCover`), over a pane, each with the action that fixes it:
  - *Another device drives it* (the pane follows): the lock, "Another device is driving this pane", "Its size and
    keys are the other device's. Take control to type here; the other device follows.", **Take control**. On a
    machine running `ngterm-headless` this is the first thing a pane shows: its own terminal attached first.
  - *Attaching* (no screen yet): a spinner, "Attaching", "Reading this pane from build-01."
  - *Reconnecting* (the link dropped): not a veil. A 40% scrim that takes the touches and a strip at the pane's top
    with a spinner, "Reconnecting to build-01" and "The last screen stays until it is back." The screen stays
    readable; nothing is typed into it.
  - *Program exited*: "The program exited", with its code or signal ("Exit code 1", "Stopped by SIGKILL"), "The last
    screen stays as it was.", **Detach**.
  - *Can't show it* (the core refused the pane): "Can't show this pane", the core's words, **Retry** (which reads the
    machine's sessions again first). A pane a split added since the machine was last read is looked for in a fresh
    read before it can't be shown, and a pane refused while its machine was reconnecting is tried again once the
    machine is back.
- **Session states** in place of the panes (`MessagePanel`): opening (a spinner, "Opening docs-site"); a machine whose
  ngterm can't stream ("build-01's ngterm can't share live sessions", update it there, **Detach**); and gone ("docs-site
  is gone", "It ended on build-01, so there is nothing left to show.", **Close**), which also covers a session that
  left the machine's list while it was open.
- **More:** Detach (at once: it goes on running there, and the view closes), Duplicate session (the copy opens in its
  place, as on a machine's list), Chat view and End session (in the error color, confirmed as on a machine's list).
- **Chat view** (`ChatViewItem`, in every session's More menu: the bar's, the attached list's, a machine's list): the
  session's own Agent chat view, a menu item with a checkbox at its end, checked while the session's panes running an
  agent show as a chat; TalkBack reads it as a checkbox. It starts at Settings' Agent chat view, and a tap sets the
  other for that session alone, kept on this device (`SessionChatViews`, the newest 256 sessions); one set back to
  Settings' value follows Settings again. The menu closes on the tap.
- **The keyboard:** one `TerminalKeyboard` for the view, shared by every pane and the extra-keys row, so a held Ctrl
  stays as the selection moves; the row sits under the panes over the soft keyboard, and steps aside while every pane
  on screen shows its chat (The chat, below). Ctrl+Shift+Left and Right go to
  the previous and next pane, Ctrl+Shift+Up and Down to the previous and next attached session.
- **One selected pane:** each session has one selected pane, the one a tab, a swipe, a chord or a touch on a pane
  picked, and everything typed goes to it alone: the soft keyboard's text, the extra keys and a hardware keyboard's
  keys. The input focus follows the selection, never the reverse. While the soft keyboard is up or a hardware keyboard
  is attached, the pane selected takes the focus once its terminal is there, so typing goes on in it; with the soft
  keyboard down, a pane the selection left lets the focus go without the keyboard opening, and the next touch on a
  pane focuses it. Selecting another pane ends a word the soft keyboard is composing in the pane it was typed in, and
  finishes it there. Until the selected pane's terminal is there (on its way, or it couldn't be shown) the keys go nowhere,
  but the chords still move between panes and sessions.
- **The attached sessions in the rail and the sidebar:** the sidebar groups them by machine (the machine's name as a
  `SectionHeader`, then a `SessionRow` each with its status, the bus bar on the one open), companions indented as in
  the list. The rail, collapsed, gives each a 64dp cell in the same order, its status glyph over its name cut to fit; opened, the sidebar's rows. Either opens the session
  under Sessions.
- **State:** the open session, the focused pane of each session, the shown panes, each pane's view and its open chat
  live in the app's process, not the screen, so rotation, folding, a size-class change or a trip to another
  destination keeps them, and a pane keeps its lead.
- **Clipboard:** a program's OSC 52 write sets the device's clipboard (Android shows its own notice); a write to the
  selection or primary clipboard, which Android lacks, is dropped.

### An agent asks (`ui/asks/`)

An origin hands this app an ask (`ngtermctl ask`) for any attached session: the agent has stopped and waits for an
answer. The ask rule says where it shows: a **sheet over the session** while that session is on screen (its view
open under Sessions and the app in the foreground), and a **notification** otherwise, including while the app is in
the background, which is as good a place to answer as any.

The plan, reviewed against the brief:

| First pass | Why it was generic | Revised to |
|---|---|---|
| A centered alert dialog, the buttons in a row at its foot | covers the terminal the question is about, and a row of three caller-written labels truncates on a phone | a sheet from the bottom edge, the shape the clipboard and host-key questions already have; the terminal stays readable above it |
| Every button filled alike | flattens what the origin marked: a default, a destructive choice, a cancel | the default filled copper, the destructive outlined in the error color (filled in it when it is also the default), the cancel button a quiet text button, the rest outlined; in the origin's order, one per line, full width |
| A "Question from docs-site" eyebrow over the title | a template label; the session is the one on screen | the attention diamond (the status "Needs you") over the title, nothing else |
| Choices as list items between dividers | the card-and-divider kit | 56dp rows on the sheet's tone, the marker at the start in a 48dp cell, the label (bodyLarge) over its subtitle (bodySmall); the marker alone carries the pick |
| The free-form answer as a separate field under the list | detached from the list it belongs to | the list's last row: its marker, then the field in place of a label |

```
+--------------------------------------+
|                ----                  |
|  <>                                  |
|  Which branch should the fix go on?  |
|  (o) main                            |
|      The release line                |
|  (*) develop                         |
|      Has the new installer           |
|  ( ) [ Another branch            ]   |
|  [        Use this branch         ]  |
|               Cancel                 |
+--------------------------------------+
```

- **Content:** the diamond; the ask's title (headlineSmall, a heading); its message (bodyMedium, `onSurfaceVariant`);
  its form; its buttons. The sheet scrolls when it doesn't fit.
- **A text field** starts from the origin's text with its placeholder (or "Your answer"); one line, or an editing area
  of three to eight lines. An edit that would add a character the field can't hold (a control character; a line break
  in a one-line field) is refused whole, as on desktop, and the text stops at 64 KiB. Enter in a one-line field
  presses the default button (or the one answering button).
- **A choice list** is radio buttons or checkboxes, as the origin asks, starting on its checked ids (a radio list on
  its first item when none is). The free-form row's marker takes a tap only while the row holds text: typing selects
  it, and in a radio list takes the pick over; clearing it gives the pick back. Blank text never answers.
- **Answers** follow desktop's: a button answers with the field's text, or the checked ids and the free-form row's
  text; the cancel button, Back or swiping the sheet away answer as Escape. An ask the origin takes back (answered
  elsewhere, ended there, its stream gone) closes its sheet; one answered here goes once.
- **Not here:** the origin's style (terminal or GUI), button alignment, width and hotkeys are desktop's; the sheet is
  the app's one look, and its buttons stack at every width.

### The chat (`ui/chat/`)

A pane running Claude Code or Codex, read as a conversation (The view switch, above). Underneath it is still the
agent's TUI in the pane, so the chat is a second reading of the terminal, not a messenger: everything the agent typed
or ran is in the terminal's own face (JetBrains Mono, `ChatType`), and only what people and the agent wrote as prose is
in the chrome's.

The plan, reviewed against the brief:

| First pass | Why it was generic | Revised to |
|---|---|---|
| Bubbles on both sides, avatars, a time under each | the messenger kit; a reply is a document, often with code, and a bubble squeezes it | only the prompt is a bubble (at the end of the line, `surfaceContainerHigh`); a reply is full-width Markdown on the ground; no avatars, no times |
| Each tool call a card with a header, a chevron and a shadow | the card kit, and thirty cards bury the replies | one 48dp line on the ground: the kind's glyph, the title in the terminal's face, the status's glyph and word at the end; a tap opens what it holds on a plate under it |
| Code in a syntax-highlighting theme | a second palette fighting the app's, and color carrying meaning | the terminal's face in one color on a `surfaceContainerLow` plate; a diff is the only colored text: verdigris added, oxide removed, each line keeping its `+` or `-` |
| A spinner on every running tool | motion nobody asked for, scattered down the list | the Connecting ring and "Running"; nothing in the list moves on its own |
| The permission prompt as a dialog over the chat | hides the conversation the question is about | a Needs-you card pinned at the chat's foot, in the ask sheet's shape and order (`ui/asks/`) |
| A paper-plane Send beside an attach clip and an emoji face | the messenger kit's glyphs, for things the agent's prompt doesn't take | the messenger's layout, which the user asked for, with the app's own glyphs: one pill holding Attach at its start and at its end one place that Commands (a `/`) gives up to Send once there is text, as Telegram's bot chats share theirs, so the prompt keeps the width; Send is the return key's arrow (`ic_send`) in copper on the pill, as a prompt goes in with Enter in the terminal; no emoji face |
| The account as a header card: an avatar, the email, a big percentage on a ring gauge | the dashboard's KPI tile, shouting over the conversation it sits on, for a number that changes once a turn | one quiet line over the rows: the email at the start in `onSurfaceVariant`, at the end "Weekly limit 72%, resets Mon 14:00"; no meter and no hairline |
| Email, limit and reset joined in one meta string with middle dots | template chrome, and three facts read as one | two ends of the line: who is signed in, and how much room is left; the words turn the attention copper from 80% and oxide when spent |
| A "Claude is typing" bubble with three bouncing dots at the end of the list | the messenger kit, and motion that loops while nobody asked for it | the activity line pinned at the chat's foot, standing still and with no plate: the Running ring and what the agent is doing, Stop at its end |
| A count badge ("3") on Stop for what runs in the background | a number alone says neither what runs nor that the reply above isn't the end | words: "Still running in the background" and "2 subagents, 1 command" on one line; a tap lists each as its tool line |
| The `/` menu as a popup with an icon per command and the matched letters in bold | a second set of glyphs for a list read by name, and a highlight that fights the terminal's face | a plate (`surfaceContainerHigh`, 12dp corners) rising from the composer over the foot of the rows, the activity line's Stop left in reach; each row the command as it will go in, in the terminal's face, a line of what it does, and where it comes from as a word |

```
+--------------------------------------+
|  Start of the conversation           |
|  ~/docs-site                         |
|           +------------------------+ |
|           | Fix the install guide. | |
|           +------------------------+ |
|  ... Thinking                        |
|  I'll look at the installer first.   |
|  >_ grep -n PATH install.sh   o Done |
|  /  docs/install.md           o Done |
|  Y  Look for which in ...     o Done |
|     2 steps                          |
|  >_ npm run build          O Running |
|+------------------------------------+|
|| <>                                 ||
|| Allow Bash?                        ||
|| rm -rf build && npm run build      ||
|| [             Yes                ] ||
|| ( Yes, and don't ask again ...   ) ||
|| ( No, and tell Claude what to do ) ||
|+------------------------------------+|
+--------------------------------------+
```

- **Where:** in the pane's slot, in its terminal's place. The terminal stays attached behind it and keeps the grid the
  slot gives it, so switching never resizes the pane, and a chat action that takes control lays the program out for
  this device. No cover goes over a chat (reading never takes control) except a pane that couldn't be shown, whose
  Retry stays. The extra keys type into no chat: they step aside while every pane on screen is a chat, and the
  terminal's grid still leaves them their room. The chat takes the input focus in its terminal's place, so a hardware
  keyboard's Ctrl+Shift arrows still move between panes and sessions, ahead of a question's field; its other keys are
  the field's.
- **The list** (`ChatList`): oldest at the top, one column 640dp at most, centered, with the phone's 16dp gutter. It
  opens at the newest row and follows new ones while it is at the end; a finger on it lets it be at once (a row that
  comes mid-fling doesn't pull it back), and a scroll that stops at the end follows again. Which rows are open
  survives scrolling and rotation.
- **Selecting text:** every row's text can be selected and copied: a long press selects the word under it, with the
  system's handles and its menu (Copy, Select all); Select all takes that row alone (a prompt, a reply, a tool's title
  and its open output or diff, a notice), never the whole chat. A tap still opens and closes a line.
- **Prompts** (`PromptBubble`): at the end of the line on `surfaceContainerHigh`, 12dp corners, bodyLarge, no wider
  than 480dp, with "With 2 images" under them when they carried any. TalkBack reads "You: …", then the images.
- **Replies** (`ReplyText`, mikepenz multiplatform-markdown-renderer): full width on the ground, paragraphs and lists
  in bodyLarge, headings at the title sizes (never the display ones: a reply is not a page), links copper and
  underlined, quotes in `onSurfaceVariant`. Code blocks are the terminal's face at 13 sp on a `surfaceContainerLow`
  plate with 6dp corners, scrolling sideways, never highlighted; inline code the same face on `surfaceContainerHigh`.
  Images aren't fetched. A reply is parsed as it is first drawn, so its row has its height at once.
- **Tool lines** (`ToolRow`, on `ChatLine`): the kind's glyph (18dp, `onSurfaceVariant`, read as its word: Command,
  Edit, Read, Search, Web, Subagent, Tool), the title in the terminal's face at 13 sp on one line (the tool's name
  when the title isn't known), and the status at the end: Running on the Connecting ring, Done on the Online circle,
  Failed on the Error triangle. A line that holds something is a 48dp target: a tap opens its diff, its output (the
  terminal's face at 12 sp on a plate, scrolling sideways) and "Cut short here: the terminal has the rest." when the
  origin cut them, indented to the title; the title then shows whole.
- **Diffs** (`DiffView`): each file's header lines (a diff of several edits or files has one per block) and the hunks'
  `@@` lines quiet (`onSurfaceVariant`, the headers bold), added lines verdigris, removed lines oxide, kept lines
  `onSurface`; at most 400 lines, then how many more.
- **Thinking:** one quiet line, the three-dot glyph and "Thinking" (Claude) or Codex's summary title in bodyMedium
  `onSurfaceVariant`; a tap shows a long title whole.
- **Subagents:** the task's line says "2 steps" under its title; a tap shows the steps (every kind of row) behind a 2dp
  `outlineVariant` rule. Steps whose task is older than what was read sit under "A subagent's earlier task" until the
  task comes.
- **Questions** in the conversation: a line with the Chat glyph, the first question, and Needs you (the attention
  diamond) while it waits, then Answered; a tap shows the questions and the answer the agent got.
- **Notices:** a quiet line with its glyph: an interrupt (a square), a compaction (two arrows meeting at a line; a tap
  shows its summary), an API error (the alert glyph, in oxide), a summary (the page).
- **A new conversation** (the origin's reset): "New conversation" (labelMedium, a heading) and a rule; what came
  before is gone.
- **The account** (`AccountLine`), over the rows in the reading width, while the origin read it (Claude Code's
  email from its global config and its weekly limit from its usage endpoint; Codex's from its `auth.json` and its
  rollout's rate limits), as one quiet line with no rule and no meter (the user asked for it as minimal as the
  activity line): the email (bodySmall, `onSurfaceVariant`, one line, cut with an ellipsis) at the start; at the end
  "Weekly limit 72%" (bodySmall, `onSurfaceVariant`), with ", resets Mon 14:00" after it when the origin read when (the
  weekday and the locale's short time within six days, else the date); the words take the attention copper from 80%
  and oxide at 100%, the only color on the line. When the two ends don't fit on one line (a long email, a large font)
  the limit goes under the email, so neither cuts the other short. Either end shows alone when only it was read.
  Nothing on it moves.
  TalkBack reads it as one sentence: "Signed in as dev@example.com. 72% of the weekly limit used, resets Mon 14:00."
- **Earlier messages:** the list's first row asks for the page before as soon as it is on screen, however it got
  there (a jump or a finger), and a new conversation's once its first row is in: "Loading earlier messages" with a
  16dp spinner, then the page goes in above and the row that was on top stays where it was. At the start it says
  "Start of the conversation" over the working directory in mono; a page that failed gives the core's reason in oxide
  and Retry.
- **Needs-you cards** (`NeedsYouCard`), pinned under the list, an approval before a question: the ask sheet's tone
  (`surfaceContainerHigh`) and 20dp top corners, rising from the chat's foot as the sheet does from the screen's; the
  attention diamond over the title (titleMedium, a heading). It takes at most 60% of the chat's height and scrolls
  inside.
  - *Approval* (`ApprovalCardView`): "Allow Bash?", the call's title in the terminal's face, its detail (a command, or
    a diff as `DiffView`) on a plate up to 160dp tall, then the dialog's options in its order, one per line, full
    width: the first, the dialog's own default, filled; the rest outlined. Until the pane's screen is read the agent's
    own answers stand in (Yes and No for Claude Code; "Yes, proceed" and "No, and tell Codex what to do differently"),
    worded as its dialogs word them, so a tap still finds its option.
  - *Question* (`QuestionCardView`): "Claude Code asks", then each question: its header (labelMedium), the question
    (titleSmall), the ask sheet's rows (`ChoiceRow`: radio buttons for one pick, checkboxes for several, the label over
    its description), and the "Type an answer" row (its marker, then the field; typing picks it, and in a radio list
    takes the pick over). **Send answers** (filled) once every question has an answer.
  - While an answer is on its way the buttons hold still and "Sending" with a 16dp spinner shows under them.
- **The activity line** (`ActivityStrip`), pinned at the chat's foot just over the problem banner and the composer,
  in the reading width, on the ground with no plate and on one line (the user found the plated, two-line strip too
  big: the composer is what the foot is for): so a finished reply isn't read as the work being done. The Connecting
  ring at 12dp (still, as on a Running line), its middle over the composer's paperclip, then "Claude Code is working"
  (labelLarge, `onSurfaceVariant`) while the agent's status says it works on a turn, else "Still running in the
  background" while calls of the conversation have no result yet (a subagent or a command the turn left going, until
  its notification or a stop), then how many (bodySmall, "2 subagents, 1 command", a subagent's own steps counted as
  its task), each cut with an ellipsis rather than wrapped, the counts first: the title is what tells the two states
  apart. With anything running the line is a 48dp toggle (Show
  what's running) that lists each under it as its tool line: the kind's glyph and the title in the terminal's face.
  **Stop**, the word in copper (a text button, labelLarge), ends the line while the agent works on a turn and the turn
  wasn't interrupted (no hook tells the origin of an interrupt, so its status stays working: Stop comes back once the
  agent works again or a prompt is typed, not for a `/command` alone); it holds while an action is on its way. Nothing
  to say shows nothing, and a card waiting on the user takes the foot alone.

  ```
  |   o Claude Code is working  1 subagent    Stop |
  | .--------------------------------------------. |
  | ( @  Prompt, or / for commands             / ) |
  | '--------------------------------------------' |
  ```

- **Problems:** what the last action couldn't do, as a `Banner` over the card: in the Attention tone when the user can
  act on it (the pane doesn't show the prompt any more, a dialog is open, the keys went but the screen didn't confirm
  them), in the Error tone when it only reports (another device kept control, the program isn't reading, the pane is
  gone, the core refused). When the terminal is the way on it offers **Answer in terminal**; it can be dismissed.
- **States:** the backlog on its way with nothing yet: a spinner and "Reading the conversation from build-01"; nothing
  said yet: the Chat glyph, "Nothing said yet", what will show, and Show terminal; the link reconnecting: a strip over
  the top of the rows (`surfaceContainerHigh`, 6dp corners, a 16dp spinner, "Reconnecting to build-01", "What's here
  stays; new messages follow once it's back."), the rows readable under it; the chat ended (the agent left, or the
  origin can't share its conversation): "This chat ended" and Show terminal, which the pane then does by itself.
- **The composer** (`Composer`), under the cards on the bar's tone (`surfaceContainer`), in the reading width, in a
  messenger's shape (the user asked for the composer of a chat app, overriding the first pass's "no messenger kit" for
  this one part): one pill (`surfaceContainerHigh`, no outline, 24dp corners, so a one-line field is fully round, 48dp
  tall) with its buttons inside it, each a 48dp target in `onSurfaceVariant`, and never more than two beside the prompt
  while it is typed, so on a phone the prompt keeps most of the width. At its start **Attach** (the paperclip,
  `ic_attach`; see Attaching a file below), there whatever the field holds, as a path goes in around the text. Then the
  prompt (bodyLarge on 24 sp lines, "Prompt, or / for commands", the copper caret) that grows to six lines and then
  scrolls inside, so a long prompt leaves the conversation in sight; the buttons stay on its last line. (Stop is the
  activity line's, over the pill.) At its very end one place that two buttons share, as Telegram's bot
  chats share theirs: **Commands**, a `/` in the terminal's face, while there is nothing to send (it puts the `/` in,
  takes the focus and opens the menu), and **Send** (`ic_send` in copper, `primary`, with no plate of its own: the pill
  is its ground) once there is, the `/` included. The one leaving shrinks and fades as the other grows in from its
  middle (120ms; a cut with animations off), both 48dp, so the prompt never changes width as it is typed.
  The field takes line breaks as the clipboard
  brings them and refuses an edit that adds any other control character. Send takes the text with its ends trimmed;
  the field lets it go once the agent has it and keeps it when nothing was sent. While an action is on its way the
  field holds still and Send turns to a 16dp spinner ("Sending"), so a second tap sends nothing. While a card waits,
  "Answer the card above first: a prompt now would go to its dialog." stands over the field, and Send holds;
  the field still takes a draft. A draft survives the app being put away, and comes back only to its pane and
  conversation. The chat's input focus is the field's: the session view moving the focus to a chat pane while typing
  lands in it, and a touch in a field of the chat keeps the focus where it put it.

  ```
  +--------------------------------------+
  |  ... the rows ...                    |
  | +----------------------------------+ |
  | | /compact [instructions]  Built-in| |
  | | Clear history but keep a summ... | |
  | | /context                 Built-in| |
  | | Show how the context window i... | |
  | | 2 commands                       | |
  | +----------------------------------+ |
  | .----------------------------------. |
  | ( @  /co                         <-) |
  | '----------------------------------' |
  +--------------------------------------+
  ```

- **Attaching a file** (`AttachButton`, `UploadLine`, `ChatUploads`): the paperclip opens a small menu, **Photo or
  video** (the system photo picker) and **File** (the documents UI, any type); neither asks for a storage permission.
  The picked file goes to the pane's machine over its ssh link (`ngtermctl agent upload`, stored a week under the
  origin's state directory, at most 32 MiB: a larger one is refused before a byte is read). Over the pill a line shows
  it on its way: the paperclip at 16dp, "Uploading shot.png" (bodySmall, one line), the percentage (labelMedium,
  `onSurfaceVariant`) and **Cancel upload** (Close, 48dp), over a determinate bar; the paperclip holds meanwhile, as
  one file goes at a time. Once every byte has gone the line says "Saving shot.png" over the full bar, with no
  percentage and no Cancel (the line keeps its 48dp): the machine has the whole file then and stores it whatever the
  phone does, so Cancel would promise what it can't keep. Done, the line goes and the stored path goes into the field at the cursor with a space on
  each side (in double quotes when it holds a space), for the user to write around before Send; Claude Code reads an
  image's path as the image. Stopped short, the line says why in the Error tone ("Couldn't attach shot.png: …", a
  polite live region) with Dismiss; Cancel, offered only while bytes are still to go, leaves nothing on the machine.

  ```
  |  [@] Uploading shot.png         42%  X |
  |  =============------------------------ |
  | .------------------------------------. |
  | ( @  Look at                       <-) |
  | '------------------------------------' |
  ```

- **The `/` menu** (`SlashCommandMenu`): a `/` at the start of the field, with no space or line break after it, opens
  it over the foot of the rows, leaving the activity line and its Stop in reach under it (over the card while one
  waits, as that leaves the line out); what follows the `/` narrows it as it is typed (the IME's unfinished word
  included), ranked by the core's `filter_commands`, the palettes' ranking. A space, a line break or deleting the `/`
  closes it, and so do Escape and Back until the text changes (Back only the selected pane's: a split's other pane
  keeps its menu open). Rows (56dp): the command as it goes in (`/compact`, a
  Codex skill's `$name`) in the terminal's face at 13 sp with its argument hint quiet after it, a line of what it does
  (bodySmall, one line), and its kind at the end (labelMedium: Built-in, Skill, Command, Plugin). About six show, the
  rest a scroll away; the highlighted row, the one Enter or Tab picks, is on `surfaceContainerHighest`. Picking one
  puts it in the field with a space after it, the cursor at the end, and the field keeps the focus and the keyboard.
  Under the rows a quiet line says how many match or that none does ("No command matches /deploy. It goes as
  typed."); with no catalog yet it says the commands are on their way, with a 16dp spinner, and an origin that lists
  none (an older one, or a link that went) says they aren't listed here. Never an error: the prompt goes as typed.
- **A hardware keyboard:** Ctrl+Enter sends; while the menu lists commands, Up and Down move its highlight and Enter or
  Tab picks it; Escape closes the menu. Enter alone is a line break.
- **TalkBack:** the chat is "Chat with Claude Code"; a prompt reads "You: …"; a line reads its kind, its title and its
  status, says Expanded or Collapsed, and names its tap (Show output, Hide steps, Show summary); the cards' titles are
  headings; the reconnecting strip, Sending and the problems are polite live regions. Commands, Attach a file, Cancel upload, Send and Stop are named for what
  they do ("Sending" while on its way); the line over the field and the menu's count are polite live regions; a menu
  row reads whole (the command, its hint, what it does, its kind), says Selected when highlighted, and names its tap
  Insert. The rows take no input focus, so a hardware keyboard stays in the field.

### Notifications (`platform/Notifications.kt`, `data/Alerts.kt`)

Each machine has its own channel, named after it ("Questions, notifications and agent status from build-01"), so
the system's settings can quiet one machine. Notifications use the small `>_` and the brand copper as accent; a tap
opens the session and the pane it is about (through the session view's selection, never the keyboard's focus), or
Machines when that session is no longer attached. A pane shown as its chat opens at its newest row, following again
wherever it was scrolled to, with the card the agent waits on pinned at its foot; a pane showing its terminal (Agent
chat view off, its terminal picked, or an origin with no chat) opens as its terminal.

| What | Title / text | Lasts |
|---|---|---|
| An agent's notification (`ngtermctl notify`) | its title (or the session's name) / its body; the session as the header's subtext | each its own, until the session is detached |
| Waiting (status `blocked`) | "docs-site is waiting for you" / the session's context, or "On build-01" | one per session: the next status replaces it, any other takes it down, and so does opening the session or detaching it |
| Done (status `completed`) | "docs-site finished" / as above | as above |
| A machine lost for good while sessions are attached | "Lost build-01" / the core's reason, or that its host key changed | until it connects again; the tap opens the app |
| An ask out of sight | its title / its message; "docs-site on build-01" as the subtext | until answered, taken back, or shown as the sheet |

- **Asks in a notification:** a button ask gets an action per button; an input ask an inline reply per answering
  button (its placeholder as the hint; a one-line field's line breaks become spaces) and its cancel button as a plain
  action; a choice ask, or one with more than three buttons, says "Tap to answer in ngterm." (as its text, or under
  its message) and opens the session, where the ask is then its sheet. An ask says it once (`onlyAlertOnce`) and isn't cleared by a tap.
- Waiting, Done, agents' notifications and lost machines can each be turned off (Settings); asks can't, since they
  wait for an answer. A status change of the session on screen posts nothing: the user is looking at it.

### Keeping sessions connected (`service/LinkService.kt`)

While any session is attached, a foreground service keeps the process, and with it the ssh connections, alive: one
quiet notification on its own low channel ("Attached sessions"), "2 sessions attached" over their names, with
**Detach all**; tapping it opens the app. It stops itself when the last session is detached. Its type is
`specialUse` (a terminal's long ssh link fits no other type, and `dataSync` is time-limited on Android 15).

**The permission** (Android 13+) is asked once, when it starts to matter: the first time a session is attached. A
sheet says why first ("Get told when an agent needs you": what reaches you in the background, that a question can be
answered from the notification, and the one quiet notification), then **Turn on notifications** raises the system's
question and **Not now** (or swiping the sheet away) leaves it for Settings. Either way it isn't asked again.

### Settings (`ui/settings/`, `data/SettingsStore.kt`)

A `ScreenFrame` titled Settings: one column, 640dp at most, of groups under quiet headings (titleSmall,
`onSurfaceVariant`). Rows are 56dp on the ground; the whole row toggles or picks, a Switch or a radio at its end or
start shows which, its label (bodyLarge) over a note (bodySmall). The memorable thing is the terminal itself: a live
sample at the group's top.

The plan, reviewed against the brief:

| First pass | Why it was generic | Revised to |
|---|---|---|
| A list of rows that open a dialog each ("Text size", "Theme"...) | the stock preference screen; the user can't see what a choice does until it is made | every choice in place, and the terminal sample over them redrawn as they change |
| Segmented buttons for the theme and the extra keys | a kit control, and three labels in a row truncate at font scale 2.0 | radio rows with notes, which wrap |
| Palette chips as colored circles | color alone, and a circle says nothing about a terminal | radio rows named for the palette, each with a strip of its ground and its eight normal colors |
| Copper section headings | a second loud element beside the bus bar | `onSurfaceVariant` headings; copper stays for the selected destination and the actions |
| A slider with a tick per sp in the theme's secondary | 23 dots of noise, and verdigris means healthy | a plain track (`surfaceContainerHighest`), the value in mono beside the label |
| Key rotation as one destructive button | it would cut every machine off until the new key is on them | Rotate key makes a new key and shows its public half; the machines switch to it only on "Use for every machine", under a note that the old key comes off the machines last |

```
+--------------------------------------+
| Settings                             |
| Terminal                             |
| +----------------------------------+ |
| | nick@build-01 ~/docs-site (main) | |
| | $ ls                             | |
| | site/  README.md  build.sh       | |
| |  M site/install.md               | |
| | $ #                              | |
| +----------------------------------+ |
| Text size                     14 sp  |
| ===========o-----------------------  |
| Font   (o) JetBrains Mono            |
| Colors (o) Match the theme  [######] |
| Keep the screen on              [on] |
| Appearance  Extra keys  Agents       |
| Notifications                        |
| Clipboard  Keys  About               |
+--------------------------------------+
```

- **Terminal:** the sample (`TerminalPreview`: a prompt, a listing, `git status` with a line selected, the cursor)
  in the chosen palette, font and size, cut at the plate's edge as a terminal's lines are; TalkBack reads "A sample of
  the terminal in Copper at 14 sp". Text size is a slider of whole sp (8-32) that sets every pane. Font: JetBrains
  Mono or the device's monospace. Colors: Match the theme (Copper when dark, Drafting when light), Copper, Drafting
  or Ghostty, each with its strip (`PaletteSwatch`). Keep the screen on, while an attached session is open
  (`KeepScreenOn`, off by default).
- **Appearance:** Match the system, Light or Dark; Wallpaper colors on Android 12 and later. The system bars' icons
  follow the side picked.
- **Extra keys:** Fit to the width (as Keyboard above), Always two lines, or Hidden (only the soft keyboard; a
  hardware keyboard hides the row whatever is picked).
- **Agents:** Agent chat view, a switch, on by default: a pane running Claude Code or Codex shows as a chat, with the
  terminal one tap away in the session's bar (The view switch, above). Off, a session's panes show their terminal and
  its bar has no switch. It is the default: a session's More menu sets its own either way (Chat view, above).
- **Notifications:** a switch per kind (Waiting for you, Finished, Agent notifications, Lost machines), and the line
  that questions always notify. While the system lets the app post none (the permission refused, Not now, or turned
  off), a `Banner` with the attention diamond says so, and Turn on opens the system's notification settings for the
  app; it is read again when the app resumes.
- **Clipboard:** the machines whose programs read the clipboard unasked (Always allow), each with its mono address
  and Ask again; or that every machine asks.
- **Keys:** a plate per key (`surfaceContainerLow`): Made on this device or Imported, the mono fingerprint, the
  machines that use it, Show public key and Copy public key. Under them the rotation note, Rotate key (outlined; a
  spinner and "Making a key" while it works, the core's words under it when it fails) and Import a key (the first
  run's import screen, secure window and all). A key made or brought opens its public half (the first run's screen
  with Back) and under it "Then switch your machines": what to do first, and **Use for every machine**, after which it
  says every machine signs in with this key. A connected machine keeps its connection and attached sessions, and
  signs in with the new key from its next connect or reconnect (`MobileCore.useKey`). A private key is never shown
  or logged.
- **About:** the app's name and the core's version in mono.
- **States:** a spinner while the stores are read; the reason and Retry when they can't be.

## Copy

Plain verbs, sentence case, no filler. Name things by what the user sees: "Machines", "Add machine", "End session",
"Take control", not transports or channels. Errors say what happened and the fix ("ngtermctl isn't on build-01:
install ngterm there and make sure ngtermctl is on the PATH of ssh logins"). Numbers carry units ("in 4 s").
