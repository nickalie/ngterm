import NgtermDesign
import SwiftUI

/// The machines column until the machines land: the empty state.
internal struct MachinesPlaceholder: View {
    internal var body: some View {
        StatePanel.empty(
            systemImage: "server.rack",
            title: String(localized: "No machines yet"),
            message: String(localized: "Add a machine that runs ngterm to see its sessions here."),
            action: nil
        )
    }
}
