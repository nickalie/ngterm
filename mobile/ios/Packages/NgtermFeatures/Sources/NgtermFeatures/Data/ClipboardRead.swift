import NgtermCore

/// A program's read of the clipboard waiting for the user's answer (mobile/android's `ClipboardRead`).
///
/// It asks for `mimes` (text as `text/plain`) and, with `list`, the types the clipboard holds; `canRemember` when an
/// Allow may keep a grant for the program in its terminal (the Kitty protocol's password). A `listOnly` read asks only
/// which types it holds.
public struct ClipboardRead: Hashable, Identifiable, Sendable {
    /// The pane whose program reads.
    public let pane: PaneRef
    /// The read's id, which the answer names.
    public let readId: UInt64
    /// Whether it asks only which kinds of content the clipboard holds.
    public let listOnly: Bool
    /// The types it wants, in its order of preference.
    public let mimes: [String]
    /// Whether it also wants the types the clipboard holds.
    public let list: Bool
    /// Whether an Allow may keep a grant for the program in its terminal.
    public let canRemember: Bool

    /// The read, as its pane and id name it.
    public var id: Self {
        self
    }

    /// The read `readId` of the program in `pane`.
    public init(
        pane: PaneRef,
        readId: UInt64,
        listOnly: Bool,
        mimes: [String] = ["text/plain"],
        list: Bool = false,
        canRemember: Bool = false
    ) {
        self.pane = pane
        self.readId = readId
        self.listOnly = listOnly
        self.mimes = mimes
        self.list = list
        self.canRemember = canRemember
    }
}
