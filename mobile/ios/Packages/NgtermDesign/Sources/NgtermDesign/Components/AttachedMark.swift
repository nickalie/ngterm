import SwiftUI

/// The mark of a session this app has attached: a link glyph and "Attached", in verdigris.
internal struct AttachedMark: View {
    internal var body: some View {
        Label(String(localized: "Attached", bundle: .module), systemImage: "link")
            .labelStyle(.titleAndIcon)
            .font(Typography.statusWord)
            .foregroundStyle(Palette.healthy.color)
            .lineLimit(1)
            .fixedSize()
    }
}
