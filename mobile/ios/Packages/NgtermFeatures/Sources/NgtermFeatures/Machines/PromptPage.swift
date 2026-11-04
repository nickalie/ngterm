import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The page of a question about a machine's connection (a host key to trust, a sign-in, a changed host key): the
/// tone's glyph, the question as a heading, the machine's address, what it means, then the page's own content.
///
/// VoiceOver reads the address in words.
internal struct PromptPage<Content: View>: View {
    internal let tone: StatusTone
    internal let title: String
    internal let machine: StoredMachine
    internal let message: String
    @ViewBuilder internal let content: () -> Content

    internal var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                Image(systemName: tone.shape.symbolName)
                    .font(.title2)
                    .foregroundStyle(tone.color.color)
                    .accessibilityHidden(true)
                Text(title)
                    .font(.title2.bold())
                    .accessibilityAddTraits(.isHeader)
                Text(verbatim: machine.address)
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(machine.spokenAddress)
                Text(message)
                    .foregroundStyle(Palette.secondaryText.color)
                    .fixedSize(horizontal: false, vertical: true)
                content()
            }
            .frame(maxWidth: OnboardingLayout.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }
}
