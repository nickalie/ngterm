import Foundation
import NgtermDesign
import SwiftUI

/// A machine's sessions: the machine's name and address as the title, New session in the toolbar while it is
/// connected, and the sessions this app has attached, then the others, companions under their parents, with pull to
/// refresh.
///
/// Instead of the list it shows what the machine's link or listing calls for (``SessionsBody``), always with the one
/// action that moves on. End session confirms; a failure the list can't show is a banner at its end. A tap on a
/// session attaches it, then `onOpen` opens it. When the machine is removed meanwhile, `onGone` lets the screen go.
public struct SessionsView: View {
    // where the far side's setup is explained: the README's section on running ngterm on a server
    private static let serverSetup = URL(string: "https://github.com/nickalie/ngterm#a-server-without-a-display")

    private let model: SessionsModel
    private let machines: MachinesModel
    private let selection: String?
    private let onOpen: @MainActor (String) -> SessionPlacement
    private let onGone: @MainActor () -> Void
    @State private var confirming: SessionLine?
    @State private var isReviewing = false
    @State private var page: SafariPage?

    /// The body of the view.
    public var body: some View {
        if let state = model.state {
            screen(state)
        } else {
            StatePanel(
                title: String(localized: "Machine removed"),
                systemImage: "server.rack",
                message: String(localized: "Pick a machine to see its sessions.")
            )
            .onAppear {
                onGone()
            }
        }
    }

    // the machine's changed host key under review, while it is still changed
    private var reviewed: Binding<MachineItem?> {
        Binding {
            guard isReviewing else {
                return nil
            }
            return machines.items.first { item in
                guard item.id == model.machineId, case .hostKeyChanged = item.link else {
                    return false
                }
                return true
            }
        } set: { item in
            isReviewing = item != nil
        }
    }

    private var isNamingSession: Binding<Bool> {
        Binding {
            model.newSession != nil
        } set: { isShown in
            if !isShown {
                model.closeNewSession()
            }
        }
    }

    private var isConfirming: Binding<Bool> {
        Binding {
            confirming != nil
        } set: { isShown in
            if !isShown {
                confirming = nil
            }
        }
    }

    @ViewBuilder private var notice: some View {
        if let words = model.notice {
            Banner(.failed, message: words) {
                model.dismissNotice()
            }
            .padding(.horizontal, Spacing.large)
            .padding(.vertical, Spacing.small)
            .frame(maxWidth: .infinity)
            .background(Color(uiColor: .systemGroupedBackground))
        }
    }

    // the spinner takes the button's place so a second tap can't start another create
    @ViewBuilder private var newSessionButton: some View {
        if model.isCreating {
            ProgressView()
                .accessibilityLabel(String(localized: "Creating"))
                .accessibilityIdentifier("creating-session")
        } else {
            Menu {
                Button(String(localized: "New named session…"), systemImage: "character.cursor.ibeam") {
                    model.openNewSession()
                }
                .accessibilityIdentifier("new-named-session")
            } label: {
                Label(String(localized: "New session"), systemImage: "plus")
            } primaryAction: {
                model.createAndOpen(then: openCreated)
            }
            .accessibilityIdentifier("new-session")
        }
    }

    private var problemActions: SessionsProblem.Actions {
        SessionsProblem.Actions(
            connect: model.connect,
            refresh: refresh,
            review: { isReviewing = true },
            openReadme: {
                page = Self.serverSetup.map(SafariPage.init)
            }
        )
    }

    /// The sessions of the machine of `model`.
    ///
    /// `selection` is the session this window shows, if it is the machine's; opening one calls `onOpen` with its id
    /// once it is attached, which says where the session shows, and `onGone` runs when the machine is no longer saved.
    /// A changed host key is reviewed over `machines`.
    @preconcurrency
    public init(
        model: SessionsModel,
        machines: MachinesModel,
        selection: String?,
        onOpen: @escaping @MainActor (String) -> SessionPlacement,
        onGone: @escaping @MainActor () -> Void
    ) {
        self.model = model
        self.machines = machines
        self.selection = selection
        self.onOpen = onOpen
        self.onGone = onGone
    }

    private func screen(_ state: MachineSessions) -> some View {
        let isConnected = state.link == .connected
        return content(state)
            .navigationTitle(state.machine.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    SessionsTitle(machine: state.machine)
                }
                if hasNewSessionButton(state) {
                    ToolbarItem(placement: .primaryAction) {
                        newSessionButton
                    }
                }
            }
            .refreshable {
                if isConnected {
                    await model.refresh()
                } else {
                    model.connect()
                }
            }
            .safeAreaInset(edge: .bottom) {
                notice
            }
            .task {
                model.opened()
            }
            .onAppear {
                model.setShowing(true)
            }
            .onDisappear {
                model.setShowing(false)
            }
            .onChange(of: selection) { _, shown in
                model.windowShows(shown)
            }
            .sheet(isPresented: isNamingSession) {
                if let newSession = model.newSession {
                    NewSessionSheet(machine: state.machine.title, state: newSession) { name in
                        model.create(name: name, then: openCreated)
                    } onCancel: {
                        model.closeNewSession()
                    }
                    .ngtermTheme()
                }
            }
            .confirmingDestructive(
                String(localized: "End \(confirming?.session.name ?? "")?"),
                isPresented: isConfirming,
                message: String(
                    localized:
                        "It stops on \(state.machine.title), with everything running in it. This can't be undone."
                ),
                action: RowAction(String(localized: "End session")) {
                    if let line = confirming {
                        model.end(sessionId: line.id)
                    }
                    confirming = nil
                }
            )
            .sheet(item: $page) { shown in
                SafariView(url: shown.url) {
                    page = nil
                }
                .ignoresSafeArea()
            }
            .sheet(item: reviewed) { item in
                HostKeyChangedSheet(model: machines, item: item)
                    .ngtermTheme()
            }
    }

    private func hasNewSessionButton(_ state: MachineSessions) -> Bool {
        guard state.link == .connected else {
            return false
        }
        // the empty state shows the create's progress itself
        if model.isCreating, case .empty = SessionsBody(state) {
            return false
        }
        return true
    }

    @ViewBuilder
    private func content(_ state: MachineSessions) -> some View {
        let name = state.machine.title
        switch SessionsBody(state) {
        case .connecting:
            StatePanel.loading(String(localized: "Connecting to \(name)"))

        case .reading:
            StatePanel.loading(String(localized: "Reading the sessions on \(name)"))

        case .empty where model.isCreating:
            StatePanel.loading(String(localized: "Creating a session on \(name)"))

        case .empty(let failure):
            empty(state, failure: failure)

        case .sessions(let attached, let others, let failure):
            SessionsList(
                machine: name,
                attached: attached,
                others: others,
                banners: banners(state, failure: failure),
                selection: selection,
                onOpen: open,
                onDetach: model.detach,
                chatView: model.chatView(sessionId:)
            ) { line in
                confirming = line
            }

        case let problem:
            SessionsProblem(machine: name, problem: problem, actions: problemActions)
        }
    }

    // no sessions: the invitation to start one, under what the banners say
    private func empty(_ state: MachineSessions, failure: String?) -> some View {
        let name = state.machine.title
        return StatePanel.empty(
            systemImage: "terminal",
            title: String(localized: "No sessions on \(name)"),
            message: String(localized: "Start one here. It runs on \(name) and keeps running when you leave."),
            action: RowAction(String(localized: "New session")) {
                model.createAndOpen(then: openCreated)
            }
        )
        .background(Color(uiColor: .systemGroupedBackground))
        .safeAreaInset(edge: .top) {
            let banners = banners(state, failure: failure)
            if banners.isShown {
                banners
                    .padding(.horizontal, Spacing.large)
            }
        }
    }

    private func banners(_ state: MachineSessions, failure: String?) -> SessionsBanners {
        SessionsBanners(isOffline: state.isOffline, failure: failure, onRetry: refresh)
    }

    private func refresh() {
        Task {
            await model.refresh()
        }
    }

    private func open(_ sessionId: String) {
        model.attach(sessionId: sessionId) {
            onOpen(sessionId)
        }
    }

    private func openCreated(_ sessionId: String) {
        _ = onOpen(sessionId)
    }
}
