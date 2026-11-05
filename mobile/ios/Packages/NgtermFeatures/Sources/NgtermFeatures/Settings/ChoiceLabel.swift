import NgtermDesign
import SwiftUI

/// A setting's words: its title over what it means, in the app's secondary text color (the system's grey stays under
/// 4.5:1 on the grouped ground).
internal struct ChoiceLabel: View {
    internal let title: String
    internal let note: String?

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xxSmall) {
            Text(title)
            if let note {
                Text(note)
                    .font(Typography.note)
                    .foregroundStyle(Palette.secondaryText.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }

    internal init(_ title: String, note: String?) {
        self.title = title
        self.note = note
    }
}
