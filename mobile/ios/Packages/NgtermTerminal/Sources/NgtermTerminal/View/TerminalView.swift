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
public final class TerminalView: UIView {
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

    /// The colors the terminal starts from, and the selection's.
    public var palette: TerminalPalette {
        didSet {
            guard palette != oldValue else {
                return
            }
            sendColors()
            redrawAll()
            refreshHandles()
        }
    }

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
    public var drawsFocused = true {
        didSet {
            if drawsFocused != oldValue, let cursor = screen.header?.cursor {
                redraw(rows: [cursor.row])
            }
        }
    }

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

    /// It takes the focus, and the edit menu's actions with it.
    override public var canBecomeFirstResponder: Bool {
        true
    }

    private var glyphs: GlyphCache
    private var painter: TerminalPainter
    private var rowLayers: [TerminalRowLayer] = []
    private var link: CADisplayLink?
    private var restart = true
    private let handles = SelectionHandles()
    private lazy var editMenu = TerminalEditMenu(view: self)

    private var scale: CGFloat {
        max(traitCollection.displayScale, 1)
    }

    private var look: TerminalLook {
        TerminalLook(selection: palette.selectionBackground.argb, focused: drawsFocused)
    }

    /// A terminal showing `pane` in `palette`, at `textSize` points in `faces` (the bundled JetBrains Mono by default).
    ///
    /// Copies and pastes go through `clipboard`, the system's by default.
    public init(
        pane: any PaneClient,
        palette: TerminalPalette,
        textSize: CGFloat,
        faces: TerminalFaces? = Fonts.terminalFaces,
        clipboard: (any TerminalClipboard)? = nil
    ) {
        self.pane = pane
        self.clipboard = clipboard ?? SystemClipboard()
        self.palette = palette
        self.textSize = textSize
        self.faces = faces
        let made = TerminalFont(size: textSize, faces: faces)
        font = made
        glyphs = GlyphCache(font: made)
        painter = TerminalPainter(font: made, glyphs: glyphs)
        super.init(frame: .zero)
        isOpaque = true
        clipsToBounds = true
        backgroundColor = palette.background.uiColor
        registerForTraitChanges([UITraitDisplayScale.self]) { (view: Self, _: UITraitCollection) in
            view.setNeedsLayout()
        }
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
    }

    /// Lays the rows out over the bounds and gives the pane the grid that fits them.
    override public func layoutSubviews() {
        super.layoutSubviews()
        layoutRows()
        handles.frame = bounds
        refreshHandles()
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
    }

    /// Takes the focus (the first responder), as a tap or a click on the terminal does.
    public func focus() {
        _ = becomeFirstResponder()
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

    /// Shows a touch selection's handles where the screen has it, and the edit menu over it once it is wanted.
    internal func refreshHandles() {
        let spots = selection.isTouchSelection ? HandleSpots(screen.selection, font: font, size: bounds.size) : nil
        handles.show(spots, color: palette.cursor.uiColor.cgColor)
        guard let spots else {
            // the touch selection went: its menu goes with it, a secondary click's stays
            if !selection.isTouchSelection {
                editMenu.hideOverSelection()
            }
            return
        }
        if selection.wantsMenu, !selection.isAdjusting {
            selection.wantsMenu = false
            editMenu.show(over: spots.rows)
        }
    }

    /// Shows the edit menu at `point` (a secondary click).
    internal func showMenu(at point: CGPoint) {
        editMenu.show(at: point)
    }

    /// Takes the edit menu down.
    internal func hideMenu() {
        editMenu.hide()
    }

    @objc
    private func step(_: CADisplayLink) {
        drawFrame()
    }

    // a layer per row of the screen, each a cell high across the bounds, at the screen's scale
    private func layoutRows() {
        while rowLayers.count < screen.rowCount {
            let row = TerminalRowLayer()
            row.index = rowLayers.count
            row.view = self
            layer.addSublayer(row)
            rowLayers.append(row)
        }
        while rowLayers.count > screen.rowCount {
            rowLayers.removeLast().removeFromSuperlayer()
        }
        for row in rowLayers {
            if row.contentsScale != scale {
                row.contentsScale = scale
                row.setNeedsDisplay()
            }
            row.frame = CGRect(x: 0, y: font.top(row.index), width: bounds.width, height: font.cellHeight)
        }
    }

    private func redraw(rows: some Sequence<Int>) {
        for index in rows where rowLayers.indices.contains(index) {
            rowLayers[index].setNeedsDisplay()
        }
    }

    private func redrawAll() {
        redraw(rows: rowLayers.indices)
    }

    private func remakeFont() {
        font = TerminalFont(size: textSize, faces: faces)
        glyphs = GlyphCache(font: font)
        painter = TerminalPainter(font: font, glyphs: glyphs)
        setNeedsLayout()
        redrawAll()
    }

    // asks before pasting `text`, which could run a command, in an alert over whatever the window shows
    private func confirmPaste(_ text: String) {
        let alert = PasteConfirmation.alert(for: text) { [weak self] in
            self?.selection.confirmPaste()
        } onCancel: { [weak self] in
            self?.selection.cancelPaste()
        }
        var presenter = window?.rootViewController
        while let shown = presenter?.presentedViewController {
            presenter = shown
        }
        presenter?.present(alert, animated: true)
    }

    private func sendColors() {
        // a pane with no terminal gives its next one the colors anyway
        try? pane.setColors(colors: palette.paneColors)
    }

    deinit {
        // the display link holds the view while it is in a window, and stops when it leaves (didMoveToWindow)
    }
}
