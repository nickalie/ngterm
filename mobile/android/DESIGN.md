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
4.5:1 on its ground and on its selection, and its cursor 3:1.

## Type

`ui/theme/Type.kt`. One family in two widths, both variable fonts (wght 200-800) bundled from Google Fonts under the
OFL (`licenses/`):

- **Atkinson Hyperlegible Next** for the chrome. The Braille Institute drew it to tell similar letters apart, which
  is the job here: hostnames, session names and statuses read at a glance, at any font scale.
- **Atkinson Hyperlegible Mono** for what is compared character by character: `user@host:port`, fingerprints, key
  lines (`NgtermTheme.mono`). It is not the terminal's font; the terminal's is its own choice (TerminalView).

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
(a struck signal), Close, Lock. The status glyphs are 16dp filled or stroked shapes (`ic_status_*`). The mark
(`ic_ngterm_mark`) is the launcher's `>_` cut out of a copper tile. No icon font or icon library.

## Components

`ui/components/`. Each takes its content as parameters; none reads app state.

| Component | What it is |
|---|---|
| `BusBar` | the copper bar marking where you are |
| `StatusGlyph`, `StatusBadge`, `AgentStatusBadge` | a status's glyph; glyph and word for an app tone; for an origin's status |
| `MachineCard` | a machine: name, mono address, connection badge, and a summary slot for its sessions' status |
| `SessionRow` | a session: name, context or companion line, status slot, Attached mark, the bus bar when on screen |
| `SectionHeader` | a group's heading with an optional count and one text action |
| `MessagePanel` with `EmptyState`, `ErrorState`, `OfflineState` | what a screen shows instead of content: icon, title, what to do, and the one action that does it |
| `Banner` | a strip about the whole screen while it stays usable, with an optional action and dismiss; announced politely |
| `ConfirmDialog` | the confirmation before what can't be undone; the button repeats the action's words, in the error color |
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

Destinations today: Machines, Sessions (what is attached, across machines) and Settings.

## Copy

Plain verbs, sentence case, no filler. Name things by what the user sees: "Machines", "Add machine", "End session",
"Take control", not transports or channels. Errors say what happened and the fix ("ngtermctl isn't on build-01:
install ngterm there and make sure ngtermctl is on the PATH of ssh logins"). Numbers carry units ("in 4 s").
