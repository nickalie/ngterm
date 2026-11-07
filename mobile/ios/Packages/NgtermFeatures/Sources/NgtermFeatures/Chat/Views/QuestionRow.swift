import NgtermCore
import SwiftUI

/// An `AskUserQuestion` in the conversation (mobile/android's `QuestionRow`): its first question and its status (Needs
/// you while it waits, its card pinned at the chat's foot); a tap shows every question and the answer the agent got.
internal struct QuestionRow: View {
    internal let question: ChatQuestion
    internal let isOpen: Bool
    internal let onToggle: @MainActor () -> Void

    internal var body: some View {
        let first = question.questions.first?.question ?? ""
        ChatLine(
            symbol: ChatStyle.chatSymbol,
            glyphLabel: String(localized: "Question"),
            title: AskText.isBlank(first) ? String(localized: "Question") : first,
            titleFont: .subheadline,
            status: .question(question.status),
            toggle: ChatToggle(
                isExpanded: isOpen,
                show: String(localized: "Shows the answer"),
                hide: String(localized: "Hides the answer"),
                onToggle: onToggle
            )
        ) {
            ForEach(Array(question.questions.enumerated()), id: \.offset) { _, item in
                Text(item.question)
                    .font(.subheadline)
                    .textSelection(.enabled)
            }
            if let output = question.output, !AskText.isBlank(output) {
                OutputPlate(text: output)
            }
        }
    }
}
