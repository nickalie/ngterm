import NgtermDesign
import NgtermPlatform
import SwiftUI

/// A key's public half and how to put it on a machine: the three steps, the `authorized_keys` line with Copy and
/// Share, its fingerprint, and the one command that appends it.
///
/// The first run ends here with Done (`onDone`); from the machine form it is a pushed screen, with no Done, and from
/// Settings one with what to do next under it (its `footer`).
public struct PublicKeyView<Footer: View>: View {
    private let key: AppKey
    private let onDone: (@MainActor () -> Void)?
    private let footer: Footer

    /// The body of the view.
    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                if onDone != nil {
                    Text("Add this key to your machines")
                        .font(.largeTitle.bold())
                        .accessibilityAddTraits(.isHeader)
                }
                steps
                ValuePlate(String(localized: "Public key"), value: key.authorizedKey) {
                    CopyButton(key.authorizedKey)
                    ShareLink(item: key.authorizedKey, subject: Text("ngterm public key")) {
                        Label(String(localized: "Share"), systemImage: "square.and.arrow.up")
                            .font(Typography.detail.weight(.semibold))
                            .frame(minHeight: Spacing.hitTarget)
                    }
                }
                Text("Fingerprint \(Text(verbatim: key.fingerprint).font(Typography.address))")
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
                    .textSelection(.enabled)
                let command = Self.appendCommand(for: key.authorizedKey)
                ValuePlate(String(localized: "Or run this on the machine"), value: command) {
                    CopyButton(command)
                }
                footer
                if let onDone {
                    Button {
                        onDone()
                    } label: {
                        Text("Done")
                            .frame(maxWidth: .infinity)
                    }
                    .prominentAction()
                    .padding(.top, Spacing.small)
                    .accessibilityIdentifier("key-done")
                }
            }
            .frame(maxWidth: OnboardingLayout.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.xxLarge)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
        .accessibilityIdentifier("public-key-page")
        .navigationTitle(onDone == nil ? String(localized: "Public key") : "")
        .navigationBarTitleDisplayMode(.inline)
    }

    // a real sequence, so numbered: what to do, in order
    private var steps: some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            let lines = [
                String(localized: "Copy the public key below."),
                String(
                    localized: """
                        On each machine, add it as a line of ~/.ssh/authorized_keys for the user you sign in as.
                        """
                ),
                String(localized: "Add the machine here."),
            ]
            ForEach(Array(lines.enumerated()), id: \.offset) { index, step in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.medium) {
                    Text(verbatim: "\(index + 1)")
                        .font(.headline)
                        .foregroundStyle(Palette.tint.color)
                        .accessibilityHidden(true)
                    Text(step)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Text("Step \(index + 1): \(step)"))
            }
        }
    }

    /// The public half of `key`, pushed, with `footer` under it.
    public init(key: AppKey, @ViewBuilder footer: () -> Footer) {
        self.key = key
        onDone = nil
        self.footer = footer()
    }

    /// The shell command that adds `line` to the signing-in user's `authorized_keys`, making `~/.ssh` as sshd wants
    /// it.
    public static func appendCommand(for line: String) -> String {
        let quoted = line.replacing("'", with: "'\\''")
        return "mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '\(quoted)' >> ~/.ssh/authorized_keys"
    }
}

extension PublicKeyView where Footer == EmptyView {
    /// The public half of `key`, with `onDone` ending the first run (none when the screen is pushed).
    @preconcurrency
    public init(key: AppKey, onDone: (@MainActor () -> Void)? = nil) {
        self.key = key
        self.onDone = onDone
        footer = EmptyView()
    }
}
