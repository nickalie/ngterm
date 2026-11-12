import NgtermCore

/// The device's clipboard as the terminal uses it (mobile/android's `DeviceClipboard` and `ClipboardWriter`): text for
/// the user's own copy and paste, and every type a program's clipboard reads and writes reach (the Kitty clipboard
/// protocol), text as `text/plain`.
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

    /// The MIME types it holds, asked without reading their data.
    func types() -> [String]

    /// Each of `mimes` it holds, in that order; the others are skipped.
    func read(_ mimes: [String]) -> [ClipboardPart]

    /// Puts what it can keep of `contents` on it (none empties it), and says how that went.
    func write(_ contents: [ClipboardPart]) -> ClipboardWriteStatus
}
