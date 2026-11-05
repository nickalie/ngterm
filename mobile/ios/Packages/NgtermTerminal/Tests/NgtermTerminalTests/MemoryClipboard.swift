import NgtermTerminal

/// A clipboard in memory, for tests: no system pasteboard, so nothing asks to allow a paste.
@MainActor
internal final class MemoryClipboard: TerminalClipboard {
    /// What it holds.
    internal var held: String?

    internal var hasText: Bool {
        held != nil
    }

    internal init(_ held: String?) {
        self.held = held
    }

    internal func text() -> String? {
        held
    }

    internal func write(_ text: String?) {
        held = text
    }

    deinit {
        // nothing to release
    }
}
