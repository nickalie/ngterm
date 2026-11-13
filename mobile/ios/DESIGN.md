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
- `onTint` reaches the same on `failed`, the fill of a prominent button that is also destructive (an ask's default
  that can't be undone).
- An origin's status color is the user's choice. It is lifted toward white or black until it reads on every ground at
  once (`RGB.readable(on:minimum:)`); the word goes with it anyway.
- Text, separators and fills are the system's (`.primary`), which follow Increase Contrast themselves. Secondary
  text is `secondaryText`, not the system's `.secondary`: that grey stays under 4.5:1 on the grouped grounds (the
  accessibility audit says "nearly passed"), and so do the system's form headers and footers, which the app writes as
  its own (`FormHint`).

### Status

`StatusShape`, `StatusTone`, `AgentState`, `AgentStatusLook`. A status is a glyph, a color and a word, and the three
always go out together. The thirteen glyphs are Android's: upstream's eleven status shapes plus a ring and a slash.

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
- An agent's status comes from its origin as its wire name, with a hex color and one of upstream's eleven shapes
  (circle, square, triangle, triangle-down, play, diamond, capsule, seal, star, bolt, bell) only when the agent
  overrode them, each drawn by its `.fill` SF Symbol (`arrowtriangle.down.fill` for triangle-down). The app's hello
  lists `status.shapes`, so an origin sends it all eleven; a name this build doesn't know draws the circle. What the
  origin leaves unset comes from the app's defaults, its own tones in desktop's roles:

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
| `peerWord` | caption, medium | the app a machine runs, in its `PeerBadge` (small enough for a two-line title) |
| `banner` | subheadline, semibold | a banner's message and action |
| `note` | footnote | a banner's or a cover's explanation |
| `coverTitle` | title 3, semibold | a cover's title |
| `address` | Atkinson Hyperlegible Mono at footnote, scaling with it | `user@host:port` |
| `plateLabel` | footnote, semibold | a plate's label ("Public key", "Fingerprint") |
| `plateValue` | Atkinson Hyperlegible Mono at callout, scaling with it | a plate's value, and the form's host, port and user |
| `chatCode` | JetBrains Mono at 13 pt, scaling as the footnote does | what the terminal holds, in a chat: a tool's line, a command, a reply's code |
| `chatOutput` | JetBrains Mono at 12 pt, scaling as the caption does | a tool's output and a diff, in a chat |

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
| `PeerBadge` | the app a machine runs when it isn't ngterm ("agterm"): the word in secondary ink inside a hairline 6 pt outline, so it reads as a fact about the machine, not a status or an action. It takes no tap; what that app lacks is its VoiceOver hint and, with a pointer, its tooltip, and the screen with room says it in full |
| `SessionRow` | a session: name, the Attached mark (link glyph and "Attached" in verdigris), its line (context, working directory, or "Companion of …"), its status slot, or "Ending" with a spinner; a companion is indented (a machine's list indents every companion; the machines' column, one more step, only one under its attached parent) |
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
- A selected row (an iPad's sidebar draws the selection in the tint) draws its colored text, glyphs and actions in
  the row's own foreground (`rowForeground`), since the app's colors don't read on the tint; a status keeps its glyph
  and word there.

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

## The terminal (`NgtermTerminal`)

`TerminalView` draws a pane as Ghostty draws a frame, and as Android's terminal does (`TerminalPainter`,
`TerminalDecorations`). It is its own surface, in its palette whatever the chrome's appearance.

- **Grid:** whole cells from the top left. A cell is the face's advance wide and its ascent and descent tall, rounded up
  to whole points; what is left at the right and the bottom shows the terminal's ground. The view gives the pane the
  grid that fits, again as rotation, Stage Manager or the split resize it.
- **Text:** JetBrains Mono at the terminal's text size (13 pt until Settings picks another, 8 to 32), or the system's
  monospace. Bold and italic pick the face, faint text keeps half its alpha (Ghostty's `faint-opacity`), inverse swaps
  the text and the ground, invisible text leaves the ground alone. The font's ligatures stay, as Ghostty keeps them. A
  character the face lacks (CJK, emoji, symbols) comes from the system's fonts, centered in its cells or squeezed into
  them, so nothing leaves the grid.
- **Lines:** the five underlines (single, double, curly, dotted, dashed) in the underline color the program set, else
  the text's; strikethrough through the middle of the lowercase; overline along the cell's top. A line is the text
  size over 14 thick.
- **Cursor:** block (the character under it in the cell's ground color), bar (two lines wide at the cell's left),
  underline (two lines along its bottom) or hollow block, as the program asks; in the palette's cursor color, which
  the pane takes with the palette, else the cell's text color. On a wide character it covers both columns. An
  unfocused terminal shows a hollow block. It doesn't blink.
- **Selection:** the palette's selection color behind the text, which keeps its own colors (the palettes promise their
  text 4.5:1 on it, `ContrastTests`).
- **Motion:** none of its own. New rows show at once (no implicit animations), and the terminal draws only while output
  comes: its display link pauses when no frame waits.

### Touch, pointer and the clipboard

`Input/`, `Selection/`, `Prompts/`. The gestures are Android's (`TerminalGestures`, `TerminalMouse`), in UIKit's
recognizers; what is new to iOS is the system's: the edit menu, the text beam, the paste rule.

- **A finger:** a drag scrolls the screen with the finger, in whole rows, and flings on as a list does (a program that
  asked for the mouse gets wheel turns); a tap gives the terminal the focus, or lets a touch selection go, and clicks
  where it landed while a program tracks the mouse; a double tap selects the word, a triple tap the line; a long press
  selects the word with a selection tick (haptic) and a drag from it extends the selection as text selection does; a
  pinch changes the text size in whole points within Settings' 8 to 32, for this view only (Settings keeps its own).
- **Selection handles** (`SelectionHandles`), iOS's own shape: a stem two points wide on the selection's edge, a row
  tall, and a round knob 10 pt across, the start's above its stem and the end's below (on the other side where the view
  has no room), in the palette's cursor color, the terminal's accent on its own ground. A finger within 22 pt of a knob
  or a stem takes it (a 44 pt target); a long press there doesn't start a new word.
- **The edit menu** (`UIEditMenuInteraction`): Copy (with a selection), Paste (with text on the clipboard) and Select
  All, the system's menu with the system's words, over a touch selection once the finger lifts (under it when there is
  no room above), and at the pointer for a secondary click. It keeps out of the way while a finger moves the selection.
  Its items are the standard edit actions, so the menu's Paste and ⌘V are the user's own paste, which iOS doesn't
  question; Copy lets the selection go.
- **A pointer (iPad):** a text beam a row tall over the terminal, the system's pointer while a program takes the mouse;
  under mouse tracking (shift held overrides it) the buttons, their drags and the motion go to the program; otherwise a
  click lets the selection go, a drag selects, a double click the word, a triple click the line; the wheel and a
  trackpad's two-finger scroll scroll it; a trackpad pinch zooms as a finger's does.
- **Paste confirmation:** a paste the pane refuses as able to run a command (a line break or a control character) asks
  first, in the system's alert: "Paste text that can run a command?", what it can do, and the text's first six lines
  as they are (cut at 60 characters, control characters as their symbols, ␛ for escape). **Cancel** is the safe
  answer and what dismissing does; **Paste** repeats the menu's word.
- **A program reading the clipboard** (OSC 52, or the Kitty clipboard protocol's OSC 5522) asks in a sheet over
  whatever shows (`ClipboardReadQuestion`), a page of the connect prompts' shape: the attention diamond, "Let
  docs-site read your clipboard?" as a heading, from which session on which machine, then what the program would see
  on a plate (the grouped ground's raised surface), in Ghostty's macOS form: the copied text in the plate's mono
  (scrolling past 160 pt, the first 64 KiB, the rest counted), or, with no UTF-8 text, a line per type with its size,
  and under it the first image, fitted, at most 200 pt tall. Then the toggle **Don't ask again for this program in this
  terminal**, only for a read that can keep a grant (the Kitty protocol's password), **Allow** (prominent), and
  **Always allow on build-01** and **Deny**, quieter (the tint on the ground). Swiping it away denies. The pasteboard
  is read when the sheet comes up, which may bring up iOS's own question about pasting from another app (the app can't
  skip it outside the user's own paste); Allow hands over exactly what the sheet showed. A listing of the types, or a
  read asking for no type, never asks; neither does a read whose program holds a grant. Every read gets an answer: a
  read whose session is detached, whose pane's program ends, whose machine drops, or that is waiting when the app
  suspends is denied. A machine allowed always is answered unasked until Settings asks again.
- **A program writing the clipboard** isn't asked about (Ghostty's `clipboard-write = allow`): its text, HTML and
  images land on the general pasteboard as one item, and the program learns the outcome once the pasteboard took it. A
  write of none of those types is refused as unsupported, only a write with nothing in it clears, and a write to the
  selection or primary clipboard, which iOS doesn't have, is refused by the core.

### The keyboard

`Keyboard/`, `TextInput/`, `ExtraKeys/`. Typing is the terminal's other half, and on a phone the soft keyboard is in
charge of the screen. The behavior is Android's (`KeyMapping`, `TerminalKeyboard`, `ExtraKeysRow`); the extra keys are
iOS's own: the keyboard's accessory, on the keyboard's own ground, not a Material row.

- **Focus and the soft keyboard are apart.** A tap gives the terminal the focus and opens the soft keyboard; a pointer's
  click, a long press and the edit menu take the focus without it. The terminal asks for plain text: no autocorrect,
  spell check, smart quotes, dashes or spaces, automatic capitals, inline predictions or Writing Tools (it types
  commands and passwords). What the keyboard commits goes to the program at once, Return as Enter, delete with
  nothing composed as the program's backspace.
- **A composition** (a CJK input method's, a hardware dead key's, dictation's) waits at the cursor in the terminal's
  face on its ground, underlined 2 pt in the cursor's color, moved left as far as it needs to fit, and goes when it
  commits. A key from elsewhere (an extra key, a sticky tap) sends it first, and the terminal letting the keyboard go
  finishes it there.
- **The extra keys** (`ExtraKeysBar`, the terminal's `inputAccessoryView` on the keyboard's own ground) sit over the
  soft keyboard, and at the screen's foot while the terminal has the focus with the keyboard closed (the arrows and Esc
  work a pager or a menu without it). They stand aside while a hardware keyboard is attached (`GCKeyboard`), and when
  Settings hides them; Settings may also keep two lines at every width.

  ```
  phone, upright: two lines of eight
  +-----+------+-----+-----+------+-----+-----+------+
  | Esc |  /   |  |  |  ~  | Home |  ↑  | End | PgUp |
  | Tab | Ctrl | Alt |  ⌨  |  ←   |  ↓  |  →  | PgDn |
  +-----+------+-----+-----+------+-----+-----+------+

  sideways or wider (44 pt a key or more): one line of sixteen
  | Esc | Tab | Ctrl | Alt | / | | | ~ | Home | End | PgUp | PgDn | ← | ↑ | ↓ | → | ⌨ |
  ```

  - Ctrl and Alt by Tab under the thumb, the arrows an inverted T with Home and End over Left and Right, the page keys
    at the end. A key is its whole cell (no gaps), 44 pt tall. Android's `-` gives its place to the keyboard's key
    (`⌨`): an iPhone's keyboard has no key of its own that closes it. The key closes the soft keyboard and opens it
    again (`keyboard.chevron.compact.down`, then `keyboard`), the focus and the extra keys staying.
  - **Legends:** nothing is drawn around a key at rest. Words in SF Pro semibold at the subheadline size, all at one
    size, the widest's fitted to its key; `/`, `|` and `~` in JetBrains Mono at 18 pt (scaling to 26), the character as
    it will land; the arrows and the keyboard in SF Symbols. The label color on the keyboard's ground; the system's
    fill under a pressed key.
  - **A key acts as it goes down**, with the keyboard's click (as the user set it) and a light tap; the arrows and the
    page keys repeat while held (after 400 ms, every 50 ms), Esc, Tab and the symbols don't.
  - **Sticky Ctrl and Alt:** a tap holds the modifier for the next key, a second locks it, a third lets it go, each with
    a selection tick. Shape carries the state, never color alone: an outline 1.5 pt in the tint holds for the next key,
    a tint fill (`onTint` legend) locks. VoiceOver reads "Off", "On for the next key" or "Locked on"; each key is a
    keyboard key with its name ("Escape", "Vertical bar", "Hide keyboard").
- **A hardware keyboard:** what types plainly goes through the system's text input, so dead keys and input methods
  work as in any text field; special keys and chords go to the program as keys (Ctrl, Command as Super, the arrows,
  Esc, the function keys), repeating while held. Option types the layout's own ASCII character (`@`, `|`, `~` on
  European layouts) and is Alt otherwise, as Ghostty's `macos-option-as-alt` is on a US layout.
- **The app's chords** are upstream's Mac ones, listed while ⌘ is held: ⌘+ and ⌘- step the text a point (within 8 to
  32), ⌘0 goes back to Settings' size, ⌘⌥→ and ⌘⌥← move between panes, ⌘⌥↓ and ⌘⌥↑ between sessions (offered once the
  session view gives them somewhere to go). Copy and paste are the system's ⌘C and ⌘V, in its Edit menu.
- **VoiceOver** reads a terminal ("Terminal", a text input) line by line, as a page: each row is a line framed on its
  row, the page down to its last row with text (`UIAccessibilityReadingContent`). The selected pane says its cursor's
  line as it changes, at most once a second (a faster change is said once the second is up, as it stands then).

## Screens

### First run (`Onboarding/`)

A full-screen cover while the app has no key (or can't read its keys), in a `NavigationStack`, its column at most
520 pt wide:

- **Welcome:** the key glyph in the tint, "Set up a key" (large title, a heading), what the key is for, and the
  Keychain note beside a verdigris lock. **Create a key** is the prominent action (a spinner and "Creating a key"
  while it runs, and no second tap); **Import a key** is the quiet one, the tint on the ground (a tinted fill under
  it stays under 4.5:1). A key that couldn't be made says why under the text
  (`InlineProblem`). An import goes on through Back: the welcome shows a spinner with "Importing a key" over the
  actions, **Create a key** is disabled, and **Import a key** reopens the import busy. Its key shows its public half
  even from the welcome; what was wrong with it shows only on the import, so a failure leaves the welcome ready.
- **Import a key** (pushed): a scroll view of grouped fields, not a `Form` (a list lets go of the rows larger text
  pushes off the screen, which the accessibility audit reads as text that doesn't scale). The private key in a mono `TextEditor`, Choose file (the system's file importer,
  64 KB at most), the passphrase (`SecureField` with Show and Hide), and **Import key**. A problem shows under the field
  it is about: the key's under the key, the passphrase's under the passphrase. The key and the passphrase live in the
  screen's state only.
- **Add this key to your machines:** the three numbered steps, the public key's plate with Copy and Share
  (`ShareLink`), the fingerprint, the plate of the command that appends it, and **Done**, which ends the first run.
  The machine form pushes the same screen without Done.
- **States:** the keys are read before the cover goes up, so there is no loading state. Keys that can't be read (the
  file is there, but not readable yet) are no first run: the cover says "Your keys couldn't be read" with the system's
  reason and **Retry** (`StatePanel.error`), never the welcome, so no second key is made over them. A retry that reads
  them ends the first run when there is a key, and shows the welcome when there is none.

### Machines (`Machines/`)

The first column: an inset grouped `List` of `MachineRow`s, its selection the machine whose sessions show.

- **Each row** reads its link: Connected (and its sessions' statuses: a badge per status with its count, then the
  idle count, or "No sessions"), Connecting, Offline with **Connect**, Reconnecting in *n* s (counting down each
  second) with **Retry now**, Unreachable with the core's reason and **Retry**, "Needs you" while a host key waits on
  its sheet, "Waiting for sign-in" while a sign-in does, and "Host key changed" with **Review**.
- **A Mac running agterm** (its listing says so; one with no sessions listed yet says nothing) has a `PeerBadge`
  after its address; at accessibility sizes it goes under the address.
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

### A machine's sessions (`Sessions/`)

The second column: an inset grouped `List` of `SessionRow`s, its selection the session this window shows. The title is
inline: the machine's name over its mono address, read by VoiceOver as one heading with the address in words. **New
session** is the toolbar's `+` while the machine is connected.

- **A Mac running agterm:** its `PeerBadge` follows the address in the title (VoiceOver reads it last), and the core's
  note on what agterm lacks ("Attached from agterm: the agent chat view, … are not available") is the list's footer,
  in the app's note style. Its sessions list, attach, open and end as an ngterm machine's do.

- **Groups:** the families with a session this app has attached under "Attached", then the others under "On
  build-01", each a parent with its companions right under it (indented, "Companion of …"). The headers are the app's
  `FormHint` headings, not the system's grey. A row's line is the context, else the working directory; its status is
  the `AgentStatusBadge`; while its End runs it says "Ending" with a spinner. The Attached mark stays off the rows: the
  group's heading says it.
- **Actions:** a tap attaches the session and opens it. The context menu has Attach or Detach, Chat view (The
  session, below) and **End session**; the
  swipe actions (VoiceOver's row actions) End session and, when attached, Detach. End session confirms
  (`confirmingDestructive`: "End api?", "It stops on build-01, with everything running in it. This can't be undone.");
  Detach doesn't. The swipe's End has no destructive role, so the row stays until the machine's listing lets it go.
- **Pull to refresh** lists the machine again; on a machine that isn't connected it connects.
- **States,** each with the one action that moves on: connecting and reading (`StatePanel.loading`), empty ("No
  sessions on build-01" with **New session**), not connected (**Connect**), offline (**Retry**), dropped ("Trying
  again in 4 s", counting down, **Retry now**), unreachable (the core's reason, **Retry**), host key changed
  (**Review**, the changed key's sheet), `ngtermctl` missing (the shell's words read as such: **Open the README** in
  Safari's view, **Retry** under it), and a first listing that failed (**Retry**). Over the list or the empty state, a
  banner says the device is offline, and another that the list may be out of date (with Retry). An End or an attach
  that failed is a dismissible failed banner at the list's end.
- **New session** lands in the session in one tap: the `+` (and the empty state's New session) creates one the
  machine names, attaches it and opens it in this device's control. The origin's own window leads a session it
  started, so the pane's first report as a follower takes the lead at once, with no Take control cover to tap; a pane
  that leads already is left as it is. While the create runs, a spinner ("Creating") holds the `+`'s place, so a
  second tap starts nothing; an empty list says "Creating a session on build-01" (`StatePanel.loading`) instead. A create or
  an attach that fails is the failed banner at the list's end, and nothing opens. A create whose screen was left, or
  whose window opened another session meanwhile, stays attached and opens nothing: it never takes the lead unseen. Until
  the takeover's answer, the pane shows attaching, never a Take control cover that would only flash.
- **New named session…** is the `+`'s long-press menu (`Menu` with a primary action): a sheet with a `Form`: the name
  (mono, optional: "build-01 names it when blank"), Cancel and Create in the toolbar. Create lands in the session as
  the `+` does, the sheet closing first. While the create runs, Create gives way to a spinner and the field holds
  still; Cancel closes the sheet and the create goes on, its session only listed (unless a window opens it meanwhile),
  nothing opened (its failure then comes as the banner). What went wrong shows under the field, which keeps its text.
  The field isn't focused on its own: a field being edited reports only its text's height to the accessibility audit,
  under 44 pt.
- **A machine removed** while its sessions show takes the window back to the machines.

### The session (`Session/`)

The detail column, and an iPad's session window: the terminal is the hero, so the chrome is the navigation bar and
nothing else, and the panes run to the screen's edges on the terminal's own ground.

```
compact: the selected pane alone                regular: the origin's split
+-----------------------------+                  +-----------+------------------------------------------------+
| <    api              1 2 … |                  | Machines  |              api                     1 2 …     |
|  ● working  Fix the parser  |                  | build-01 ●|        ● working  Fix the parser               |
| [ Deploying api  step 2/3 ] |                  |   api   ● | [ Deploying api   step 2 of 3 ]               |
| $ make test                 |                  |   docs  ◐ | $ make test          ┃ $ cargo watch            |
| ok parser 0.42s             |                  | nas      ⊘| ok parser 0.42s      ┃                          |
+-----------------------------+                  +-----------+------------------------------------------------+
```

- **Toolbar:** the session's name over its state (the agent's status while the stream is live; "Connecting",
  "Reconnecting" or "Can't stream" while it isn't), the `PeerBadge` of a session on a Mac running agterm, and its
  context, read as one heading. Trailing: the pane switcher,
  the view switch while the selected pane runs an agent, and More.
- **Pane switcher** (two panes or more): the panes' numbers in the origin's order, 44 pt each; the selected one bold,
  underlined and in the tint (weight and the line carry it, not color alone). VoiceOver reads "Pane 1 of 2" and the
  title its program set, and which is selected.
- **The view switch** (`ViewSwitch`): while the selected pane runs Claude Code or Codex (its origin announces the
  agent) and its session's chat view is on, the pane shows as a chat (The chat, below) and the toolbar has one more
  button, named for what a tap does: "Show terminal" (`terminal`) over a chat, "Show chat" (`text.bubble`) over the
  terminal. Picking the terminal holds for that pane until its agent starts its next conversation; picking the chat
  lifts it. When the agent ends, or its chat ends or can't open (an origin without the chat view), the pane shows its
  terminal and the switch offers the chat back while the agent is still announced. A pane without an agent, or every
  pane of a session with its chat view off, has no switch. Switching never resizes the pane or moves its lead.
- **More** (`ellipsis`): Detach at once, Open in New Window (where the device has windows), Chat view, and End
  session, which confirms ("End api?", as the sessions list does).
- **Chat view** (`ChatViewToggle`, in every session's menu: More, and a machine's list's context menu): the session's
  own Agent chat view, a toggle the system's menu draws with a checkmark while the session's panes running an agent
  show as a chat. It starts at Settings' Agent chat view, and a tap sets the other for that session alone, kept on
  this device (`session-chat-views.json`, the newest 256 sessions); one set back to Settings' value follows Settings
  again. A choice the store can't write holds until the app ends. On a Mac running agterm, which has no chat view, it
  is off and takes no tap, its subtitle saying "agterm has no chat view"; its panes always show their terminal, with
  no view switch.
- **HUD:** the origin's HUD as a strip under the toolbar on the secondary ground (a spinner when the origin runs one,
  its message and its detail), in the app's colors, not the desktop's; announced as it changes, and left out below
  320 pt of height (a phone on its side with the keyboard up), where it would take the terminal's last rows.
- **Compact:** the selected pane alone; a sideways swipe on it (twice as far across as down, 64 pt or flung) moves to
  the next pane leftward and the previous rightward, sliding in from that side (a cut with Reduce Motion).
- **Regular:** the origin's split, side by side (`vertical`) or stacked, split by a hairline; a split it hides, or one
  pane, shows the selected pane alone. A pane tab toggles the split on this device alone, the origin's untouched: a tab
  in the split shows its pane alone and selected, the shown pane's own tab brings the split back, and the other tab
  shows that pane alone instead; VoiceOver's hint names the tap ("Shows this pane alone", "Shows both panes"). The
  choice is kept per session while the app runs. Each pane of a split has a 4 pt gutter at its leading edge, where the selected
  one shows a 3 × 32 pt bar in the tint; its cursor draws as it is, the other's as a hollow block. A tap or a click on
  a pane selects it. Beside it, the machines' column lists each machine's attached sessions under it (name and
  status; the session view says the context) as the tree a machine's sessions shows, kept to what is attached: in the
  machine's order by family, a companion indented once more under its attached parent with "Companion of …" as its
  line. A companion whose parent isn't attached keeps that line but stands at the sessions' own indent, so it never
  reads as another session's; there is nothing to expand or collapse. The one shown is marked with a checkmark in the
  tint (the list selects nothing meanwhile: a selection it draws grey once the session has the focus would leave the
  machine's colored text under 4.5:1). A compact width's machines list shows the same rows under each machine, so a phone sees every
  attached session in one place, as Android's Sessions destination shows them.
- **Selection and the keyboard:** one keyboard serves every pane; the selected pane is the only one anything typed
  reaches. A tab, a swipe, ⌘⌥←/→ and a touch select a pane, and the focus follows: the pane selected next takes it,
  with the soft keyboard if it was open. The selection is kept per session, so another size class, a turn or a trip
  elsewhere finds the same pane.
- **Covers** (`Cover`, over the pane, hiding it from VoiceOver; over a chat only a pane that couldn't be shown, since
  reading never takes control): another device drives it ("Another device is driving
  this pane", **Take control**); attaching (a spinner, "Reading this pane from build-01."); the program exited ("The
  program exited", the exit code or signal, **Detach**: the session ended there); a pane that couldn't be shown (the
  core's words, **Retry**). A dropped link is no veil: the last screen stays readable under a light scrim that takes
  the touches, with a strip at the top ("Reconnecting to build-01", "The last screen stays until it is back").
- **States:** "Opening api" (a spinner) before the layout; "build-01's ngterm can't share live sessions" with
  **Detach**; "api is gone" ("It ended on build-01, so there is nothing left to show.", **Close**) once it ended there.
  Detached some other way, the view closes (an iPad's session window too).
- **Screen awake:** while a session shows and Settings' Keep screen on is set, the idle timer is off (across windows).

### The chat (`Chat/`)

A pane running Claude Code or Codex, read as a conversation (The view switch, above). Underneath it is still the
agent's TUI in the pane, so the chat is a second reading of the terminal, not a messenger: everything the agent typed
or ran is in the terminal's face (`chatCode`, `chatOutput`), and only what people and the agent wrote as prose is in
SF Pro. The behavior is Android's (`ui/chat/`); the look is the system's grounds, SF Symbols and the app's tokens.

| First pass | Why it was generic, or wrong for iOS | Revised to |
|---|---|---|
| Android's Material chat: tonal containers, a filled composer bar | a Material system on iOS | the system grounds: the chat on `systemBackground`, prompts, plates, cards and the pill one level up (`secondarySystemBackground`), the composer on the system's bar material |
| Bubbles on both sides, avatars, a time under each | the messenger kit; a reply is a document | only the prompt is a bubble (at the end of the line); a reply is full-width text on the ground |
| A sheet or alert for a permission prompt | hides the conversation the question is about | a Needs-you card pinned at the chat's foot, in the ask sheet's shape and order |
| A spinner on every running tool | motion nobody asked for | the connecting ring and "Running", standing still |
| A typing indicator at the end of the list | loops while nobody asked | the activity line over the composer, Stop at its end |
| A rich Markdown kit as a dependency | another dependency, and its own look | Foundation's Markdown parser, drawn as the app's text (`MarkdownText`) |

- **Where:** in the pane's slot, in its terminal's place. The terminal stays laid out under it, unfocused and out of
  sight, at the grid the slot gives it, so switching never resizes the pane, and a chat action that takes control lays
  the program out for this device. The extra keys are the terminal's keyboard accessory, so they don't come up over a
  chat. A hardware keyboard's ⌘⌥ arrows still move between panes and sessions from the chat's field.
- **The list** (`ChatList`): oldest at the top, one column 640 pt at most, centered, with the phone's 16 pt gutter. It
  opens at the newest row and follows new ones while it is at the end; a finger on it lets it be at once, and a scroll
  that stops at the end follows again. Which rows are open survives scrolling and the size class.
- **Copying:** a prompt, a reply, an output, a diff and a question's words take the system's text selection
  (`textSelection(.enabled)`: a long press offers Copy).
- **Prompts** (`PromptBubble`): at the end of the line on the raised ground, 12 pt corners, the body size, no wider
  than 480 pt, with "With 2 images" under them when they carried any. VoiceOver reads "You: …", then the images.
- **Replies** (`MarkdownText`): full width; paragraphs and lists in the body size, headings at title 2, title 3 and
  headline (never the large titles), links in the tint and underlined, quotes in `secondaryText` behind a 2 pt rule,
  tables in a grid that scrolls sideways. Code blocks are `chatCode` on a plate with 6 pt corners, scrolling sideways,
  never highlighted; inline code the terminal's face. Images aren't fetched.
- **Tool lines** (`ToolRow`, on `ChatLine`): the kind's glyph (an SF Symbol: `apple.terminal`, `pencil`, `doc.text`,
  `magnifyingglass`, `globe`, `arrow.triangle.branch`, `wrench.adjustable`, read as its word: Command, Edit, Read,
  Search, Web, Subagent, Tool), the title in `chatCode` on one line, and the status at the end: Running on the
  connecting ring, Done on the healthy circle, Failed on the failed triangle. A line that holds something is a 44 pt
  target: a tap opens its diff, its output (`chatOutput` on a plate, scrolling sideways) and "Cut short here: the
  terminal has the rest." when the origin cut them, indented to the title.
- **Diffs** (`DiffView`): each file's header lines bold and quiet, hunks quiet, added lines `healthy`, removed lines
  `failed`, kept lines the primary text, each keeping its `+` or `-`; at most 400 lines, then how many more.
- **Thinking, subagents, questions, notices** as Android's: a quiet line ("Thinking", or Codex's summary title); a
  task's "2 steps", its steps behind a 2 pt rule; a question with Needs you, then Answered; an interrupt, a compaction
  (a tap shows its summary), an API error in `failed`, a summary. A new conversation is "New conversation" (a heading)
  and a rule.
- **The account** (`AccountLine`), over the rows: the email at the start in `secondaryText`, at the end "Weekly limit
  72%", with ", resets Mon 14:00" when the origin read when (the weekday and time within six days, else the date), in
  `attention` from 80% and `failed` at 100%; the limit goes under the email when they don't fit. VoiceOver reads one
  sentence.
- **Earlier messages:** the list's first row asks for the page before as soon as it shows ("Loading earlier messages"
  with a small spinner); the page goes in above and the row that was on top stays where it was. At the start it says
  "Start of the conversation" over the working directory in mono; a page that failed says why in `failed` with Retry.
- **Needs-you cards** (`NeedsYouCard`), pinned under the list, an approval before a question: the raised ground with
  20 pt top corners, the attention diamond over the title (a heading), at most 60% of the chat's height, scrolling
  inside. An approval ("Allow Bash?", the call's title in `chatCode`, its detail on a plate up to 160 pt tall, then the
  dialog's options, the first filled with `prominentAction()`, the rest outlined in the tint), with the agent's own
  answers standing in until the pane's screen is read. A question ("Claude Code asks", each question's header, radio
  marks or checkboxes as the asks have them, a "Type an answer" row, **Send answers** once every question has one),
  also for Claude's permission prompt that carries the questions before its transcript does, and for Codex's.
  While an answer is on its way the buttons hold and "Sending" shows under them.
- **The activity line** (`ActivityStrip`), over the composer, on the ground and on one line: the connecting ring at 12
  pt, standing still, "Claude Code is working" while the agent works on a turn, else "Still running in the background"
  while calls have no result yet, with how many ("2 subagents, 1 command"); with any running it is a 44 pt toggle that
  lists them. **Stop**, the word in the tint, ends the line while a turn the agent works on wasn't interrupted. A card
  waiting takes the foot alone.
- **Problems:** what the last action couldn't do, as a `Banner` over the card: `attention` when the user can act on it,
  `failed` when it only reports; **Answer in terminal** when the terminal is the way on; dismissible.
- **States:** the backlog on its way ("Reading the conversation from build-01"); nothing said yet (the chat glyph,
  "Nothing said yet", **Show terminal**); reconnecting (a strip over the rows: "Reconnecting to build-01", "What's here
  stays; new messages follow once it's back."); ended ("This chat ended", **Show terminal**, which the pane then does
  by itself).
- **The composer** (`ChatComposerView`), on the system's bar material: one pill (the raised ground, 22 pt corners, 44
  pt tall) holding **Attach** (`paperclip`) at its start, the prompt ("Prompt, or / for commands", growing to six
  lines, then scrolling inside), and at its end one place that **Commands** (a `/` in the terminal's face, which starts
  a command and opens the menu) gives up to **Send** (`arrow.turn.down.left` in the tint) once there is text; the one
  leaving shrinks and fades as the other grows in (120 ms; a cut with Reduce Motion). Send takes the text with its ends
  trimmed; the field lets it go at once, keeping the keyboard, so the next prompt can be typed while this one is on its
  way, and takes it back, ahead of anything typed since, when it wasn't sent. While an action is on its way Send is a
  spinner ("Sending"). While a card waits, "Answer the card above first: a prompt now would go to its dialog." stands
  over the field and Send holds. A draft left in a chat that closes comes back to the pane's next chat of the same
  conversation, a prompt still on its way then ahead of it.
- **Attaching a file** (`AttachButton`, `UploadLine`, `ChatUploads`): the paperclip's menu offers **Photo or video**
  (the system's photo picker, in its compatible encoding, so a HEIC photo goes as JPEG) and **File** (the document
  picker, any type); neither asks for a permission. The file goes to the pane's machine (`ngtermctl agent upload`, 32
  MB at most, refused before a byte is read when larger). Over the pill: "Uploading shot.png", the percentage and
  **Cancel upload** over a progress bar; "Saving shot.png" with no Cancel once every byte has gone; done, the stored
  path goes in at the cursor with a space on each side (quoted when it holds a space); stopped short, "Couldn't attach
  shot.png: …" in `failed` with Dismiss. Cancel goes through the core's cancel token (`UploadCancel`), as a Swift task's
  cancel can't stop the core's call.
- **The `/` menu** (`SlashCommandMenu`): a `/` at the start of the field, with no space or line break after it, opens
  it over the foot of the rows (over the card while one waits); what follows narrows it, ranked by the core's
  `filter_commands`. Rows (56 pt): the command as it goes in, in `chatCode`, its argument hint quiet after it, a line
  of what it does, and its kind at the end (Built-in, Skill, Command, Plugin); about six show. Under them a quiet line
  says how many match or that none does ("No command matches /deploy. It goes as typed."), or that the commands are on
  their way or not listed. Escape closes it until the text changes.
- **A hardware keyboard:** ⌘Return sends; while the menu lists commands, Up and Down move its highlight and Return or
  Tab picks it; Escape closes the menu. Return alone is a line break.
- **VoiceOver:** the chat is "Chat with Claude Code"; a prompt reads "You: …"; a line reads its kind, title and status,
  says Expanded or Collapsed, and its hint names the tap; the cards' titles are headings; the reconnecting strip,
  Sending, the menu's count and an upload's failure are announced.

### Asks (`Asks/`)

An attached session's origin asks the user something (a button ask, a text to type, a choice). While the session
shows, the ask is a sheet over it; out of sight it is a notification (below).

- **The sheet** is a page of the prompts' shape (`AskQuestion`): the attention diamond, the ask's title as a heading,
  its message in `secondaryText`, its form, then its buttons in the origin's order, one per line, full width, at least
  44 pt tall:
  - the default is filled (`prominentAction()`), in `failed` when it is also the destructive one;
  - the cancel button is quiet (the tint on the ground, no container);
  - the rest are outlined: a 1 pt rounded outline in the tint, or in `failed` for a destructive one, with the label in
    the same color. A tinted fill under a label stays under 4.5:1, so none is used.
- **An input ask** is one field on the raised grouped surface (12 pt corners), one line or an area that grows from three
  lines to eight, starting from the origin's text, with its hint ("Your answer" when the origin gives none). Return in a
  line field presses the default button (or the one answering button). An edit that would add a control character the
  field can't hold is refused whole, as desktop refuses it.
- **A choice ask** is one grouped surface of rows, each at least 44 pt: a radio mark (`circle`,
  `inset.filled.circle`) or a checkbox (`square`, `checkmark.square.fill`) in the tint, the label and its subtitle in
  `secondaryText`. VoiceOver reads a row's selected state (and a checkbox as a toggle), not the mark. **The free-form
  row** has its mark and a field in place of the label ("Something else" unless the origin names it): typing picks it
  (a radio list hands its pick over, and gets it back when the text goes blank); its mark takes a tap only while it
  holds text, greyed until then.
- **Dismissing** the sheet with a swipe answers as the cancel button does (the origin gets Escape). A sheet whose ask
  went (answered elsewhere, ended on the origin, taken back as the app suspends) goes by itself. A draft outlives its
  sheet while the ask is open.

**The question about notifying** shows once, as the first session is attached, and only while the user hasn't answered
the system's question: the bell (`bell.badge`) in the tint, "Get told when an agent needs you" as a heading, what
reaches the user and that a question can be answered from the notification, then when notifications come (while the
app is open, for a short time after it, and from a refresh now and then: iOS keeps no connection up in the
background). **Turn on notifications** (prominent) raises the system's question; **Not now** (quiet), or a swipe,
leaves it for Settings.

### Settings (`Settings/`)

A sheet from the machines' toolbar (the gear), a `NavigationStack` over a `Form`, **Done** in the toolbar. The set is
Android's, without its wallpaper colors. Every section's header and note are the app's `FormHint`s; a choice is one of
the section's rows, its title over what it means in `secondaryText`, the chosen one checked in the tint (the system's
inline `Picker`); a switch is the system's `Toggle`, tinted copper.

- **Terminal:** a live sample of a shell (a prompt, a listing, `git status` with a line selected, the cursor) in the
  chosen font, size and palette, on the palette's ground with 12 pt corners; it keeps the terminal's own size in
  points, not Dynamic Type, and VoiceOver reads it as an image ("A sample of the terminal in Copper at 13 points").
  Then **Text size**, a slider of whole points from 8 to 32 with its value in mono beside its name, each step written
  as it is made (the sample follows); and **Keep the screen on** ("While an attached session is open").
- **Font:** JetBrains Mono, or the system monospace (SF Mono).
- **Colors:** Match the theme (Copper when dark, Drafting when light), Copper, Drafting, Ghostty, each with a swatch:
  the palette's ground with its eight normal ANSI colors (88 × 20 pt, decorative).
- **Appearance:** System, Light or Dark. It is the window's own style (`overrideUserInterfaceStyle`, `appAppearance`),
  so everything in a window follows, sheets and the theme's terminal palette included, every window alike.
- **Agents:** Agent chat view, a switch, on by default: a pane running Claude Code or Codex shows as a chat. A
  session's More menu sets its own either way (Chat view, above).
- **Extra keys:** Fit to the width, Always two lines, or Hidden (the soft keyboard alone, which on an iPhone has no
  key that closes it; the note says so). A session's bar follows a change at once.
- **Notifications:** one switch per kind (Waiting for you, Finished, Agent notifications, Lost machines), each with
  what it tells; the note says questions from agents always notify. While the app may not notify, an attention banner
  heads the section: **Turn on** raises the system's question when it hasn't been answered, **Open Settings** opens the
  system's settings for the app once the user said no. The permission is read again as the app comes back.
- **Clipboard:** each machine trusted to read it unasked, its name over its mono address, with **Ask again** (read as
  "Ask again before nas reads the clipboard"); with none, that every machine asks.
- **Keys:** each key ("Created on this device" or "Imported", a heading) with its mono fingerprint, "Used by build-01
  and nas" or "No machine uses it", and **Show public key** and **Copy public key** (side by side, or stacked when they
  don't fit). Then **Rotate key** (a spinner and "Making a key" while it runs; why it failed, under it) and **Import a
  key** (the first run's import screen, pushed), with what rotating means. Neither confirms: nothing is lost.
- **A key just made or brought** opens its public half (the first run's screen, pushed) with, under it, "Then switch
  your machines", what to do first, and **Use for every machine** (prominent); once every machine uses it, "Every
  machine signs in with this key." The machines connected keep their connections and sign in with it next time.
- **About:** "ngterm for iOS" and the core's version in mono.
- **States:** the stores are read at launch, so there is no loading state; a change a store couldn't write is a failed
  banner at the sheet's foot, dismissible, the setting left as it was. The one exception is a clipboard **Ask again**:
  it still takes effect until the app closes, and the banner says so.

### Notifications

Each machine's and session's notifications group in their own thread; a tap opens the session and its pane. A pane
shown as its chat opens at its newest row, following again, with the card the agent waits on pinned at its foot; one
showing its terminal opens as its terminal. They show as banners in the foreground too, except for the session on
screen.

| Notice | Title | Under it | Text | Actions |
|---|---|---|---|---|
| an agent's notice | its title (the session's name when blank) | the session | its body | |
| waiting | "api is waiting for you" | | the context, or "On build-01" | |
| done | "api finished" | | the context, or "On build-01" | |
| lost for good | "Lost build-01" | | the core's reason, or the changed host key's advice | |
| an ask | its title | "api on build-01" | its message | its buttons (at most three); an input ask's answering buttons take a reply; a choice ask, or more buttons, says "Tap to answer in ngterm." |

An ask's action asks for the device to be unlocked first, and a destructive button draws as one. The kinds the user
turned off (Settings) are not posted; an ask always is.

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
