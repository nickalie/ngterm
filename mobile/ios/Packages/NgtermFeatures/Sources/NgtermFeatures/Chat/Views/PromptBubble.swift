import NgtermDesign
import SwiftUI

/// A prompt the user sent (mobile/android's `PromptBubble`): a bubble at the end of the line on the raised ground, no
/// wider than a reading line, with how many images went with it.
///
/// VoiceOver reads "You: …", then the images.
internal struct PromptBubble: View {
    internal let text: String
    internal let images: Int

    internal var body: some View {
        let carried: String? =
            switch images {
            case 0:
                nil

            case 1:
                String(localized: "With 1 image")

            default:
                String(localized: "With \(images) images")
            }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        HStack {
            Spacer(minLength: Spacing.xxxLarge)
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(verbatim: trimmed)
                    .font(.body)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                if let carried {
                    Text(carried)
                        .font(.footnote)
                        .foregroundStyle(Palette.secondaryText.color)
                }
            }
            .padding(.horizontal, Spacing.medium)
            .padding(.vertical, Spacing.small)
            .frame(maxWidth: ChatStyle.bubbleWidth, alignment: .leading)
            .background(ChatGrounds.raised, in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(
                [String(localized: "You: \(trimmed)"), carried].compactMap(\.self).joined(separator: ". ")
            )
        }
        .padding(.vertical, Spacing.small)
    }
}
