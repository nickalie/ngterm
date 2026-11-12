import NgtermCore
import UIKit
import UniformTypeIdentifiers

/// The system's general pasteboard (or another, for tests).
///
/// iOS asks the user before an app reads what another app copied, unless the read is the user's own paste (the edit
/// menu's Paste, ⌘V): ``text()`` and ``read(_:)`` from anywhere else may show that question. ``hasText`` and
/// ``types()`` read no data, so they never do.
///
/// A program's write keeps its text (`public.utf8-plain-text`), its HTML (`public.html`) and its images (the type of
/// each image's MIME type), one pasteboard item with each as a representation; other types aren't kept. iOS adds RTF
/// beside HTML on its own, so a listing names `text/rtf` too.
@preconcurrency
@MainActor
public final class SystemClipboard: TerminalClipboard {
    private static let textType = UTType.utf8PlainText.identifier
    private static let htmlType = UTType.html.identifier

    private let pasteboard: UIPasteboard

    /// Whether the pasteboard holds text.
    public var hasText: Bool {
        pasteboard.hasStrings
    }

    /// The general pasteboard, or `pasteboard`.
    public init(_ pasteboard: UIPasteboard = .general) {
        self.pasteboard = pasteboard
    }

    // the pasteboard type a program's MIME type is kept as: text, HTML and images only
    private static func kept(_ mime: String) -> String? {
        switch mime {
        case "text/plain":
            return textType

        case "text/html":
            return htmlType

        default:
            guard mime.hasPrefix("image/"), let type = UTType(mimeType: mime), type.conforms(to: .image) else {
                return nil
            }
            return type.identifier
        }
    }

    // a pasteboard type's MIME type: every text type is `text/plain`; a type with none is left out
    private static func mime(of identifier: String) -> String? {
        guard let type = UTType(identifier) else {
            return nil
        }
        if type.conforms(to: .html) {
            return "text/html"
        }
        if type.conforms(to: .plainText) {
            return "text/plain"
        }
        return type.preferredMIMEType
    }

    /// The pasteboard's text.
    public func text() -> String? {
        pasteboard.string
    }

    /// Puts `text` on the pasteboard, or empties it.
    public func write(_ text: String?) {
        if let text {
            pasteboard.string = text
        } else {
            pasteboard.items = []
        }
    }

    /// The MIME types of the pasteboard's first item, text as `text/plain`.
    public func types() -> [String] {
        var mimes: [String] = []
        for identifier in pasteboard.types {
            guard let mime = Self.mime(of: identifier), !mimes.contains(mime) else {
                continue
            }
            mimes.append(mime)
        }
        return mimes
    }

    /// Each of `mimes` the pasteboard's first item holds, in that order: only a type it lists itself, never data a
    /// broader type (`public.data` for `application/octet-stream`) would hand over.
    public func read(_ mimes: [String]) -> [ClipboardPart] {
        let held = Set(pasteboard.types)
        var seen: Set<String> = []
        return mimes.compactMap { mime in
            guard seen.insert(mime).inserted else {
                return nil
            }
            if mime == "text/plain" {
                return pasteboard.string.map { ClipboardPart(mime: mime, data: Data($0.utf8)) }
            }
            guard let type = UTType(mimeType: mime), held.contains(type.identifier),
                let data = pasteboard.data(forPasteboardType: type.identifier)
            else {
                return nil
            }
            return ClipboardPart(mime: mime, data: data)
        }
    }

    /// Puts the text, HTML and images of `contents` on the pasteboard as one item; none empties it.
    public func write(_ contents: [ClipboardPart]) -> ClipboardWriteStatus {
        guard !contents.isEmpty else {
            pasteboard.items = []
            return .done
        }
        var item: [String: Any] = [:]
        for part in contents {
            guard let type = Self.kept(part.mime), item[type] == nil else {
                continue
            }
            if type == Self.textType {
                // text that isn't UTF-8 is Latin-1 (X11's STRING, the one other text type a program names)
                item[type] = String(bytes: part.data, encoding: .utf8) ?? String(bytes: part.data, encoding: .isoLatin1)
            } else {
                item[type] = part.data
            }
        }
        guard !item.isEmpty else {
            return .unsupported
        }
        pasteboard.setItems([item])
        return .done
    }

    deinit {
        // nothing to release
    }
}
