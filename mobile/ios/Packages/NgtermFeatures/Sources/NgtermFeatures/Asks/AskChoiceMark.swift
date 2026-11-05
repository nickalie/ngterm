import NgtermDesign
import SwiftUI

/// A choice's mark, in a touch target's room.
///
/// A radio mark (a ring, filled when picked) or a checkbox, in the tint; greyed while it can't be picked. VoiceOver
/// reads its row's selected state instead.
internal struct AskChoiceMark: View {
    internal let isMultiple: Bool
    internal let isChecked: Bool
    internal let isEnabled: Bool

    internal var body: some View {
        Image(systemName: symbol)
            .font(.title3)
            .foregroundStyle(isEnabled ? Palette.tint.color : Palette.secondaryText.color)
            .frame(width: Spacing.hitTarget, height: Spacing.hitTarget)
            .accessibilityHidden(true)
    }

    private var symbol: String {
        switch (isMultiple, isChecked) {
        case (true, true):
            "checkmark.square.fill"

        case (true, false):
            "square"

        case (false, true):
            "inset.filled.circle"

        case (false, false):
            "circle"
        }
    }
}
