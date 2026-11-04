import Foundation
import NgtermDesign
import SwiftUI
import UniformTypeIdentifiers

/// Importing a key the user already has: pasted, or read from a file they choose, and its passphrase.
///
/// What was wrong with the last attempt shows under the field it is about, and the key (typed, or the chosen file's)
/// stays for another try. The key and the passphrase live only in this screen's state and go when it does; a chosen
/// file's bytes are zeroed when they are replaced or the screen goes.
internal struct ImportKeyView: View {
    // a private key file is a few kilobytes; anything far bigger is not one
    private static let maxKeyBytes = 65_536
    // the key field shows a few lines of a key: three hit targets tall
    private static let keyFieldLines = 3.0

    internal let model: OnboardingModel
    @State private var keyText = ""
    @State private var passphrase = ""
    @State private var isPassphraseShown = false
    @State private var chosen: Data?
    @State private var isChoosingFile = false

    internal var body: some View {
        let (isBusy, problem) = state
        // a scroll view, not a Form: every part stays laid out (a list lets go of rows pushed off screen), so each
        // grows with the text size
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.xLarge) {
                Text(
                    """
                    Paste the private key, or choose its file. OpenSSH and PEM keys work; \
                    ngterm keeps it in this device's Keychain.
                    """
                )
                .foregroundStyle(Palette.secondaryText.color)
                keyGroup(problem: problem?.isAboutPassphrase == false ? problem : nil)
                passphraseGroup(problem: problem?.isAboutPassphrase == true ? problem : nil)
                BusyButton(title: String(localized: "Import key"), isBusy: isBusy) {
                    importKey()
                }
                .accessibilityIdentifier("import-key-action")
            }
            .frame(maxWidth: OnboardingLayout.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Color(uiColor: .systemGroupedBackground))
        .disabled(isBusy)
        .navigationTitle(String(localized: "Import a key"))
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(isPresented: $isChoosingFile, allowedContentTypes: [.item]) { result in
            chose(result)
        }
        .onDisappear {
            replaceChosen(with: nil)
        }
    }

    private var state: (isBusy: Bool, problem: ImportProblem?) {
        if case .importing(let isBusy, let problem) = model.step {
            return (isBusy, problem)
        }
        return (false, nil)
    }

    private func keyGroup(problem: ImportProblem?) -> some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            FormHint(String(localized: "Private key"), .header)
            TextEditor(text: $keyText)
                .font(Typography.plateValue)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .scrollContentBackground(.hidden)
                .frame(minHeight: Spacing.hitTarget * Self.keyFieldLines)
                .padding(Spacing.small)
                .background(
                    Color(uiColor: .secondarySystemGroupedBackground),
                    in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
                )
                .accessibilityLabel(String(localized: "Private key"))
                .accessibilityIdentifier("private-key")
                .onChange(of: keyText) { _, text in
                    // typing uses the text instead of the chosen file's key
                    if !text.isEmpty {
                        replaceChosen(with: nil)
                    }
                }
            if let problem {
                InlineProblem(problem.message)
            } else if chosen != nil {
                FormHint(
                    String(localized: "The key from the chosen file is ready. Typing here uses the text instead."),
                    .note
                )
            }
            Button {
                isChoosingFile = true
            } label: {
                Text("Choose file")
                    .frame(minHeight: Spacing.hitTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
        }
    }

    private func passphraseGroup(problem: ImportProblem?) -> some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            FormHint(String(localized: "Passphrase"), .header)
            HStack {
                Group {
                    if isPassphraseShown {
                        TextField(String(localized: "Passphrase"), text: $passphrase)
                    } else {
                        SecureField(String(localized: "Passphrase"), text: $passphrase)
                    }
                }
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textContentType(.password)
                .accessibilityIdentifier("passphrase")
                Button(isPassphraseShown ? String(localized: "Hide") : String(localized: "Show")) {
                    isPassphraseShown.toggle()
                }
                .buttonStyle(.borderless)
                .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                .accessibilityLabel(
                    isPassphraseShown ? String(localized: "Hide passphrase") : String(localized: "Show passphrase")
                )
            }
            .padding(.horizontal, Spacing.medium)
            .background(
                Color(uiColor: .secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
            )
            if let problem {
                InlineProblem(problem.message)
            } else {
                FormHint(String(localized: "Only if the key has one."), .note)
            }
        }
    }

    // imports the chosen file's key (a copy of its own: the import zeroes what it is handed), or the text
    private func importKey() {
        let file = chosen
        let text = keyText
        let typed = passphrase
        Task {
            var bytes = file.map { Data([UInt8]($0)) } ?? Data(text.utf8)
            await model.importKey(privateKey: &bytes, passphrase: typed)
        }
    }

    private func chose(_ result: Result<URL, any Error>) {
        guard case .success(let url) = result else {
            return
        }
        let isScoped = url.startAccessingSecurityScopedResource()
        defer {
            if isScoped {
                url.stopAccessingSecurityScopedResource()
            }
        }
        guard let bytes = try? Data(contentsOf: url, options: .mappedIfSafe), bytes.count <= Self.maxKeyBytes else {
            model.unreadable()
            return
        }
        keyText = ""
        replaceChosen(with: Data([UInt8](bytes)))
        importKey()
    }

    // keeps `bytes` as the chosen key, zeroing the one it replaces
    private func replaceChosen(with bytes: Data?) {
        if var old = chosen {
            chosen = nil
            old.resetBytes(in: old.startIndex..<old.endIndex)
        }
        chosen = bytes
    }
}
