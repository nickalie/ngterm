import SwiftUI

/// A session's Duplicate session item: its machine copies the session, and the copy opens here in control.
internal struct DuplicateButton: View {
    internal let action: @MainActor () -> Void

    internal var body: some View {
        Button(String(localized: "Duplicate session"), systemImage: "plus.square.on.square", action: action)
    }
}
