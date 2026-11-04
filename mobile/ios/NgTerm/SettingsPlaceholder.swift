import NgtermDesign
import SwiftUI

/// Settings until its screen lands: the core's version.
internal struct SettingsPlaceholder: View {
    @Environment(\.dismiss)
    private var dismiss

    internal let coreVersion: String

    internal var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent(String(localized: "Core")) {
                        Text(verbatim: coreVersion)
                            .font(Fonts.mono(.body))
                    }
                    .accessibilityIdentifier("core-version")
                }
            }
            .navigationTitle(String(localized: "Settings"))
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Done")) {
                        dismiss()
                    }
                }
            }
        }
    }
}
