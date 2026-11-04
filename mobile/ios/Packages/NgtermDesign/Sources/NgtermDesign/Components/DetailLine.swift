import SwiftUI

/// A row's line of what its state means, with the action it calls for at its end, or under it when they don't fit.
internal struct DetailLine: View {
    internal let detail: String?
    internal let action: RowAction?

    internal var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
                text
                Spacer(minLength: 0)
                button
            }
            VStack(alignment: .leading, spacing: Spacing.xSmall) {
                text
                button
            }
        }
    }

    @ViewBuilder private var text: some View {
        if let detail {
            Text(detail)
                .font(Typography.detail)
                .foregroundStyle(Palette.secondaryText.color)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var button: some View {
        if let action {
            // the row reads as one element and offers the action as its custom action, so VoiceOver skips this one
            QuietButton(action: action)
                .font(Typography.detail.weight(.semibold))
                .accessibilityHidden(true)
        }
    }
}
