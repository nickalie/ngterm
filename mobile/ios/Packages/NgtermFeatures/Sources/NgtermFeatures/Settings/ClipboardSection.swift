import NgtermDesign
import SwiftUI

/// The machines whose programs read the clipboard without asking, each with Ask again; or that every machine asks.
internal struct ClipboardSection: View {
    internal let model: SettingsModel

    internal var body: some View {
        let trusted = model.trusted
        Section {
            if trusted.isEmpty {
                Text("Every machine asks before its programs read your clipboard.")
                    .foregroundStyle(Palette.secondaryText.color)
            }
            ForEach(trusted) { machine in
                row(machine)
            }
        } header: {
            FormHint(String(localized: "Clipboard"), .header)
        } footer: {
            if !trusted.isEmpty {
                FormHint(String(localized: "These machines' programs read your clipboard without asking."), .note)
            }
        }
    }

    private func row(_ machine: TrustedMachine) -> some View {
        HStack(spacing: Spacing.small) {
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(machine.title)
                Text(verbatim: machine.address)
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
                    .accessibilityLabel(machine.spokenAddress)
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: 0)
            Button(String(localized: "Ask again")) {
                model.revokeClipboard(machine.id)
            }
            .buttonStyle(.borderless)
            .frame(minHeight: Spacing.hitTarget)
            .accessibilityLabel(Text("Ask again before \(machine.title) reads the clipboard"))
            .accessibilityIdentifier("clipboard-revoke-\(machine.title)")
        }
    }
}
