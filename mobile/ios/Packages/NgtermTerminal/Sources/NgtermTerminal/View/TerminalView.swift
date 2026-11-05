import NgtermCore
import NgtermDesign
import UIKit

/// A shown pane's terminal: its rows drawn with Core Text, each in its own layer, from the pane's packed frames
/// (`Pane.frame`), with the cursor and the selection (mobile/android's `TerminalState` and `TerminalView`).
///
/// Its owner routes the pane's events to it: ``dirty()`` on `PaneDirty`, ``attached()`` on `PaneAttached` (a fresh
/// terminal, read from scratch). A display link reads at most one frame per screen refresh, asking for the rows changed
/// since the frame it holds, and runs only while a frame waits: it pauses after a frame read, and runs on while the
/// terminal answers with a stale one (it was busy). The frame is read on the main thread: the core hands out the frame
/// its terminal made last, never waiting for it. A frame redraws only the rows that look different
/// (``TerminalScreen``). A pane with no terminal (hidden, between attaches) keeps the last screen on view.
///
/// It fills its bounds with whole cells from the top left and gives the pane the grid that fits (``grid``), again as
/// its size changes (rotation, Stage Manager, the split), and the colors of ``palette``. With ``drawsFocused`` the
/// cursor takes its shape; an unfocused terminal shows a hollow block.
///
/// A finger's gestures (``gestures``) and a pointer's (``pointerGestures``) scroll, click, select and zoom it through
/// ``pointer`` and ``selection``; a touch selection shows its handles and the edit menu (Copy, Paste, Select All),
/// whose standard edit actions it answers as the first responder, as it answers ⌘C, ⌘V and ⌘A. A paste the pane
/// refuses as unsafe waits for the user's yes (``PasteConfirmation``).
///
/// As the first responder it takes the keyboard (``keyboard``): it is the soft keyboard's text input, its composition
/// drawn at the cursor (``TerminalTextInput``), a hardware keyboard's keys and chords come to it, and the extra keys
/// sit over the keyboard. Being the first responder and showing the soft keyboard are apart: a tap opens the keyboard,
/// where a click, a long press or the edit menu only take the focus (``focus(showingKeyboard:)``).
public final class TerminalView: UIView {
    // how far up the composition sits: over the rows (0), under the handles (1)
    private static let compositionDepth: CGFloat = 0.5
    // the edit menu's items: the standard edit actions, with their titles
    internal static let editActions: [(Selector, String)] = [
        (#selector(copy(_:)), String(localized: "Copy")),
        (#selector(paste(_:)), String(localized: "Paste")),
        (#selector(selectAll(_:)), String(localized: "Select All")),
    ]

    /// The pane it shows: a new pane needs a new view.
    public let pane: any PaneClient
    /// The clipboard its copies go to and its pastes come from.
    public let clipboard: any TerminalClipboard
    /// The keyboard it takes while it is the first responder, which every terminal on screen shares.
    public let keyboard: TerminalKeyboard

    /// No automatic capitals: a terminal types commands and passwords.
    @objc public var autocapitalizationType = UITextAutocapitalizationType.none
    /// No autocorrection.
    @objc public var autocorrectionType = UITextAutocorrectionType.no
    /// No spell check.
    @objc public var spellCheckingType = UITextSpellCheckingType.no
    /// No smart quotes.
    @objc public var smartQuotesType = UITextSmartQuotesType.no
    /// No smart dashes.
    @objc public var smartDashesType = UITextSmartDashesType.no
    /// No smart spaces around what is inserted or deleted.
    @objc public var smartInsertDeleteType = UITextSmartInsertDeleteType.no
    /// No inline predictions.
    @objc public var inlinePredictionType = UITextInlinePredictionType.no
    /// No math completion.
    @objc public var mathExpressionCompletionType = UITextMathExpressionCompletionType.no
    /// No Writing Tools: what is typed goes to the program at once.
    @objc public var writingToolsBehavior = UIWritingToolsBehavior.none

    /// The colors the terminal starts from, and the selection's.
    public var palette: TerminalPalette {
        didSet {
            guard palette != oldValue else {
                return
            }
            sendColors()
            redrawAll()
            refreshHandles()
            placeComposition()
        }
    }

    /// The text size Settings gives, which ⌘0 goes back to.
    public var defaultTextSize: CGFloat
    /// Whether a tap asked for the soft keyboard: while it is the first responder, the keyboard shows; a click, a long
    /// press or the edit menu take the focus without it.
    public internal(set) var softKeyboardWanted = false

    /// The text size, in points.
    public var textSize: CGFloat {
        didSet {
            if textSize != oldValue {
                remakeFont()
            }
        }
    }

    /// The font's faces: `nil` is the system's monospace.
    public var faces: TerminalFaces? {
        didSet {
            if faces != oldValue {
                remakeFont()
            }
        }
    }

    /// Whether the terminal has the focus: an unfocused one shows its cursor as a hollow block.
    ///
    /// The session view draws its selected pane focused: one that becomes so in a window takes the keyboard's focus
    /// when the last selected one handed it over (``TerminalKeyboard/handOver()``).
    public var drawsFocused = true {
        didSet {
            if drawsFocused != oldValue, let cursor = screen.header?.cursor {
                redraw(rows: [cursor.row])
            }
            takeHandover()
        }
    }

    /// What a touch or a click on it, or its taking the focus, does: the session view selects its pane.
    public var onSelect: (@MainActor () -> Void)?
    /// Where a finger's sideways swipe goes (`true` forward: a swipe to the left); with none, a drag only scrolls.
    public var onSwipe: (@MainActor (Bool) -> Void)?
    /// Whether VoiceOver hears its cursor's line as it changes (the selected pane's), at most once a second.
    public var readsAloud = false

    /// The grid it last gave the pane; `nil` until it has a size that holds a cell.
    public private(set) var grid: PaneSize?
    /// The rows read so far.
    public private(set) var screen = TerminalScreen()
    /// Whether a frame waits to be read: the display link runs while one does.
    public private(set) var framePending = true
    /// The font it draws in.
    public private(set) var font: TerminalFont

    /// The pointer's side of the pane: cells, clicks, the wheel, flings and the text size.
    public private(set) lazy var pointer = TerminalPointer(view: self)
    /// The selection's side of the pane: the touch selection, copy and paste.
    public private(set) lazy var selection = TerminalSelection(pane: pane, clipboard: clipboard) { [weak self] in
        self?.dirty()
    }
    /// A finger's gestures.
    public private(set) lazy var gestures = TerminalGestures(view: self)
    /// A mouse's and a trackpad's.
    public private(set) lazy var pointerGestures = TerminalPointerGestures(view: self)

    /// Whether the display link runs: while the view is in a window and a frame waits.
    public var readsFrames: Bool {
        link.map { !$0.isPaused } ?? false
    }

    /// Where a touch selection's handles stand; `nil` while none shows.
    public var handleSpots: HandleSpots? {
        handles.spots
    }

    /// Whether the edit menu shows.
    public var isMenuShown: Bool {
        editMenu.isShown
    }

    /// It takes the focus, and the edit menu's actions and the keyboard with it.
    override public var canBecomeFirstResponder: Bool {
        true
    }

    /// The soft keyboard while a tap asked for it; else a stand-in with no keys, so the terminal keeps the focus (and a
    /// hardware keyboard's keys) with the extra keys at the screen's foot.
    override public var inputView: UIView? {
        softKeyboardWanted ? nil : noKeyboard
    }

    /// The extra keys, over the soft keyboard, unless a hardware keyboard is attached.
    override public var inputAccessoryView: UIView? {
        keyboard.showsExtraKeys ? keyboard.bar : nil
    }

    // swiftlint:disable discouraged_optional_collection - UIResponder declares it optional
    /// The app's chords, listed while ⌘ is held (``shortcutCommands``).
    override public var keyCommands: [UIKeyCommand]? {
        shortcutCommands
    }
    // swiftlint:enable discouraged_optional_collection

    /// What VoiceOver reads of it: its screen's text, line by line (``accessibilityPageContent()``).
    override public var accessibilityValue: String? {
        get {
            accessibilityPageContent()
        }
        set {
            super.accessibilityValue = newValue
        }
    }

    /// The composition, and where it draws.
    internal let textInput = TerminalTextInput()
    /// What says the cursor's line to VoiceOver.
    internal lazy var announcer = CursorAnnouncer()
    /// The hardware keys taken as the program's while they are held, and the repeat of the one held last.
    internal let heldKeys = HeldKeys()
    /// The soft keyboard's stand-in while it isn't wanted: no keys, so the extra keys sit at the foot of the screen.
    internal let noKeyboard = UIView(frame: .zero)

    private var glyphs: GlyphCache
    private var painter: TerminalPainter
    /// The rows' layers, one per row of the screen.
    internal var rowLayers: [TerminalRowLayer] = []
    private var link: CADisplayLink?
    private var restart = true
    /// A touch selection's handles.
    internal let handles = SelectionHandles()
    /// The edit menu.
    internal lazy var editMenu = TerminalEditMenu(view: self)

    internal var scale: CGFloat {
        max(traitCollection.displayScale, 1)
    }

    private var look: TerminalLook {
        TerminalLook(selection: palette.selectionBackground.argb, focused: drawsFocused)
    }

    /// A terminal showing `pane` in `palette`, at `textSize` points in `faces` (the bundled JetBrains Mono by default).
    ///
    /// Copies and pastes go through `clipboard`, the system's by default, and keys come through `keyboard`, its own by
    /// default.
    public init(
        pane: any PaneClient,
        palette: TerminalPalette,
        textSize: CGFloat,
        faces: TerminalFaces? = Fonts.terminalFaces,
        clipboard: (any TerminalClipboard)? = nil,
        keyboard: TerminalKeyboard? = nil
    ) {
        self.pane = pane
        self.clipboard = clipboard ?? SystemClipboard()
        self.keyboard = keyboard ?? TerminalKeyboard()
        self.palette = palette
        self.textSize = textSize
        defaultTextSize = textSize
        self.faces = faces
        let made = TerminalFont(size: textSize, faces: faces)
        font = made
        glyphs = GlyphCache(font: made)
        painter = TerminalPainter(font: made, glyphs: glyphs)
        super.init(frame: .zero)
        isOpaque = true
        clipsToBounds = true
        // a text input VoiceOver reads line by line (UIAccessibilityReadingContent, View/TerminalView+Reading)
        isAccessibilityElement = true
        accessibilityLabel = String(localized: "Terminal")
        backgroundColor = palette.background.uiColor
        registerForTraitChanges([UITraitDisplayScale.self]) { (view: Self, _: UITraitCollection) in
            view.setNeedsLayout()
        }
        // the composition over the rows, under the selection's handles, in depth and in order (a render of the layer
        // keeps the order and leaves the depth out); the rows go in under both
        textInput.layer.zPosition = Self.compositionDepth
        layer.addSublayer(textInput.layer)
        handles.zPosition = 1
        layer.addSublayer(handles)
        _ = (gestures, pointerGestures, editMenu)
        sendColors()
    }

    @available(*, unavailable, message: "a terminal view is made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// The pane has a new frame (`PaneDirty`): the display link reads it on its next tick.
    public func dirty() {
        framePending = true
        link?.isPaused = false
    }

    /// The pane has a fresh terminal (`PaneAttached`): the next frame is read whole.
    public func attached() {
        restart = true
        dirty()
    }

    /// Reads the pane's next frame and marks the rows it changed for drawing: what a display-link tick does.
    public func drawFrame() {
        framePending = false
        let since = restart ? 0 : screen.seq
        let update: FrameUpdate
        do {
            update = try RowBufferReader.read(pane.frame(sinceSeq: since))
        } catch {
            // no terminal now (hidden, or between attaches), or a frame this build can't read: the last screen stays
            // until the next one
            link?.isPaused = true
            return
        }
        restart = false
        let changed = screen.apply(update)
        glyphs.trim(to: screen.rowCount)
        layoutRows()
        redraw(rows: changed)
        backgroundColor = UIColor(cgColor: .argb(update.header.defaultBackground))
        selection.shown(screen.selection)
        refreshHandles()
        placeComposition()
        if readsAloud {
            announcer.cursorLine(screen.text(update.header.cursor.row))
        }
        // the terminal was busy and this is the frame it made last: ask again on the next tick
        framePending = update.header.stale
        link?.isPaused = !framePending
    }

    /// Draws the rows marked for drawing now, as the next screen update would.
    public func displayNow() {
        layoutIfNeeded()
        for row in rowLayers {
            row.displayIfNeeded()
        }
        textInput.layer.displayIfNeeded()
    }

    /// Lays the rows out over the bounds and gives the pane the grid that fits them.
    override public func layoutSubviews() {
        super.layoutSubviews()
        layoutRows()
        handles.frame = bounds
        refreshHandles()
        placeComposition()
        let fits = font.grid(fitting: bounds.size, scale: scale)
        guard let fits, fits != grid else {
            return
        }
        grid = fits
        // a pane with no terminal keeps the size for its next one
        try? pane.resize(size: fits)
    }

    /// Runs the display link while the view is in a window, and stops it when it leaves.
    override public func didMoveToWindow() {
        super.didMoveToWindow()
        link?.invalidate()
        link = nil
        guard window != nil else {
            pointer.stopFling()
            hideMenu()
            return
        }
        let made = CADisplayLink(target: self, selector: #selector(step))
        made.isPaused = !framePending
        made.add(to: .main, forMode: .common)
        link = made
        takeHandover()
    }

    /// A touch or a click on it selects it (``onSelect``), whatever it goes on to do.
    override public func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        onSelect?()
        super.touchesBegan(touches, with: event)
    }

    /// Takes the focus (the first responder), as a tap or a click on the terminal does, opening the soft keyboard when
    /// `showingKeyboard` (a tap); without it, a keyboard showing stays and none opens.
    public func focus(showingKeyboard: Bool = false) {
        if showingKeyboard, !softKeyboardWanted {
            softKeyboardWanted = true
            if isFirstResponder {
                reloadInputViews()
            }
        }
        _ = becomeFirstResponder()
    }

    /// Takes the keyboard: every key goes here now.
    override public func becomeFirstResponder() -> Bool {
        tookKeyboard(super.becomeFirstResponder())
    }

    /// Lets the keyboard go: the composition is finished here first (``letKeyboardGo(_:)``).
    override public func resignFirstResponder() -> Bool {
        commitComposition()
        return letKeyboardGo(super.resignFirstResponder())
    }

    /// A hardware keyboard's keys: what the program takes goes to it (``keyDown(_:)``), the rest to the text system.
    override public func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = untaken(presses, by: keyDown)
        if !rest.isEmpty {
            super.pressesBegan(rest, with: event)
        }
    }

    /// Keys let go: those the program took are released to it (``keyUp(_:)``), the rest go to the text system.
    override public func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = untaken(presses, by: keyUp)
        if !rest.isEmpty {
            super.pressesEnded(rest, with: event)
        }
    }

    /// Keys the system took back: as if let go.
    override public func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = untaken(presses, by: keyUp)
        if !rest.isEmpty {
            super.pressesCancelled(rest, with: event)
        }
    }

    /// Whether the edit action `action` can run now: Copy with a selection, Paste with text on the clipboard, Select
    /// All always.
    override public func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        switch action {
        case #selector(copy(_:)):
            selection.hasSelection

        case #selector(paste(_:)):
            clipboard.hasText

        case #selector(selectAll(_:)):
            true

        default:
            super.canPerformAction(action, withSender: sender)
        }
    }

    /// Copy: the selection's text goes on the clipboard, and the selection goes.
    override public func copy(_: Any?) {
        selection.copy()
    }

    /// Paste: the clipboard's text goes to the program, once the user says yes when it could run a command.
    override public func paste(_: Any?) {
        if selection.pasteClipboard(), let text = selection.unsafePaste {
            confirmPaste(text)
        }
    }

    /// Select All: everything on screen and in the scrollback.
    override public func selectAll(_: Any?) {
        selection.selectAll()
    }

    /// Draws row `index` into `context` (its row layer's), as the screen has it now.
    internal func drawRow(_ index: Int, in context: CGContext) {
        painter.draw(row: index, of: screen, width: bounds.width, look: look, in: context)
    }

    @objc
    private func step(_: CADisplayLink) {
        drawFrame()
    }

    private func remakeFont() {
        font = TerminalFont(size: textSize, faces: faces)
        glyphs = GlyphCache(font: font)
        painter = TerminalPainter(font: font, glyphs: glyphs)
        setNeedsLayout()
        redrawAll()
        placeComposition()
    }

    deinit {
        // the display link holds the view while it is in a window, and stops when it leaves (didMoveToWindow)
    }
}
