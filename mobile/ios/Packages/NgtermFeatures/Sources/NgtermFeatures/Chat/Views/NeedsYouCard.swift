import NgtermDesign
import SwiftUI

/// A card pinned at the chat's foot while the agent waits on the user (mobile/android's `NeedsYouCard`), in the ask
/// sheet's shape: the raised ground with its top corners rounded, the attention diamond over a title that is a heading,
/// then its content.
///
/// It scrolls inside when it is taller than `maxHeight`.
internal struct NeedsYouCard<Content: View>: View {
    // the ask sheet's top corners: the card rises from the chat's foot as the sheet does from the screen's
    private static var corner: CGFloat {
        Radius.medium + Spacing.small
    }

    internal let title: String
    internal let maxHeight: CGFloat
    @ViewBuilder internal let content: () -> Content

    internal var body: some View {
        ViewThatFits(in: .vertical) {
            card
            ScrollView {
                card
            }
            .frame(maxHeight: maxHeight)
        }
        .frame(maxWidth: .infinity)
        .background(
            ChatGrounds.raised,
            in: UnevenRoundedRectangle(
                topLeadingRadius: Self.corner,
                topTrailingRadius: Self.corner,
                style: .continuous
            )
        )
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: Spacing.medium) {
            Image(systemName: StatusShape.diamond.symbolName)
                .font(.title3)
                .foregroundStyle(Palette.attention.color)
                .accessibilityHidden(true)
            Text(title)
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
                .fixedSize(horizontal: false, vertical: true)
            content()
        }
        .padding(.horizontal, Spacing.xLarge)
        .padding(.vertical, Spacing.large)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
