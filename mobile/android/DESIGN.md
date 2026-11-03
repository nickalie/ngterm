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
- **Dynamic color** is optional (`NgtermTheme(dynamicColor = true)`, Android 12+, a later Settings switch). It
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
- An agent's status comes from its origin as text, a hex color and one of upstream's six shapes (circle, square,
  triangle, diamond, capsule, star). `AgentStatusBadge` draws that shape, and lifts the color toward white or black
  (`readableOn`) until it reads on every surface up to `surfaceContainerHigh`, so it holds on a card or the sidebar
  as on the ground: the origin's color is the user's choice, the legibility is ours.

### Terminal palettes

`ui/theme/TerminalPalette.kt`: background, foreground, cursor, selection and the 16 ANSI colors.

- **Copper** (dark default): the Graphite ground, so the terminal and the chrome are one surface; a copper cursor,
  verdigris for cyan.
- **Drafting** (light default): the drafting ground with ink text, the ANSI colors darkened to read on it.
- **Ghostty**: desktop ngterm's starting point, Ghostty's `#282c34` ground with white text and libghostty-vt's
  built-in 16 colors.

The theme picks Copper or Drafting with the light or dark side; Settings may pin one later. Every palette's text is
4.5:1 on its ground and on its selection, and its cursor 3:1. `TerminalView` hands the palette to the pane
(`Pane.set_colors`), so the terminal resolves ANSI colors with it and every reattach starts from it; colors a program
sets (OSC 4/10/11/12) win, as on desktop.

## Type

`ui/theme/Type.kt`. One family in two widths, both variable fonts (wght 200-800) bundled from Google Fonts under the
OFL (`licenses/`):

- **Atkinson Hyperlegible Next** for the chrome. The Braille Institute drew it to tell similar letters apart, which
  is the job here: hostnames, session names and statuses read at a glance, at any font scale.
- **Atkinson Hyperlegible Mono** for what is compared character by character: `user@host:port`, fingerprints, key
  lines (`NgtermTheme.mono`). It is not the terminal's font: the terminal's is JetBrains Mono (The terminal, below).

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
(a struck signal), Close, Lock, Back (mirrored right to left) and More (an item's menu). The status glyphs are 16dp filled or stroked shapes (`ic_status_*`). The mark
(`ic_ngterm_mark`) is the launcher's `>_` cut out of a copper tile. No icon font or icon library.

## Components

`ui/components/`. Each takes its content as parameters; none reads app state.

| Component | What it is |
|---|---|
| `BusBar` | the copper bar marking where you are |
| `StatusGlyph`, `StatusBadge`, `AgentStatusBadge` | a status's glyph; glyph and word for an app tone; for an origin's status |
| `MachineCard` | a machine: name, mono address, connection badge, and a summary slot for its sessions' status |
| `SessionRow` | a session: name, context or companion line, status slot, Attached mark, the bus bar when on screen, and an optional menu at the end (Attach or Detach, End session) |
| `SectionHeader` | a group's heading with an optional count and one text action |
| `MessagePanel` with `EmptyState`, `ErrorState`, `OfflineState` | what a screen shows instead of content: icon, title, what to do, and the one action that does it; at most one quieter text-button action under it, for a retry after the fix the main one points to |
| `Banner` | a strip about the whole screen while it stays usable, with an optional action and dismiss; announced politely |
| `ConfirmDialog` | the confirmation before what can't be undone; the button repeats the action's words, in the error color |
| `KeyPlate`, `CopyButton` | text compared or pasted character by character (a key line, a fingerprint, a command): mono, selectable, wrapping anywhere, on a `surfaceContainerLow` plate with an optional label above and Copy or Share under it; Copy says "Copied" for 2 s |
| `ScreenFrame` | a screen of reading and filling in: an optional top row (Back or Close, the title as a heading, one action such as Save), then a scrolling column no wider than 640dp (520dp in the first run), centered, with the phone's or the tablet's gutter, padded for the bars, the cutout and the keyboard |
| `Cover` | a veil over a pane that can't be used as it is (another device drives it, it is reattaching), with its action; it takes every touch, and the pane under it is hidden from TalkBack (`Modifier.underCover`) |

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

Destinations today: Machines, Sessions (what is attached, across machines) and Settings. A machine's own session list
lives under Machines (tap a machine); Sessions is the cross-machine view of what is attached, and fills in with the
session view.

## Screens

### First run (`ui/onboarding/`)

Full screen, no navigation: there is nowhere to go without a key. A column of 520dp at most, left aligned (it is
reading, not a poster), the mark on top of the welcome.

- **Welcome:** "Set up a key", what the key is for, and a line with the lock glyph in verdigris that the private key
  is sealed by the device's keystore. Create a key is the filled button, Import a key the outlined one under it; while
  the key is made the button shows a spinner and its words say so.
- **Import:** the private key pasted into a mono field or read from a chosen file, and the passphrase (hidden, with a
  Show toggle). What went wrong shows under the field it is about: no passphrase or the wrong one under the
  passphrase, anything else under the key, and the text stays for another try. The key's text lives in memory only,
  never in saved state.
- **Public key:** the three steps, numbered because they are a sequence; the `authorized_keys` line on a `KeyPlate`
  with Copy and Share; its fingerprint; the one command that appends it, with Copy; Done. The machine form opens the
  same screen with Back.

### Machines (`ui/machines/`)

- **Home:** "Machines" with Add machine at the top, then a `MachineCard` per machine, 640dp at most and centered.
  The card's badge is the connection; under the address, what the state means and the one action it calls for at
  the end of the line: Not connected / Connect, The connection dropped / Retry now (the badge counts down "in 4 s"),
  the core's reason / Retry, Host key changed / Review. Connected shows the sessions' status instead: a badge per
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
  rather than losing its end), and New session at the end while connected.
- **List:** 640dp at most, centered. "Attached" (what this app has attached) first, then "On build-01" (the rest);
  each heading with its count, a group with nothing in it left out. A row is a `SessionRow`: the name, the context (or
  the working directory when it has none), the agent's status badge, and a More menu with Attach or Detach and End
  session (in the error color). Companions sit indented under their parent with "Companion of …" as their line.
  Tapping a row attaches it (the session view opens from there). A row being ended says "Ending" with a small
  spinner in place of its status. Pull to refresh lists again; on a machine not connected it connects.
- **New session:** a dialog over the list: one optional Name field ("Optional: build-01 names it when blank"),
  Create session and Cancel. While the machine creates and lists it (up to 20 s) the button says Creating with a
  spinner and the field holds still; Cancel closes the dialog and the create goes on. A failure shows under the
  field, in the core's words, and the name stays for another try.
- **End session** confirms ("End docs-site?", "It stops on build-01, with everything running in it. This can't be
  undone.", End session in the error color). A failed End or attach is a snackbar in the core's words.
- **States**, each a `MessagePanel` in place of the list: connecting and reading are a spinner with what is under
  way; not connected offers Connect; the device offline says so with Retry; a dropped connection counts down "Trying
  again in 4 s" with Retry now; unreachable gives the core's reason with Retry; a changed host key offers Review
  (the changed-key stop); no sessions invites New session; a listing that failed gives its reason with Retry. A
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
- **Size:** 13 sp by default (`TerminalTextSize`): about 50 columns on a phone held upright, 110 in landscape. It
  follows the system font scale like any text (at 2.0, about 25 columns); a pinch changes it in whole sp steps
  between 8 and 32 (`TerminalTextSizes`), so the machine's pty is resized once per step, not per frame.
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
- **TalkBack:** the view is announced as "Terminal". Reading its text out is Task 17's, with the input connection.

### Touch, mouse and selection (`ui/terminal/TerminalGestures.kt`, `TerminalMouse.kt`, `SelectionHandles.kt`)

The terminal keeps the gestures people already know from Android's text and from desktop terminals; nothing new to
learn, and nothing drawn until a gesture asks for it.

| Input | Does |
|---|---|
| a finger's drag | scrolls by whole rows, and a flick flings on and slows as a list does; the pane routes it as wheel turns: the scrollback, or the program when it asked for the mouse (or cursor keys under alternate scroll) |
| a long press | selects the word (a haptic tick), with handles and the toolbar; dragging on without lifting extends it |
| a handle's drag | moves that end of the selection |
| two fingers | pinch the text size |
| a tap | a click for a program tracking the mouse; otherwise it lets a touch selection go (and Task 17's keyboard) |
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

### A program reads the clipboard (`ui/terminal/TerminalPrompts.kt`, `data/ClipboardReads.kt`)

A program's OSC 52 read waits for the user, as Ghostty's `clipboard-read = ask`. It is a sheet over whatever is on
screen, the same shape as the first connect's: the attention diamond, "Let docs-site read your clipboard?", what the
program would see and from where ("A program in docs-site on build-01 asks for the text you copied. It sees it only
if you allow it."; a read that only lists what kinds the clipboard holds says so), then **Allow** (filled), **Always
allow on build-01** (outlined) and **Deny**. Swiping the sheet away denies. Reads queue and show one at a time; a read
the core gives up on (its pane hidden, detached, lost or exited) closes its sheet. "Always" is the app's to keep, per
machine (`ClipboardTrust`, which Settings lists), and answers that machine's later reads at once. In the background
Android lets no app read the clipboard, so an always-allowed read then gets an empty one.

## Copy

Plain verbs, sentence case, no filler. Name things by what the user sees: "Machines", "Add machine", "End session",
"Take control", not transports or channels. Errors say what happened and the fix ("ngtermctl isn't on build-01:
install ngterm there and make sure ngtermctl is on the PATH of ssh logins"). Numbers carry units ("in 4 s").
