# ngterm for iOS: design system

Every screen of the iOS app stays inside this system. A screen that needs something new adds it here on purpose, in
the same change. The code is `Packages/NgtermDesign/` (tokens and components) and `Packages/NgtermFeatures/Sources/
NgtermFeatures/Scaffold/` (the frame). Goldens in each package's `Tests/*/__Snapshots__/` show it light and dark, at
an accessibility text size, and on a phone and a tablet. Each simulator keeps its own goldens (`-on-phone`,
`-on-tablet`), since the two lay text out a little differently.

The Android app has its own Material system (`mobile/android/DESIGN.md`). The two share the brand (copper, verdigris),
the status system, the terminal palettes and the mono fonts. Everything else here is iOS's.

## Brief

- **Subject:** a remote control for terminals. ngterm runs on a server or a desktop, AI agents work in its sessions,
  and this app reaches them over ssh to watch, answer and type.
- **Audience:** developers who run agents on machines they own: checking in from a phone between other things, or
  working properly from an iPad with a keyboard.
- **Primary job:** see at a glance which machine and which session needs you, and get into its terminal fast. The
  terminal is the hero; everything else is the way to it.

## Direction: HIG-native, with ngterm's copper

The app is an iOS app first. Its navigation, lists, forms, sheets, toolbars, menus and confirmations are the
system's, so they get Liquid Glass, Dynamic Type, VoiceOver, pointer and keyboard behavior, and Stage Manager
resizing from the system, not imitated. The type is SF Pro on Dynamic Type text styles. Icons are SF Symbols.

ngterm adds three things on top, and only these:

- **Copper** is the tint (the mark's metal, `resources/icons/ngterm.svg`): the selection, buttons, links and every
  control's accent. **Verdigris**, copper's patina, means healthy: connected, working, attached.
- **The status system:** a status is always a glyph, a color and a word, never color alone.
- **The terminal:** the shared palettes and JetBrains Mono, and Atkinson Hyperlegible Mono for what is compared
  character by character.

### The plan, reviewed against the brief

| First pass | Why it was generic, or wrong for iOS | Revised to |
|---|---|---|
| Port Android's graphite and drafting grounds and tonal containers | a Material system on iOS: custom grounds fight Liquid Glass and the system's grouped lists | the system grounds (`systemBackground`, `systemGroupedBackground` and their levels); the app's color is the tint and the statuses |
| Android's bus bar marking the selected row | a second selection mark beside the system's, which already shows it in the tint | the system's list selection, tinted copper |
| Atkinson Hyperlegible Next for the chrome | a custom face in the chrome reads as a web view, and loses SF Pro's optical sizes | SF Pro; the mono family stays for addresses and fingerprints, where telling `0` from `O` matters |
| Colored dots for statuses | color alone carries meaning | an SF Symbol per status, always with its word |
| One contrast bar for every setting | Increase Contrast is a promise iOS users turn on | 4.5:1 normally, 7:1 with Increase Contrast, each token defined for both |
| A splash with the mark at launch | the HIG asks the launch screen to look like the first screen, not to brand it | the launch screen is the first screen's grouped ground, nothing on it |

## Color

`Palette.swift`. Each token (`ColorToken`) has four values: light, dark, and each with Increase Contrast. It draws as a
dynamic color, so it follows the system by itself. `ContrastTests` checks every promise below against the system
grounds as UIKit resolves them, in all four appearances.

| Token | Light | Dark | Light, increased | Dark, increased | Role |
|---|---|---|---|---|---|
| `tint` (copper) | `#984a19` | `#e3884f` | `#7c3c14` | `#efbb9a` | the app's tint (also the asset catalog's `AccentColor`) |
| `onTint` | `#ffffff` | `#2a1406` | `#ffffff` | `#2a1406` | a prominent button's label on the tint |
| `healthy` (verdigris) | `#1a6a5d` | `#5fbfaa` | `#15574d` | `#90d3c4` | connected, working, attached |
| `connecting` (brass) | `#7a5a0a` | `#d9b25a` | `#634908` | `#e1c27c` | on its way |
| `offline` (slate) | `#535f65` | `#9aa6ac` | `#454f54` | `#bfc7cb` | not connected, not trying |
| `attention` | `#984a19` | `#f0904f` | `#7c3c14` | `#f5b98f` | waiting on you |
| `failed` (oxide) | `#a83226` | `#f2867a` | `#8f2a20` | `#f7b6af` | failed |
| `idle` | `#47535a` | `#a9b4b9` | `#444f56` | `#bec7ca` | nothing happening |
| `secondaryText` | `#58585e` | `#a0a0a6` | `#4a4a50` | `#c8c8ce` | secondary text: an address, a row's line, a note, a plate's label, a form's hint |
| `brandCopper` | `#cf6e31` | | | | the icon's fill only, never text |

The light and dark values are Android's (`ui/theme/Color.kt`). The Increase Contrast values keep each hue and move
its lightness until it reaches 7:1.

Promises:

- Every text token (`tint`, `healthy`, `secondaryText` and the six status colors) reaches **4.5:1** on every system ground: the plain
  and grouped backgrounds at all three levels. With **Increase Contrast** it reaches **7:1**.
- `onTint` reaches the same on `tint`. The system's white label on the dark side's copper would not (2.6:1), so a
  prominent button is `prominentAction()`, not a bare `.borderedProminent`.
- An origin's status color is the user's choice. It is lifted toward white or black until it reads on every ground at
  once (`RGB.readable(on:minimum:)`); the word goes with it anyway.
- Text, separators and fills are the system's (`.primary`), which follow Increase Contrast themselves. Secondary
  text is `secondaryText`, not the system's `.secondary`: that grey stays under 4.5:1 on the grouped grounds (the
  accessibility audit says "nearly passed"), and so do the system's form headers and footers, which the app writes as
  its own (`FormHint`).

### Status

`StatusShape`, `StatusTone`, `AgentState`, `AgentStatusLook`. A status is a glyph, a color and a word, and the three
always go out together. The eight glyphs are Android's: upstream's six status shapes plus a ring and a slash.

| Tone | Glyph (SF Symbol) | Word | Color |
|---|---|---|---|
| online | `circle.fill` | Connected | `healthy` |
| connecting | `circle` (an open ring) | Connecting | `connecting` |
| offline | `circle.slash` | Offline | `offline` |
| attention | `diamond.fill` | Needs you | `attention` |
| failed | `triangle.fill` | Failed | `failed` |
| idle | `capsule.fill` | Idle | `idle` |

- No two tones share a glyph (`StatusTests`).
- A badge may say more than the word ("Retrying in 4 s"); the glyph and color stay the tone's.
- An agent's status comes from its origin as its wire name, with a hex color and one of upstream's six shapes (circle,
  square, triangle, diamond, capsule, star) only when the agent overrode them. What the origin leaves unset comes from
  the app's defaults, its own tones in desktop's roles:

  | Wire name | Word | Glyph | Color |
  |---|---|---|---|
  | `active` | Working | circle | `healthy` |
  | `blocked` | Waiting | diamond | `attention` |
  | `completed` | Done | star | `healthy` |
  | `idle` | Idle | capsule | `idle` |

  A status this build doesn't know shows the origin's own word.

### Terminal palettes

`TerminalPalette.swift`: ground, text, cursor, selection and the 16 ANSI colors. They are Android's value for value;
`TerminalPaletteTests` reads `mobile/android/.../ui/theme/TerminalPalette.kt` and fails when the two differ.

- **Copper** (the dark side's default): a graphite ground, a copper cursor, verdigris for cyan.
- **Drafting** (the light side's default): a drafting-paper ground with ink text, the ANSI colors darkened to read on
  it.
- **Ghostty**: desktop ngterm's starting point.

Every palette's text reaches 4.5:1 on its ground and on its selection, and its cursor 3:1 (`ContrastTests`). The
terminal keeps its palette's ground whatever the chrome's appearance: it is its own surface.

## Type

`Typography.swift`, `Fonts.swift`. The chrome is SF Pro on Dynamic Type text styles, so every role scales through the
accessibility sizes. Sentence case everywhere; no all-caps labels.

| Role | Style | Used for |
|---|---|---|
| `machineName` | headline | a machine's name in its row |
| `sessionName` | body, semibold | a session's name in its row |
| `detail` | subheadline | a row's second line, a row's action |
| `statusWord` | footnote, semibold | a status's word and glyph |
| `banner` | subheadline, semibold | a banner's message and action |
| `note` | footnote | a banner's or a cover's explanation |
| `coverTitle` | title 3, semibold | a cover's title |
| `address` | Atkinson Hyperlegible Mono at footnote, scaling with it | `user@host:port` |
| `plateLabel` | footnote, semibold | a plate's label ("Public key", "Fingerprint") |
| `plateValue` | Atkinson Hyperlegible Mono at callout, scaling with it | a plate's value, and the form's host, port and user |

Screen titles, list headers, buttons and empty states use the system's own styles.

**Mono fonts**, bundled from their one source (`mobile/android/app/src/main/res/font/`, copied by
`scripts/build-ios.sh` with their OFL texts):

- **Atkinson Hyperlegible Mono** (the regular instance of the variable font) for what is compared character by
  character: addresses, fingerprints, key lines, versions.
- **JetBrains Mono 2.304**, Regular, Bold, Italic and Bold Italic, the terminal's face, as Ghostty and desktop ngterm
  embed it.

## Spacing, size and shape

`Spacing.swift`, `Radius.swift`.

- A 4 pt grid: 2, 4, 8, 12, 16, 24, 32, 48. Gutters are the system's list and form margins.
- Anything tappable is at least **44 pt** on its short side (a row, a row's action, a banner's buttons).
- Lists and forms stop growing at 640 pt and center; a text block at about 420 pt.
- The system's containers keep their own shapes. The app's own surfaces: 6 pt for a badge, 12 pt (continuous) for a
  banner or a plate.
- Depth is the system's: grouped lists, sheets, materials. No custom shadows or gradients.

## Motion

`Motion.swift`. Motion answers an action and shows what changed; nothing moves on its own, and nothing loops. 120 ms
for a selection or a tint, 220 ms for a panel. With **Reduce Motion** on, every animation of the app's is a cut
(`Motion.animation(_:reduceMotion:)`); the system's own transitions follow the setting themselves. Nothing depends on
an animation finishing.

**Reduce Transparency:** the app's translucent surfaces are system materials (`.regularMaterial`), which turn opaque
by themselves.

## Components

`Sources/NgtermDesign/Components/`. Each takes its content as parameters; none reads app state.

| Component | What it is |
|---|---|
| `StatusGlyph` | a status's glyph alone, sized with its text; hidden from VoiceOver |
| `StatusBadge` | one of the app's tones: glyph and word (or a longer label) in its color, read as the words |
| `AgentStatusBadge` | an origin's agent status, its color lifted to read on every ground; a label may say more ("2 waiting") |
| `MachineRow` | a machine: name (a heading), connection badge, mono address, what the state means with its one action (`RowAction`) at the line's end, and a slot for its sessions' statuses. At accessibility sizes the badge goes under the name and the action under the line. VoiceOver reads it as one element, with the action as its custom action |
| `SessionRow` | a session: name, the Attached mark (link glyph and "Attached" in verdigris), its line (context, working directory, or "Companion of …"), its status slot, or "Ending" with a spinner; a companion is indented |
| `StatePanel` | what a screen shows instead of content, on `ContentUnavailableView`, scrolling when it doesn't fit (the largest text sizes): `.loading`, `.empty`, `.error`, `.offline`, each with the one action that moves on and at most one quieter one under it; the glyph (`idle` unless the state has a tone) is decorative, and the title is a heading of its own |
| `Banner` | a strip about the whole screen while it stays usable: glyph, message, note, optional action and dismiss; the text wraps and the actions go under it when they don't fit beside it; announced to VoiceOver when it appears |
| `Cover` | a veil (a system material) over a pane that can't be used as it is: glyph or spinner, title (a heading), what it means, the action that fixes it; it takes every touch, and `covered(by:)` hides the pane under it from VoiceOver |
| `confirmingDestructive` | the system's confirmation dialog before what can't be undone; its destructive button repeats the action's words |
| `prominentAction()` | the filled button: the tint, with `onTint` for its label |
| `ngtermTheme()` | the tint and the fonts on a view tree |
| `ValuePlate` | a value compared character by character (a key's line, a fingerprint, a command) under its label, mono and selectable, with its actions under it, on the grouped ground's raised surface |
| `CopyButton` | puts a value on the clipboard, then says Copied for 2 s (VoiceOver says it too) |
| `InlineProblem` | what is wrong, said where it happened (under a field, under a step): the failed glyph and the fix, in `failed` |

Rules the components hold to:

- Every screen has empty, loading, error and offline states, each with a next action. An empty screen invites the
  first step; an error says what happened and how to fix it, without apology.
- Destructive actions confirm (End session, Remove pinned key); safe ones don't (Detach).
- An action keeps its name through the flow: "End session" is confirmed by "End session".
- VoiceOver: decorative glyphs are hidden and the word next to them is read; a badge reads as its words; a row reads
  as one element; titles are headings.

## Frame: `AppScaffold`

A `NavigationSplitView` of three columns (machines, a machine's sessions, the session), by the horizontal size class:

```
compact (iPhone, Slide Over)     regular (iPad, Stage Manager large)
+--------------------+           +-----------+----------------+--------------------------+
| Machines        ⚙  |           | Machines ⚙| build-01       |                          |
| +----------------+ |           | build-01 ●| docs-site  🔗  |      the session         |
| | build-01   ●   | |           | nas      ⊘| api-refactor   |                          |
| | nas        ⊘   | |           |           |                |                          |
| +----------------+ |           |           |                |                          |
+--------------------+           +-----------+----------------+--------------------------+
 a stack: Machines → Sessions → Session        the columns side by side
```

- **Compact:** a stack, one column at a time, and the session view gets the whole screen.
- **Regular:** the columns side by side, all three where they fit. An iPad held upright has room for two, so it opens
  on the sessions column and the session, with the machines behind the sidebar button; the empty sessions column says
  "No machine selected" and offers **Show machines** (only while the machines are hidden: beside them it offers
  nothing).
- **State:** each window's navigation lives in its `ScaffoldModel` as long as the scene, so rotation, Split View and
  Stage Manager resizing change the layout and lose nothing.
- **Settings** is a sheet from the machines' toolbar (the gear).
- **Onboarding** is a full-screen cover on the first run: there is nowhere to go without a key.
- **One window per session (iPad):** a session can have a window of its own (`WindowGroup(for: SessionWindowID.self)`).
  SwiftUI keeps one window per session, so opening it again focuses that window. A session with its own window shows
  nowhere else: a window's detail column then says so and offers **Show window** (`SessionWindows`), since two views
  of one pane would fight over its size. The same holds between main windows (iPad allows several): a session shows
  in one main window's detail at a time. Opening it in another takes it over, and the first one's detail says so and
  offers **Show here**, which takes it back. When the window showing it lets it go (opens another session, shows
  another machine, closes), the window that had it before takes it back, and only that one.

## Screens

### First run (`Onboarding/`)

A full-screen cover while the app has no key, in a `NavigationStack`, its column at most 520 pt wide:

- **Welcome:** the key glyph in the tint, "Set up a key" (large title, a heading), what the key is for, and the
  Keychain note beside a verdigris lock. **Create a key** is the prominent action (a spinner and "Creating a key"
  while it runs, and no second tap); **Import a key** is the quiet one, the tint on the ground (a tinted fill under
  it stays under 4.5:1). A key that couldn't be made says why under the text
  (`InlineProblem`).
- **Import a key** (pushed): a scroll view of grouped fields, not a `Form` (a list lets go of the rows larger text
  pushes off the screen, which the accessibility audit reads as text that doesn't scale). The private key in a mono `TextEditor`, Choose file (the system's file importer,
  64 KB at most), the passphrase (`SecureField` with Show and Hide), and **Import key**. A problem shows under the field
  it is about: the key's under the key, the passphrase's under the passphrase. The key and the passphrase live in the
  screen's state only.
- **Add this key to your machines:** the three numbered steps, the public key's plate with Copy and Share
  (`ShareLink`), the fingerprint, the plate of the command that appends it, and **Done**, which ends the first run.
  The machine form pushes the same screen without Done.

### Machines (`Machines/`)

The first column: an inset grouped `List` of `MachineRow`s, its selection the machine whose sessions show.

- **Each row** reads its link: Connected (and its sessions' statuses: a badge per status with its count, then the
  idle count, or "No sessions"), Connecting, Offline with **Connect**, Reconnecting in *n* s (counting down each
  second) with **Retry now**, Unreachable with the core's reason and **Retry**, "Needs you" while a host key or a
  sign-in waits on its sheet, and "Host key changed" with **Review**.
- **Add machine** is the toolbar's `+`. A row's context menu has Connect or Disconnect, Edit, and Remove; swiping it
  removes it at once, and a banner at the list's end offers **Undo** for 10 s (the machine comes back where it was,
  with its pinned key). Removing doesn't confirm: Undo is the safety.
- **States:** empty ("No machines yet", with Add machine), offline (a banner over the list or the empty state; the
  list stays usable), and error (a change the store couldn't write: a failed banner at the list's end, dismissible).
  The stores read at launch, so the list has no loading state.

**Add machine / Edit machine** is a sheet with a `Form`: name, host, port and user (the last three mono), the key (a
choice only when the app has more than one, each with where it came from and its fingerprint) with **Show public
key**, and, editing, the pinned host key. Cancel and Save are the toolbar's. A field says what is wrong with it once
it has been left or Save was tried, under it, in the words of the fix ("Put the port in Port, not in the host.").

### Connect prompts

Sheets over the app, one at a time (a host key first, then a sign-in), each a page of the same shape (`PromptPage`):
the tone's glyph, the question as a heading, the machine's mono address (read by VoiceOver in words: "nick at
build-01.lan, port 22"), what it means, then its plates and actions.

- **Trust build-01?** The fingerprint, the command that prints the machine's own (with Copy), **Trust and connect**
  and **Don't connect**. Swiping the sheet away is Don't connect.
- **build-01 asks you to sign in:** the machine's message with its link (tappable, and Copy link), "Waiting for you to
  sign in" with a spinner, **Open link** and **Don't connect**. The link opens in Safari's view inside the app
  (`SFSafariViewController`), so the app stays in the foreground while the user signs in; the sheet goes by itself
  once the connect ends either way.
- **build-01's host key changed** (from the row's Review): what that can mean, both fingerprints, the advice, **Back to
  machines** as the prominent action and **Remove pinned key** as the destructive one, behind
  `confirmingDestructive`.

## App icon and launch screen

- **Icon** (`NgTerm/AppIcon.icon`, an Icon Composer document): the mark's three layers from `resources/icons/ngterm.svg`
  (the copper gear, as glass; the dark disc; the cream prompt), at 86% in the safe area, over a graphite gradient
  (`#1b2226`), deeper graphite on the dark appearance. The system derives the tinted and clear appearances from the
  layers. Render it with Icon Composer's `ictool` to check it.
- **Launch screen:** the first screen's grouped ground (`LaunchBackground`, the system's `systemGroupedBackground`),
  with nothing on it, as the HIG asks.

## Copy

Plain verbs, sentence case, no filler. Name things by what the user sees: "Machines", "Add machine", "End session",
"Take control". Errors say what happened and the fix ("ngtermctl isn't on build-01: install ngterm there and make
sure ngtermctl is on the PATH of ssh logins"). Numbers carry units ("in 4 s").
