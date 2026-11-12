import Foundation
import NgtermCore
import NgtermTerminal

/// A clipboard in memory, for tests: no system pasteboard, so nothing asks to allow a paste.
///
/// Its text is ``held``; a program's writes and reads see ``parts``, every type it holds (text as `text/plain`, which
/// is ``held``).
@MainActor
internal final class MemoryClipboard: TerminalClipboard {
    /// What it holds.
    internal var held: String?

    /// Every type it holds but its text.
    internal var others: [ClipboardPart] = []

    /// Every type it holds, text first.
    internal var parts: [ClipboardPart] {
        (held.map { [ClipboardPart(mime: "text/plain", data: Data($0.utf8))] } ?? []) + others
    }

    internal var hasText: Bool {
        held != nil
    }

    /// Every program's write, in order (none: emptied).
    internal private(set) var partWrites: [[ClipboardPart]] = []

    internal init(_ held: String?) {
        self.held = held
    }

    internal func text() -> String? {
        held
    }

    internal func write(_ text: String?) {
        held = text
    }

    internal func types() -> [String] {
        parts.map(\.mime)
    }

    internal func read(_ mimes: [String]) -> [ClipboardPart] {
        mimes.compactMap { mime in parts.first { $0.mime == mime } }
    }

    internal func write(_ contents: [ClipboardPart]) -> ClipboardWriteStatus {
        held = contents.first { $0.mime == "text/plain" }.flatMap { String(bytes: $0.data, encoding: .utf8) }
        others = contents.filter { $0.mime != "text/plain" }
        partWrites.append(contents)
        return .done
    }

    deinit {
        // nothing to release
    }
}
