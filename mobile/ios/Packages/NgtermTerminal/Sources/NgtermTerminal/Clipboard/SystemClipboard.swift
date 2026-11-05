import UIKit

/// The system's general pasteboard, as text.
///
/// iOS asks the user before an app reads what another app copied, unless the read is the user's own paste (the edit
/// menu's Paste, ⌘V): ``text()`` from anywhere else may show that question.
@preconcurrency
@MainActor
public final class SystemClipboard: TerminalClipboard {
    /// Whether the general pasteboard holds text.
    public var hasText: Bool {
        UIPasteboard.general.hasStrings
    }

    /// The general pasteboard.
    public init() {
        // the general pasteboard is the system's
    }

    /// The general pasteboard's text.
    public func text() -> String? {
        UIPasteboard.general.string
    }

    /// Puts `text` on the general pasteboard, or empties it.
    public func write(_ text: String?) {
        if let text {
            UIPasteboard.general.string = text
        } else {
            UIPasteboard.general.items = []
        }
    }

    deinit {
        // nothing to release
    }
}
