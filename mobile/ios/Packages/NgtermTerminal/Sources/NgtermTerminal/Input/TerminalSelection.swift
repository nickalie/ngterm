import NgtermCore

/// The selection's side of the pane a ``TerminalView`` shows (mobile/android's `TerminalSelection`).
///
/// It holds a touch selection with its handles and edit menu, and a paste waiting for its confirmation, and makes the
/// selection and clipboard calls they ask for. Each call that changes the pane's selection marks the view's pane
/// dirty, so the next frame shows it. A pane with no terminal now drops the call.
@preconcurrency
@MainActor
public final class TerminalSelection {
    // a single, a double (word) and a triple (line) click, then round again
    private static let clickKinds = 3

    /// A touch made the selection: its handles and its edit menu show while it has any.
    public private(set) var isTouchSelection = false
    /// A finger is moving the selection: the edit menu keeps out of its way.
    public internal(set) var isAdjusting = false
    /// Text the pane refused to paste as unsafe, waiting for the user's yes; `nil` with none.
    public private(set) var unsafePaste: String?
    /// The edit menu shows once a frame shows the touch selection.
    public internal(set) var wantsMenu = false

    private let pane: any PaneClient
    private let clipboard: any TerminalClipboard
    private let changed: @MainActor () -> Void
    // a touch selection is let go once a frame shows it gone (copied, cleared by output), not before its first frame
    private var selectionSeen = false

    /// Whether there is a selection to copy.
    public var hasSelection: Bool {
        pane.selectionText() != nil
    }

    /// The selection of `pane`, copying to and pasting from `clipboard`; `changed` runs after each change to the pane's
    /// selection (the view marks its frame dirty).
    @preconcurrency
    public init(pane: any PaneClient, clipboard: any TerminalClipboard, changed: @escaping @MainActor () -> Void) {
        self.pane = pane
        self.clipboard = clipboard
        self.changed = changed
    }

    /// Selects from `start` to `end` (either order).
    public func select(from start: CellPoint, to end: CellPoint) {
        try? pane.select(start: start, end: end, rectangle: false)
        changed()
    }

    /// Starts a touch selection at `cell`: the word there, or the cell itself where there is none.
    public func selectWord(at cell: CellPoint) {
        touch {
            if try pane.selectWord(at: cell) {
                return true
            }
            try pane.select(start: cell, end: cell, rectangle: false)
            return true
        }
    }

    /// Starts a touch selection of the line at `cell`, across soft wraps.
    public func selectLine(at cell: CellPoint) {
        touch { try pane.selectLine(at: cell) }
    }

    /// A mouse's click number `count` in a row at `cell`: the first lets the selection go, a second selects the word,
    /// a third the line, then round again.
    public func click(_ count: Int, at cell: CellPoint) {
        isTouchSelection = false
        switch (count - 1) % Self.clickKinds {
        case 0:
            try? pane.clearSelection()

        case 1:
            _ = try? pane.selectWord(at: cell)

        default:
            _ = try? pane.selectLine(at: cell)
        }
        changed()
    }

    /// Selects everything on screen and in the scrollback; a touch selection keeps its handles and shows its menu
    /// again.
    public func selectAll() {
        _ = try? pane.selectAll()
        wantsMenu = isTouchSelection
        changed()
    }

    /// Drops the selection, and with it the handles and the menu.
    public func clear() {
        isTouchSelection = false
        wantsMenu = false
        try? pane.clearSelection()
        changed()
    }

    /// Puts the selection's text on the clipboard and lets the selection go; false with none.
    @discardableResult
    public func copy() -> Bool {
        guard let text = pane.selectionText() else {
            return false
        }
        clipboard.write(text)
        clear()
        return true
    }

    /// Pastes the clipboard's text, as ``paste(_:allowUnsafe:)`` does; nothing when it holds none.
    @discardableResult
    public func pasteClipboard() -> Bool {
        guard let text = clipboard.text(), !text.isEmpty else {
            return false
        }
        return paste(text)
    }

    /// Pastes `text` into the program; whether it waits in ``unsafePaste`` for the user's yes, as text that can run a
    /// command (a line break the program would take as Enter) does unless `allowUnsafe`.
    @discardableResult
    public func paste(_ text: String, allowUnsafe: Bool = false) -> Bool {
        unsafePaste = nil
        if isTouchSelection {
            clear()
        }
        let outcome = try? pane.paste(text: text, allowUnsafe: allowUnsafe)
        if outcome == .unsafe {
            unsafePaste = text
            return true
        }
        return false
    }

    /// The user said yes to ``unsafePaste``: it is pasted as it is.
    public func confirmPaste() {
        if let text = unsafePaste {
            paste(text, allowUnsafe: true)
        }
    }

    /// The user said no to ``unsafePaste``.
    public func cancelPaste() {
        unsafePaste = nil
    }

    /// A frame came with `selection`: a touch selection whose frames no longer show it is over.
    public func shown(_ selection: [SelectedSpan]) {
        if !selection.isEmpty {
            selectionSeen = true
        } else if isTouchSelection, selectionSeen, !isAdjusting {
            isTouchSelection = false
            wantsMenu = false
        }
    }

    // a touch selection made by `select`, unless it found nothing to select
    private func touch(_ select: () throws -> Bool) {
        wantsMenu = false
        if (try? select()) == true {
            isTouchSelection = true
            selectionSeen = false
        }
        changed()
    }

    deinit {
        // nothing to release
    }
}
