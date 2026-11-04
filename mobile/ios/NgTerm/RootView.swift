import NgtermDesign
import NgtermFeatures
import SwiftUI

/// A main window: the app's frame over its own navigation, the first run over it while the app has no key, and the
/// questions the machines' connects wait on.
///
/// The session column holds a placeholder until the session view lands.
internal struct RootView: View {
    private let graph: AppGraph
    private let coreVersion: String
    @State private var scaffold: ScaffoldModel
    @State private var machines: MachinesModel
    @State private var onboarding: OnboardingModel

    internal var body: some View {
        AppScaffold(model: scaffold) {
            MachinesView(model: machines, selection: scaffold.machine) { machineId in
                scaffold.show(machine: machineId)
            }
        } sessions: { machineId in
            SessionsColumn(graph: graph, machineId: machineId, scaffold: scaffold, machines: machines)
                .id(machineId)
        } session: { _ in
            StatePanel.loading(String(localized: "Opening the session"))
        } settings: {
            SettingsPlaceholder(coreVersion: coreVersion)
        } onboarding: {
            OnboardingView(model: onboarding) {
                scaffold.isOnboardingPresented = false
            }
        }
        .connectPrompts(machines)
        .task {
            graph.start()
        }
    }

    internal init(graph: AppGraph, coreVersion: String, windows: SessionWindows) {
        self.graph = graph
        self.coreVersion = coreVersion
        _scaffold = State(initialValue: ScaffoldModel(windows: windows, needsOnboarding: graph.keys.keys.isEmpty))
        _machines = State(initialValue: MachinesModel(graph: graph))
        _onboarding = State(initialValue: OnboardingModel(keys: graph.keys))
    }
}
