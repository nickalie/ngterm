import Foundation
import NgtermCore

/// What a program's clipboard read would hand over, as the question shows it.
///
/// Ghostty's macOS form (desktop's `clipboard_preview::summary`): the `text/plain` text when it is UTF-8, else a "type
/// (N bytes)" line per part, and the first image part, which the view shows when it decodes.
public struct ClipboardSummary: Equatable, Sendable {
    /// The most bytes of text the question lays out (64 KiB); the rest is counted (desktop's `bounded_text`).
    public static let maxText = 65_536

    // glib's `g_format_size` steps by a thousand
    private static let step = 1_000
    private static let units = ["kB", "MB", "GB"]

    /// The text: what was copied, or what kinds of content and how much; `nil` when there are no parts.
    public let text: String?
    /// The first image part's bytes and type.
    public let image: ClipboardPart?

    /// The summary of `contents`.
    public init(_ contents: [ClipboardPart]) {
        text = contents.isEmpty ? nil : Self.utf8Text(of: contents).map(Self.bounded) ?? Self.kinds(of: contents)
        image = contents.first { $0.mime.hasPrefix("image/") }
    }

    /// `text` cut at ``maxText`` bytes on a character's edge, with a line saying how much is left out.
    public static func bounded(_ text: String) -> String {
        let utf8 = text.utf8
        guard utf8.count > maxText else {
            return text
        }
        var end = utf8.index(utf8.startIndex, offsetBy: maxText)
        while end > utf8.startIndex, String.Index(end, within: text) == nil {
            end = utf8.index(before: end)
        }
        let kept = String(text[..<end])
        return "\(kept)\n… \(byteCount(utf8.count - kept.utf8.count)) more"
    }

    /// A byte count as Ghostty's dialog says it (glib's `g_format_size`): "1 byte", "73 bytes", "2.0 kB", "1.5 MB".
    public static func byteCount(_ size: Int) -> String {
        if size == 1 {
            return "1 byte"
        }
        if size < step {
            return "\(size) bytes"
        }
        var value = Double(size) / Double(step)
        var unit = 0
        while value >= Double(step), unit < units.count - 1 {
            value /= Double(step)
            unit += 1
        }
        return String(format: "%.1f %@", locale: Locale(identifier: "en_US_POSIX"), value, units[unit])
    }

    // the text/plain part, when it is UTF-8
    private static func utf8Text(of contents: [ClipboardPart]) -> String? {
        contents.first { $0.mime == "text/plain" }.flatMap { String(bytes: $0.data, encoding: .utf8) }
    }

    // a line per part: its type and size
    private static func kinds(of contents: [ClipboardPart]) -> String {
        contents.map { "\($0.mime) (\($0.data.count) bytes)" }.joined(separator: "\n")
    }
}
