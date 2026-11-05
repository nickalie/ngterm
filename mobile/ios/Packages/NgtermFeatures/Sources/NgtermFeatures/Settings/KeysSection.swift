import NgtermDesign
import SwiftUI

/// The app's keys, each with its fingerprint, the machines that use it, Show public key and Copy; then Rotate key (a
/// spinner and "Making a key" while it runs, and why it failed) and Import a key, with what rotating means.
internal struct KeysSection: View {
    internal let model: SettingsModel
    /// Shows the public half of the key with the id given.
    internal let onShow: @MainActor (String) -> Void
    /// Opens the import.
    internal let onImport: @MainActor () -> Void

    internal var body: some View {
        let work = model.work
        Section {
            ForEach(model.keys) { item in
                KeyRow(item: item) {
                    onShow(item.id)
                }
            }
            Button {
                Task {
                    await model.rotate()
                }
            } label: {
                HStack(spacing: Spacing.small) {
                    if work.isRotating {
                        ProgressView()
                    }
                    Text(work.isRotating ? String(localized: "Making a key") : String(localized: "Rotate key"))
                }
            }
            .disabled(work.isRotating)
            .accessibilityIdentifier("rotate-key")
            Button(String(localized: "Import a key")) {
                onImport()
            }
            .accessibilityIdentifier("import-key")
        } header: {
            FormHint(String(localized: "Keys"), .header)
        } footer: {
            VStack(alignment: .leading, spacing: Spacing.small) {
                if let failure = work.rotateFailure {
                    InlineProblem(String(localized: "A key couldn't be made: \(failure)"))
                }
                FormHint(
                    String(
                        localized: """
                            Rotating makes a new key. Add its public key on every machine, then switch them to it \
                            here. Remove the old key from the machines last: until a machine has the new one, it still \
                            signs in with the old.
                            """
                    ),
                    .note
                )
            }
        }
    }
}
