import SwiftUI

struct AskChoiceList: View {
    let choices: [Choice]

    var body: some View {
        ForEach(choices) { choice in
            Text(choice.label)
                .accessibilityIdentifier(Self.rowID(choice.id))
        }
    }

    /// rowID tags a caller item's id, so the scroll identity and the accessibility id agree.
    nonisolated static func rowID(_ id: String) -> String { "ask-choice-\(id)" }
}
