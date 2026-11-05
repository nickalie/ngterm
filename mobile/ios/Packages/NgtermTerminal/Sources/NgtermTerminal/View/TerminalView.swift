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
public final class TerminalView: UIView {
    /// The pane it shows: a new pane needs a new view.
    public let pane: any PaneClient

    /// The colors the terminal starts from, and the selection's.
    public var palette: TerminalPalette {
        didSet {
            guard palette != oldValue else {
                return
            }
            sendColors()
            redrawAll()
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

    /// Whether the display link runs: while the view is in a window and a frame waits.
    public var readsFrames: Bool {
        link.map { !$0.isPaused } ?? false
    }

    private var glyphs: GlyphCache
    private var painter: TerminalPainter
    private var rowLayers: [TerminalRowLayer] = []
    private var link: CADisplayLink?
    private var restart = true

    private var scale: CGFloat {
        max(traitCollection.displayScale, 1)
    }

    private var look: TerminalLook {
        TerminalLook(selection: palette.selectionBackground.argb, focused: drawsFocused)
    }

    /// A terminal showing `pane` in `palette`, at `textSize` points in `faces` (the bundled JetBrains Mono by default).
    public init(
        pane: any PaneClient,
        palette: TerminalPalette,
        textSize: CGFloat,
        faces: TerminalFaces? = Fonts.terminalFaces
    ) {
        self.pane = pane
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
            return
        }
        let made = CADisplayLink(target: self, selector: #selector(step))
        made.isPaused = !framePending
        made.add(to: .main, forMode: .common)
        link = made
    }

    /// Draws row `index` into `context` (its row layer's), as the screen has it now.
    internal func drawRow(_ index: Int, in context: CGContext) {
        painter.draw(row: index, of: screen, width: bounds.width, look: look, in: context)
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

    private func sendColors() {
        // a pane with no terminal gives its next one the colors anyway
        try? pane.setColors(colors: palette.paneColors)
    }

    deinit {
        // the display link holds the view while it is in a window, and stops when it leaves (didMoveToWindow)
    }
}
