import Foundation

/// A shown pane of an attached session, as the live core's handle and the fake one both offer it.
///
/// ``MobileCoreClient/showPane(machineId:sessionId:paneId:size:)`` hands it out, and input goes to its terminal. Its
/// rules are the Rust `Pane`'s (crates/ngterm-mobile, api/pane.rs). Every method may be called from any thread; a
/// hidden pane refuses input with ``PaneInputError/Closed``.
public protocol PaneClient: AnyObject, Sendable {
    /// Drops the selection.
    func clearSelection() throws

    /// The pane gained or lost focus; whether a report was sent to the program.
    func focus(focused: Bool) throws -> Bool

    /// The rows to draw that changed after frame `sinceSeq` (0 for every row), packed as crates/ngterm-mobile's
    /// rows.rs lays out; pass the buffer's `seq` to the next call, and 0 after
    /// ``MobileEvent/paneAttached(pane:)``.
    func frame(sinceSeq: UInt64) throws -> Data

    /// The pane's identity in its session, as the session's layout names it.
    func id() -> String

    /// Sends a key, encoded for the program's keyboard modes; whether anything was sent.
    func key(key: PaneKey) throws -> Bool

    /// Reports a pointer event to the program when it asked for the mouse; whether anything was reported.
    func mouse(mouse: PaneMouse) throws -> Bool

    /// Whether the program takes pointer events: while it does, a drag is the program's, not a selection.
    func mouseTracking() -> Bool

    /// Pastes `text`, refusing text that could run a command (``PaneSendOutcome/unsafe``) unless `allowUnsafe`.
    func paste(text: String, allowUnsafe: Bool) throws -> PaneSendOutcome

    /// The view's new grid: the terminal reflows and the machine's pty follows when the pane leads.
    func resize(size: PaneSize) throws

    /// Selects from `start` to `end` (either order), as a run of text or a `rectangle`.
    func select(start: CellPoint, end: CellPoint, rectangle: Bool) throws

    /// Selects everything; false on a blank screen.
    func selectAll() throws -> Bool

    /// Selects the line at `point`, across soft wraps; false on a blank line.
    func selectLine(at point: CellPoint) throws -> Bool

    /// Selects the word at `point`; false where there is none.
    func selectWord(at point: CellPoint) throws -> Bool

    /// The selection's text as a copy takes it; `nil` with none.
    func selectionText() -> String?

    /// The view's theme: the colors the terminal starts from and a reset restores.
    func setColors(colors: PaneColors) throws

    /// The grid the view last gave the pane.
    func size() -> PaneSize

    /// Types `text` as the keyboard would; whether anything was sent.
    func typeText(text: String) throws -> Bool

    /// Routes a wheel turn or scroll gesture: to the program, or scrolling the view.
    func wheel(wheel: PaneWheel) throws -> PaneWheelOutcome
}
