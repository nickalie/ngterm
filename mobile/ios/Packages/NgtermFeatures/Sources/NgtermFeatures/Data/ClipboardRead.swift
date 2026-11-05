import NgtermCore

/// A program's read of the clipboard (OSC 52) waiting for the user's answer: `listOnly` asks only what it holds.
public struct ClipboardRead: Hashable, Identifiable, Sendable {
    /// The pane whose program reads.
    public let pane: PaneRef
    /// The read's id, which the answer names.
    public let readId: UInt64
    /// Whether it asks only which kinds of content the clipboard holds.
    public let listOnly: Bool

    /// The read, as its pane and id name it.
    public var id: Self {
        self
    }

    /// The read `readId` of the program in `pane`.
    public init(pane: PaneRef, readId: UInt64, listOnly: Bool) {
        self.pane = pane
        self.readId = readId
        self.listOnly = listOnly
    }
}
