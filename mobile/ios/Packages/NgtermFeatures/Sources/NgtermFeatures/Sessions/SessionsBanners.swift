import NgtermDesign
import SwiftUI

/// What is said over a machine's sessions or its empty state while they stay usable: the device is offline, and the
/// last refresh failed (with Retry).
internal struct SessionsBanners: View {
    internal let isOffline: Bool
    internal let failure: String?
    internal let onRetry: @MainActor () -> Void

    /// Whether there is anything to say.
    internal var isShown: Bool {
        isOffline || failure != nil
    }

    internal var body: some View {
        VStack(spacing: Spacing.small) {
            if isOffline {
                Banner(
                    .offline,
                    message: String(localized: "You're offline"),
                    note: String(localized: "Connected machines reconnect when the network is back.")
                )
            }
            if let failure {
                Banner(
                    .failed,
                    message: String(localized: "The list may be out of date"),
                    note: failure,
                    action: RowAction(String(localized: "Retry"), perform: onRetry)
                )
            }
        }
    }
}
