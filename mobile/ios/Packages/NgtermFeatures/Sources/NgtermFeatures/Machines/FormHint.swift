import NgtermDesign
import SwiftUI

/// A form section's header or footer in the app's own type and secondary text color.
///
/// The system's header and footer text is a grey under 4.5:1 on the grouped ground, and stops growing at the largest
/// text sizes; this one reads on every ground and scales all the way.
internal struct FormHint: View {
    /// Whether it heads a section or notes one.
    internal enum Role {
        case header
        case note
    }

    internal let text: String
    internal let role: Role

    internal var body: some View {
        Text(text)
            .font(role == .header ? Typography.plateLabel : Typography.note)
            .foregroundStyle(Palette.secondaryText.color)
            .textCase(nil)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityAddTraits(role == .header ? .isHeader : [])
    }

    internal init(_ text: String, _ role: Role) {
        self.text = text
        self.role = role
    }
}
