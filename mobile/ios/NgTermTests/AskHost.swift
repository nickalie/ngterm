import NgtermFeatures
import SwiftUI

/// A session view, as `SessionScreen` is: one per session shown, with its asks' sheet.
internal struct AskHost: View {
    internal let graph: AppGraph
    internal let shown: AskHostShown

    internal var body: some View {
        Color.clear
            .askPrompt(graph, session: shown.key)
            .id(shown.key)
    }
}
