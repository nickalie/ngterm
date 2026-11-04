import NgtermDesign
import NgtermFeatures
import SwiftUI

/// A main window: the app's frame over its own navigation.
///
/// The columns hold placeholders until their features land.
internal struct RootView: View {
    private let coreVersion: String
    @State private var model: ScaffoldModel

    internal var body: some View {
        AppScaffold(model: model) {
            MachinesPlaceholder()
        } sessions: { _ in
            StatePanel.loading(String(localized: "Reading the machine's sessions"))
        } session: { _ in
            StatePanel.loading(String(localized: "Opening the session"))
        } settings: {
            SettingsPlaceholder(coreVersion: coreVersion)
        } onboarding: {
            EmptyView()
        }
    }

    internal init(coreVersion: String, windows: SessionWindows) {
        self.coreVersion = coreVersion
        _model = State(initialValue: ScaffoldModel(windows: windows))
    }
}
