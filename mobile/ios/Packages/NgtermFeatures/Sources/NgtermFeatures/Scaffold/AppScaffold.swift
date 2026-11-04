import NgtermDesign
import SwiftUI

/// The app's frame, by the horizontal size class: a `NavigationSplitView` of machines, a machine's sessions and the
/// session.
///
/// It collapses to a stack in a compact width (one pane full screen) and shows the columns side by side in a regular
/// one. Settings opens as a sheet from the machines' toolbar, and the onboarding covers everything on the first run.
/// The columns come from the caller, with the ``ScaffoldModel`` in their environment.
public struct AppScaffold<Machines: View, Sessions: View, Session: View, Settings: View, Onboarding: View>: View {
    @Bindable private var model: ScaffoldModel
    private let machines: Machines
    private let sessions: (String) -> Sessions
    private let session: (SessionWindowID) -> Session
    private let settings: Settings
    private let onboarding: Onboarding

    /// The body of the view.
    public var body: some View {
        NavigationSplitView(columnVisibility: $model.columnVisibility, preferredCompactColumn: $model.compactColumn) {
            machines
                .navigationTitle(String(localized: "Machines"))
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            model.isSettingsPresented = true
                        } label: {
                            Label(String(localized: "Settings"), systemImage: "gearshape")
                        }
                        .accessibilityIdentifier("settings")
                    }
                }
        } content: {
            if let machine = model.machine {
                sessions(machine)
            } else {
                StatePanel(
                    title: String(localized: "No machine selected"),
                    systemImage: "server.rack",
                    message: String(localized: "Pick a machine to see its sessions."),
                    // an iPad held upright has no room for the three columns, so the machines start hidden; where
                    // they show, the panel points at them and offers nothing
                    action: model.areMachinesHidden
                        ? RowAction(String(localized: "Show machines")) {
                            model.columnVisibility = .all
                        }
                        : nil
                )
            }
        } detail: {
            SessionDetail(model: model, session: session)
        }
        .environment(model)
        .sheet(isPresented: $model.isSettingsPresented) {
            settings
                .environment(model)
        }
        .fullScreenCover(isPresented: $model.isOnboardingPresented) {
            onboarding
                .environment(model)
        }
        .ngtermTheme()
    }

    /// The frame over `model`, with its columns (`machines`; a machine's `sessions`; a `session`), the `settings`
    /// sheet's content and the `onboarding` cover's.
    public init(
        model: ScaffoldModel,
        @ViewBuilder machines: () -> Machines,
        @ViewBuilder sessions: @escaping (String) -> Sessions,
        @ViewBuilder session: @escaping (SessionWindowID) -> Session,
        @ViewBuilder settings: () -> Settings,
        @ViewBuilder onboarding: () -> Onboarding
    ) {
        self.model = model
        self.machines = machines()
        self.sessions = sessions
        self.session = session
        self.settings = settings()
        self.onboarding = onboarding()
    }
}
