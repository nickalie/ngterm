import NgtermDesign
import SwiftUI

/// The app's name and the loaded core's version.
internal struct AboutSection: View {
    internal let coreVersion: String

    internal var body: some View {
        Section {
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text("ngterm for iOS")
                Text("Core \(Text(verbatim: coreVersion).font(Typography.address))")
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
                    .textSelection(.enabled)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("core-version")
        } header: {
            FormHint(String(localized: "About"), .header)
        }
    }
}
