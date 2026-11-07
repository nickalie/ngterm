import NgtermDesign
import SwiftUI

/// The pane's conversation switched here (mobile/android's `ConversationDivider`): "New conversation", a heading, and
/// a rule; what came before it is gone.
internal struct ConversationDivider: View {
    internal var body: some View {
        HStack(spacing: Spacing.medium) {
            Text(String(localized: "New conversation"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Palette.secondaryText.color)
                .accessibilityAddTraits(.isHeader)
            Rectangle()
                .fill(Color(uiColor: .separator))
                .frame(height: 1)
                .accessibilityHidden(true)
        }
        .frame(minHeight: Spacing.hitTarget)
    }
}
