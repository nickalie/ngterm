import UIKit

/// The confirmation before pasting text the pane refused as unsafe (mobile/android's `PasteConfirmDialog`): it has a
/// line break or a control character, so the program may take it as Enter and run it at once.
///
/// The system's alert: what the paste can do, the text's first lines as they are, then Cancel (the safe default, and
/// what dismissing does) and Paste, which repeats the menu's word.
public enum PasteConfirmation {
    /// The most lines of the text the alert shows.
    public static let previewLines = 6
    /// The most characters of a line the alert shows.
    public static let previewWidth = 60

    /// The alert's title.
    public static var title: String {
        String(localized: "Paste text that can run a command?")
    }

    /// What the alert says the paste can do, then the first lines of `text`.
    public static func message(for text: String) -> String {
        let explanation = String(
            localized: """
                It has a line break or a control character in it, so the program may take it as Enter and run it as \
                soon as it lands.
                """
        )
        return explanation + "\n\n" + preview(of: text)
    }

    /// The first ``previewLines`` lines of `text`, each cut at ``previewWidth`` characters, its control characters
    /// shown as symbols; an ellipsis marks what was cut.
    public static func preview(of text: String) -> String {
        let lines = text.split(omittingEmptySubsequences: false) { $0 == "\n" || $0 == "\r" || $0 == "\r\n" }
        var shown = lines.prefix(previewLines).map { line in
            let plain = String(line.unicodeScalars.map(Self.visible))
            return plain.count > previewWidth ? String(plain.prefix(previewWidth)) + "…" : plain
        }
        if lines.count > previewLines {
            shown.append("…")
        }
        return shown.joined(separator: "\n")
    }

    /// The alert asking to paste `text`: `onPaste` pastes it as it is, `onCancel` doesn't.
    @MainActor
    @preconcurrency
    public static func alert(
        for text: String,
        onPaste: @escaping @MainActor () -> Void,
        onCancel: @escaping @MainActor () -> Void
    ) -> UIAlertController {
        let alert = UIAlertController(title: title, message: message(for: text), preferredStyle: .alert)
        alert.addAction(
            UIAlertAction(title: String(localized: "Cancel"), style: .cancel) { _ in onCancel() }
        )
        alert.addAction(
            UIAlertAction(title: String(localized: "Paste"), style: .default) { _ in onPaste() }
        )
        return alert
    }

    // a control character as its Control Pictures symbol (␛ for ESC), anything else as it is
    private static func visible(_ scalar: Unicode.Scalar) -> Character {
        let controlPictures: UInt32 = 0x2400
        let delete: UInt32 = 0x7F
        let space: UInt32 = 0x20
        if scalar.value < space, let picture = Unicode.Scalar(controlPictures + scalar.value) {
            return Character(picture)
        }
        if scalar.value == delete, let picture = Unicode.Scalar(controlPictures + space + 1) {
            return Character(picture)
        }
        return Character(scalar)
    }
}
