/// The device's clipboard as the terminal uses it: text in and out (mobile/android's `ClipboardText` and
/// `ClipboardSink`).
///
/// The app's is ``SystemClipboard``; tests give their own.
@preconcurrency
@MainActor
public protocol TerminalClipboard: AnyObject {
    /// Whether it holds text, asked without reading it (so iOS doesn't ask the user to allow a paste).
    var hasText: Bool { get }

    /// The text it holds; `nil` when it holds none.
    func text() -> String?

    /// Puts `text` on it; `nil` empties it.
    func write(_ text: String?)
}
