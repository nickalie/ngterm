import NgtermDesign
import SwiftUI

/// The chat's backlog on its way, with nothing to show yet (mobile/android's `ChatLoading`): a spinner and what is
/// under way, read from `machine`.
internal struct ChatLoading: View {
    internal let machine: String

    internal var body: some View {
        StatePanel.loading(String(localized: "Reading the conversation from \(machine)"))
    }
}
