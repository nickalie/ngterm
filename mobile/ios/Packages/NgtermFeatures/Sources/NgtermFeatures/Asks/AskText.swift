/// The rules an ask's typed text keeps to (ngterm-core's `AskInputForm`, mobile/android's `AskDraft.kt`).
public enum AskText {
    /// The most UTF-8 bytes an ask's text may hold (`AskInputForm::MAX_TEXT_BYTES` in ngterm-core).
    public static let maxBytes = 65_536
    // the C0 controls end below the space; DEL is the other control
    private static let controlEnd: UInt32 = 0x20
    private static let delete: UInt32 = 0x7f

    /// Whether `text` has no character but whitespace.
    public static func isBlank(_ text: String) -> Bool {
        text.allSatisfy(\.isWhitespace)
    }

    /// Whether `text` holds a C0 control or DEL, a line break and a tab excepted in a `multiline` field (ngterm-core's
    /// `has_forbidden_controls`): a field refuses an edit that would add one, as desktop does.
    public static func hasForbiddenControls(_ text: String, multiline: Bool) -> Bool {
        text.unicodeScalars.contains { isForbidden($0, multiline: multiline) }
    }

    /// `text` as a field of the kind (`multiline`) can answer it, for text typed where no field can refuse an edit (a
    /// notification's reply): its forbidden controls turned to spaces, then cut to the cap.
    public static func fit(_ text: String, multiline: Bool) -> String {
        var scalars = String.UnicodeScalarView()
        for scalar in text.unicodeScalars {
            scalars.append(isForbidden(scalar, multiline: multiline) ? " " : scalar)
        }
        return clamp(String(scalars))
    }

    /// `text` cut to ``maxBytes`` UTF-8 bytes at the end of the last whole character that fits.
    public static func clamp(_ text: String) -> String {
        guard text.utf8.count > maxBytes else {
            return text
        }
        var bytes = 0
        var end = text.startIndex
        for index in text.indices {
            bytes += text[index].utf8.count
            if bytes > maxBytes {
                break
            }
            end = text.index(after: index)
        }
        return String(text[..<end])
    }

    private static func isForbidden(_ scalar: Unicode.Scalar, multiline: Bool) -> Bool {
        let isControl = scalar.value < controlEnd || scalar.value == delete
        return isControl && (!multiline || (scalar != "\n" && scalar != "\t"))
    }
}
