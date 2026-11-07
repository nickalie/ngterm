import NgtermDesign
import SwiftUI

/// A tool's output on a plate, in the terminal's face, scrolling sideways as the terminal would cut it.
internal struct OutputPlate: View {
    internal let text: String

    internal var body: some View {
        ScrollView(.horizontal) {
            Text(verbatim: text.trimmingCharacters(in: .newlines))
                .font(Typography.chatOutput)
                .fixedSize(horizontal: true, vertical: false)
                .textSelection(.enabled)
                .padding(Spacing.medium)
        }
        .scrollIndicators(.hidden)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(ChatGrounds.plate, in: RoundedRectangle(cornerRadius: Radius.small, style: .continuous))
    }
}
