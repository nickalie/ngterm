import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The Machines list: a row per machine with its link and its sessions' statuses, Add machine in the toolbar, the
/// form as a sheet, swipe to remove (with Undo), and each row's menu (Connect or Disconnect, Edit, Remove).
///
/// With no machines it invites the first; a banner says when the device is offline, and when a change failed. Each
/// machine's attached sessions sit under it with their statuses, the one shown marked, when the caller asks for them
/// (``attachedSessions(shown:onOpen:)``).
public struct MachinesView: View {
    // how long Undo stays on offer after a removal
    private static let undoSeconds = 10.0

    private let model: MachinesModel
    private let selection: String?
    private let onOpen: @MainActor (String) -> Void
    private var shownSession: SessionKey?
    private var onOpenSession: (@MainActor (SessionKey) -> Void)?
    @State private var form: FormTarget?
    @State private var review: String?

    /// The body of the view.
    public var body: some View {
        content
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        form = .add
                    } label: {
                        Label(String(localized: "Add machine"), systemImage: "plus")
                    }
                    .accessibilityIdentifier("add-machine")
                }
            }
            .safeAreaInset(edge: .bottom) {
                notices
            }
            .sheet(item: $form) { target in
                MachineFormView(model: model, target: target, showsProblems: false)
                    .ngtermTheme()
            }
            .sheet(item: reviewed) { item in
                HostKeyChangedSheet(model: model, item: item)
                    .ngtermTheme()
            }
            .task(id: model.removal) {
                guard model.removal != nil else {
                    return
                }
                try? await Task.sleep(for: .seconds(Self.undoSeconds))
                if !Task.isCancelled {
                    model.forgetRemoval()
                }
            }
    }

    @ViewBuilder private var content: some View {
        let items = model.items
        if items.isEmpty {
            StatePanel.empty(
                systemImage: "server.rack",
                title: String(localized: "No machines yet"),
                message: String(
                    localized:
                        "Add a server or desktop that runs ngterm, and drive its sessions from here over ssh."
                ),
                action: RowAction(String(localized: "Add machine")) {
                    form = .add
                }
            )
            .background(Color(uiColor: .systemGroupedBackground))
            .safeAreaInset(edge: .top) {
                if model.isOffline {
                    offlineBanner
                        .padding(.horizontal, Spacing.large)
                }
            }
        } else {
            list(items)
        }
    }

    // the machine whose sessions show; none while a session shows beside the list, which marks its row instead (a
    // selection the list draws unemphasized once the session has the focus would grey the machine's colored text)
    private var selected: Binding<String?> {
        Binding {
            shownSession != nil && onOpenSession != nil ? nil : selection
        } set: { machineId in
            if let machineId {
                onOpen(machineId)
            }
        }
    }

    // the machine whose changed host key is under review, while it is still changed
    private var reviewed: Binding<MachineItem?> {
        Binding {
            model.items.first { item in
                guard item.id == review, case .hostKeyChanged = item.link else {
                    return false
                }
                return true
            }
        } set: { item in
            review = item?.id
        }
    }

    private var offlineBanner: some View {
        Banner(
            .offline,
            message: String(localized: "You're offline"),
            note: String(localized: "Connected machines reconnect when the network is back.")
        )
    }

    // the failure and the removal's Undo, on the list's ground over the list's end
    @ViewBuilder private var notices: some View {
        let failure = model.failure
        let removal = model.removal
        if failure != nil || removal != nil {
            VStack(spacing: Spacing.small) {
                if let failure {
                    Banner(.failed, message: failure) {
                        model.dismissFailure()
                    }
                }
                if let removal {
                    Banner(
                        .idle,
                        message: String(localized: "Removed \(removal.machine.title)"),
                        action: RowAction(String(localized: "Undo")) {
                            model.undoRemove()
                        }
                    ) {
                        model.forgetRemoval()
                    }
                }
            }
            .padding(.horizontal, Spacing.large)
            .padding(.vertical, Spacing.small)
            .frame(maxWidth: .infinity)
            .background(Color(uiColor: .systemGroupedBackground))
        }
    }

    /// The list of `model`, with `selection` (the machine whose sessions show) marked; opening a machine calls
    /// `onOpen` with its id.
    @preconcurrency
    public init(model: MachinesModel, selection: String?, onOpen: @escaping @MainActor (String) -> Void) {
        self.model = model
        self.selection = selection
        self.onOpen = onOpen
    }

    /// The same list with each machine's attached sessions under it, `shown` (the session this window shows) marked.
    ///
    /// Opening one calls `onOpen`. With no `onOpen` the list is as it was.
    @preconcurrency
    public func attachedSessions(shown: SessionKey?, onOpen: (@MainActor (SessionKey) -> Void)?) -> Self {
        var copy = self
        copy.shownSession = shown
        copy.onOpenSession = onOpen
        return copy
    }

    private func list(_ items: [MachineItem]) -> some View {
        List(selection: selected) {
            if model.isOffline {
                Section {
                    offlineBanner
                        .listRowInsets(EdgeInsets())
                        .listRowBackground(Color.clear)
                }
            }
            Section {
                ForEach(items) { item in
                    MachineEntry(item: item) {
                        model.connect(machineId: item.id)
                    } onReview: {
                        review = item.id
                    }
                    .tag(item.id)
                    .swipeActions(edge: .trailing) {
                        Button(String(localized: "Remove"), systemImage: "trash", role: .destructive) {
                            model.remove(machineId: item.id)
                        }
                    }
                    .contextMenu {
                        menu(for: item)
                    }
                    if let onOpenSession {
                        ForEach(model.attached(of: item.id), id: \.key) { session in
                            SidebarSessionRow(session: session, isShown: session.key == shownSession) {
                                onOpenSession(session.key)
                            }
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
    }

    @ViewBuilder
    private func menu(for item: MachineItem) -> some View {
        if item.link.isUnderWay {
            Button(String(localized: "Disconnect"), systemImage: "bolt.horizontal.circle") {
                model.disconnect(machineId: item.id)
            }
        } else {
            Button(String(localized: "Connect"), systemImage: "bolt.horizontal") {
                model.connect(machineId: item.id)
            }
        }
        Button(String(localized: "Edit"), systemImage: "pencil") {
            form = .edit(item.id)
        }
        Button(String(localized: "Remove"), systemImage: "trash", role: .destructive) {
            model.remove(machineId: item.id)
        }
    }
}
