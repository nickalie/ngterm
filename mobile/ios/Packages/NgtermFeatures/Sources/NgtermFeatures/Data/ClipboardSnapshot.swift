import NgtermCore

/// What the clipboard holds of a read's types: each one it holds, and the types it holds when the read asked.
public struct ClipboardSnapshot: Equatable, Sendable {
    /// Each requested type the clipboard holds, in the read's order.
    public let contents: [ClipboardPart]
    /// The types the clipboard holds, when the read asked for them.
    public let available: [String]

    /// What a read would hand over.
    public init(contents: [ClipboardPart], available: [String]) {
        self.contents = contents
        self.available = available
    }
}
