import NgtermFeatures
import SwiftUI

/// A main window: the app's frame over its own navigation, and the questions over it, in Settings' appearance.
///
/// The first run shows over it while the app has no key, then the questions the machines' connects wait on, the
/// programs' clipboard reads, and the question about notifying as the first session is attached. A notification's tap
/// brings its machine, session and pane here.
///
/// The machines' list shows each machine's attached sessions under it: beside a session in a regular width, and as a
/// page of its own in a compact one, where it is the phone's view of every attached session (mobile/android's
/// Sessions destination). A session opened there, or by the keyboard's moves, shows here, or brings its own window
/// forward.
internal struct RootView: View {
    @Environment(\.openWindow)
    private var openWindow
    @Environment(\.supportsMultipleWindows)
    private var supportsMultipleWindows

    private let graph: AppGraph
    private let coreVersion: String
    @State private var scaffold: ScaffoldModel
    @State private var machines: MachinesModel
    @State private var onboarding: OnboardingModel
    // a notification tap's session, attached once its machine connects and lists it
    @State private var tapAttach: Task<Void, Never>?

    internal var body: some View {
        AppScaffold(model: scaffold) {
            MachinesView(model: machines, selection: scaffold.machine) { machineId in
                scaffold.show(machine: machineId)
            }
            .attachedSessions(shown: scaffold.session?.key, onOpen: open)
        } sessions: { machineId in
            SessionsColumn(graph: graph, machineId: machineId, scaffold: scaffold, machines: machines)
                .id(machineId)
        } session: { session in
            SessionScreen(graph: graph, session: session.key, actions: actions(for: session))
                .id(session)
        } settings: {
            SettingsView(graph: graph, coreVersion: coreVersion)
        } onboarding: {
            OnboardingView(model: onboarding) {
                scaffold.isOnboardingPresented = false
            }
        }
        .connectPrompts(machines)
        .clipboardReadPrompt(graph)
        .notificationPermissionPrompt(graph)
        .appAppearance(graph)
        .onChange(of: graph.openRequests.pending, initial: true) { _, target in
            if let target {
                go(to: target)
            }
        }
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

    // goes where a notification's tap asked: the machine, then its session with the pane. A session not attached (the
    // app started afresh for the tap, as after a background refresh's notice) is attached once the machine connects
    // and lists it, while the window still shows the machine; a connect that needs the user, or a session gone, leaves
    // the machine showing
    private func go(to target: OpenTarget) {
        graph.openRequests.consume(target)
        tapAttach?.cancel()
        tapAttach = nil
        scaffold.show(machine: target.machineId)
        guard let sessionId = target.sessionId else {
            return
        }
        let key = SessionKey(machineId: target.machineId, sessionId: sessionId)
        if graph.connections.attached[target.machineId]?.contains(sessionId) == true {
            open(key, pane: target.paneId)
            return
        }
        // the connects start with the graph, which may not have started yet
        graph.start()
        tapAttach = Task {
            let isAttached = await graph.connections.attachWhenListed(machineId: key.machineId, sessionId: sessionId) {
                showsOnly(machine: key.machineId)
            }
            if isAttached, !Task.isCancelled, showsOnly(machine: key.machineId) {
                open(key, pane: target.paneId)
            }
        }
    }

    // whether this window shows the sessions of `machine` and no session, as a tap leaves it
    private func showsOnly(machine: String) -> Bool {
        scaffold.machine == machine && scaffold.session == nil && scaffold.compactColumn == .content
    }

    // shows the session `key` with its pane `paneId` selected, when given
    private func open(_ key: SessionKey, pane paneId: String?) {
        if let paneId {
            graph.panes.select(paneId, of: key)
        }
        open(key)
    }

    // shows the session `key` here, or brings its own window forward
    private func open(_ key: SessionKey) {
        let session = SessionWindowID(key)
        if scaffold.open(session) == .ownWindow {
            openWindow(value: session)
        }
    }
}
