import NgtermDesign
import SwiftUI

/// Settings (mobile/android's `SettingsRoute`), a sheet with a `Form`: the terminal (a live sample over its text size,
/// font and colors, and keeping the screen on), the appearance, the extra keys, the notifications, the machines
/// trusted with the clipboard, the keys, and the loaded core's version; Done closes it.
///
/// It pushes a key's public half (with Use for every machine under it) and the import. A key just made or brought
/// opens its public half in place of where Settings was. Whether the app may notify is read again each time the app
/// comes back to the foreground, so a change in the system's settings shows. A change the stores couldn't keep shows
/// as a banner at the sheet's foot until it is dismissed.
public struct SettingsView: View {
    @Environment(\.dismiss)
    private var dismiss
    @Environment(\.scenePhase)
    private var scenePhase

    private let coreVersion: String
    @State private var model: SettingsModel
    @State private var path: [SettingsPage] = []

    /// The body of the view.
    public var body: some View {
        NavigationStack(path: $path) {
            Form {
                TerminalSection(model: model)
                ChoiceSections(model: model)
                NotificationsSection(model: model)
                ClipboardSection(model: model)
                KeysSection(model: model) { keyId in
                    path.append(.publicKey(keyId))
                } onImport: {
                    model.startImport()
                    path.append(.importKey)
                }
                AboutSection(coreVersion: coreVersion)
            }
            .accessibilityIdentifier("settings-form")
            .navigationTitle(String(localized: "Settings"))
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Done")) {
                        dismiss()
                    }
                    .accessibilityIdentifier("settings-done")
                }
            }
            .navigationDestination(for: SettingsPage.self) { page in
                destination(page)
            }
        }
        .ngtermTheme()
        .safeAreaInset(edge: .bottom) {
            if let failure = model.failure {
                Banner(.failed, message: failure) {
                    model.dismissFailure()
                }
                .padding(Spacing.large)
            }
        }
        .task {
            await model.readPermission()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                Task {
                    await model.readPermission()
                }
            }
        }
        .onChange(of: model.newKey) { _, keyId in
            if let keyId {
                path = [.publicKey(keyId)]
                model.newKeyShown()
            }
        }
    }

    // the import over Settings' model
    private var importWork: ImportKeyWork {
        let settings = model
        return ImportKeyWork {
            (settings.work.isImporting, settings.work.importProblem)
        } importKey: { bytes, passphrase in
            await settings.importKey(privateKey: &bytes, passphrase: passphrase)
        } unreadable: {
            settings.unreadable()
        }
    }

    /// Settings over `graph`, saying `coreVersion` is the loaded core's.
    public init(graph: AppGraph, coreVersion: String) {
        self.init(model: SettingsModel(graph: graph), coreVersion: coreVersion)
    }

    /// Settings over `model` (a test's, read ahead), saying `coreVersion` is the loaded core's.
    internal init(model: SettingsModel, coreVersion: String) {
        self.coreVersion = coreVersion
        _model = State(initialValue: model)
    }

    @ViewBuilder
    private func destination(_ page: SettingsPage) -> some View {
        switch page {
        case .importKey:
            ImportKeyView(work: importWork)

        case .publicKey(let keyId):
            if let item = model.keys.first(where: { $0.id == keyId }) {
                PublicKeyView(key: item.key) {
                    SwitchMachines(item: item) {
                        model.useEverywhere(keyId)
                    }
                }
            } else {
                // the key is gone: there is nothing to show
                Color(uiColor: .systemGroupedBackground)
            }
        }
    }
}
