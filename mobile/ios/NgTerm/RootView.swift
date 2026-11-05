import NgtermFeatures
import SwiftUI

/// A main window: the app's frame over its own navigation, the first run over it while the app has no key, the
/// questions the machines' connects wait on, and the programs' clipboard reads.
///
/// Beside a session (a regular width), the machines' column lists each machine's attached sessions too; a session
/// opened there, or by the keyboard's moves, shows here, or brings its own window forward.
internal struct RootView: View {
    @Environment(\.openWindow)
    private var openWindow
    @Environment(\.supportsMultipleWindows)
    private var supportsMultipleWindows
    @Environment(\.horizontalSizeClass)
    private var sizeClass

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
            // beside a session only: a compact width's machines are a page of their own
            .attachedSessions(shown: scaffold.session?.key, onOpen: sizeClass == .regular ? open : nil)
        } sessions: { machineId in
            SessionsColumn(graph: graph, machineId: machineId, scaffold: scaffold, machines: machines)
                .id(machineId)
        } session: { session in
            SessionScreen(graph: graph, session: session.key, actions: actions(for: session))
                .id(session)
        } settings: {
            SettingsPlaceholder(coreVersion: coreVersion)
        } onboarding: {
            OnboardingView(model: onboarding) {
                scaffold.isOnboardingPresented = false
            }
        }
        .connectPrompts(machines)
        .clipboardReadPrompt(graph)
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

    // what the session view of `session` asks of this window
    private func actions(for session: SessionWindowID) -> SessionActions {
        SessionActions(
            close: {
                if scaffold.session == session {
                    scaffold.close()
                }
            },
            open: open,
            openInNewWindow: supportsMultipleWindows
                ? { @MainActor @Sendable in openWindow(value: session) }
                : nil
        )
    }

    // shows the session `key` here, or brings its own window forward
    private func open(_ key: SessionKey) {
        let session = SessionWindowID(key)
        if scaffold.open(session) == .ownWindow {
            openWindow(value: session)
        }
    }
}
