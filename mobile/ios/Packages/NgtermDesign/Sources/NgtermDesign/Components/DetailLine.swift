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
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var button: some View {
        if let action {
            Button(action.title, action: action.perform)
                .font(Typography.detail.weight(.semibold))
                .buttonStyle(.borderless)
                .frame(minHeight: Spacing.hitTarget)
        }
    }
}
