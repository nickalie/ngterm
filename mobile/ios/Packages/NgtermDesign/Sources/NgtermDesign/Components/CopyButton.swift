import SwiftUI
import UIKit

/// Puts `text` on the clipboard, then says Copied for a moment (and VoiceOver says it too).
public struct CopyButton: View {
    // how long the button says Copied
    private static let copiedSeconds = 2.0

    @State private var isCopied = false
    private let text: String
    private let title: String

    /// The body of the view.
    public var body: some View {
        Button {
            UIPasteboard.general.string = text
            isCopied = true
            AccessibilityNotification.Announcement(String(localized: "Copied", bundle: .module)).post()
        } label: {
            Label(
                isCopied ? String(localized: "Copied", bundle: .module) : title,
                systemImage: isCopied ? "checkmark" : "doc.on.doc"
            )
            .font(Typography.detail.weight(.semibold))
            .frame(minHeight: Spacing.hitTarget)
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .task(id: isCopied) {
            guard isCopied else {
                return
            }
            try? await Task.sleep(for: .seconds(Self.copiedSeconds))
            isCopied = false
        }
    }

    /// Copies `text`; `title` is the button's words (Copy unless given).
    public init(_ text: String, title: String? = nil) {
        self.text = text
        self.title = title ?? String(localized: "Copy", bundle: .module)
    }
}
