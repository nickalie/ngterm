import NgtermFeatures
import SwiftUI

/// The sessions column of a main window: one machine's sessions over a model of its own, which lives as long as the
/// column shows that machine (the caller gives it the machine's id as its identity).
///
/// Opening a session shows it in this window, or brings its own window forward; a machine removed meanwhile leaves the
/// window.
internal struct SessionsColumn: View {
    @Environment(\.openWindow)
    private var openWindow

    private let scaffold: ScaffoldModel
    private let machines: MachinesModel
    @State private var model: SessionsModel

    internal var body: some View {
        let machineId = model.machineId
        SessionsView(
            model: model,
            machines: machines,
            selection: scaffold.session.flatMap { $0.machineID == machineId ? $0.sessionID : nil }
        ) { sessionId in
            let session = SessionWindowID(machineID: machineId, sessionID: sessionId)
            if scaffold.open(session) == .ownWindow {
                openWindow(value: session)
            }
        } onGone: {
            scaffold.forget(machine: machineId)
        }
    }

    internal init(graph: AppGraph, machineId: String, scaffold: ScaffoldModel, machines: MachinesModel) {
        self.scaffold = scaffold
        self.machines = machines
        _model = State(initialValue: SessionsModel(graph: graph, machineId: machineId, windows: scaffold.windows))
    }
}
