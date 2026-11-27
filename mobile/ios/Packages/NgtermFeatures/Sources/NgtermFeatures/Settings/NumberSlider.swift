import NgtermDesign
import SwiftUI

/// A setting of whole numbers: its label with the value beside it in mono, a slider over `range`, and what it sets.
internal struct NumberSlider: View {
    internal let label: String
    internal let value: Int
    internal let range: ClosedRange<Int>
    /// The value as shown beside the label, as "13 pt".
    internal let shown: String
    /// The value as VoiceOver says it, as "13 points".
    internal let spoken: String
    internal let note: String
    internal let identifier: String
    internal let onChange: (Int) -> Void

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            HStack(alignment: .firstTextBaseline) {
                Text(label)
                Spacer()
                Text(shown)
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
            }
            .accessibilityElement(children: .combine)
            Slider(
                value: Binding {
                    Double(value)
                } set: { picked in
                    let whole = Int(picked.rounded())
                    if whole != value {
                        onChange(whole)
                    }
                },
                in: Double(range.lowerBound)...Double(range.upperBound),
                step: 1
            ) {
                Text(label)
            }
            .accessibilityValue(Text(spoken))
            .accessibilityIdentifier(identifier)
            FormHint(note, .note)
        }
        .padding(.vertical, Spacing.xSmall)
    }
}
