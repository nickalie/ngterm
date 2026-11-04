import SwiftUI

/// A veil over a pane that can't be used as it is: another device drives it, it is attaching, its program ended.
///
/// Its glyph (a spinner for none), title, what it means, and the action that fixes it. It takes every touch, and
/// `covered(by:)` hides the pane under it from VoiceOver.
public struct Cover: View {
    private let systemImage: String?
    private let title: String
    private let message: String
    private let action: RowAction?

    /// The body of the view.
    public var body: some View {
        ZStack {
            Rectangle()
                .fill(.regularMaterial)
                .contentShape(Rectangle())
                .onTapGesture {
                    // the veil takes the touch, so nothing reaches the pane under it
                }
                .accessibilityHidden(true)
            VStack(spacing: Spacing.medium) {
                if let systemImage {
                    Image(systemName: systemImage)
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                } else {
                    ProgressView()
                        .controlSize(.large)
                }
                Text(title)
                    .font(Typography.coverTitle)
                    .multilineTextAlignment(.center)
                    .accessibilityAddTraits(.isHeader)
                Text(message)
                    .font(Typography.note)
                    .foregroundStyle(Palette.secondaryText.color)
                    .multilineTextAlignment(.center)
                if let action {
                    Button(action.title, action: action.perform)
                        .prominentAction()
                }
            }
            .frame(maxWidth: Spacing.textWidth)
            .padding(Spacing.xLarge)
        }
    }

    /// A cover with `systemImage` (a spinner for `nil`), `title`, `message` and its `action`.
    public init(systemImage: String?, title: String, message: String, action: RowAction? = nil) {
        self.systemImage = systemImage
        self.title = title
        self.message = message
        self.action = action
    }
}
