import NgtermCore
import NgtermDesign
import SwiftUI

/// An ask's question, a sheet's page (mobile/android's `AskQuestion`): the attention glyph, its title as a heading and
/// its message, its form (a text field, or a choice list with its free-form row), then its buttons in the origin's
/// order, one per line (``AskButtons``).
///
/// A button answers with the draft (``AskDraft/reply(to:button:)``); a line field's Return presses the default button,
/// or the one answering button.
public struct AskQuestion: View {
    private static let width: Double = 520

    private let ask: RemoteAsk
    @Binding private var draft: AskDraft
    private let onAnswer: @MainActor (AskReply) -> Void

    /// The page.
    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                Image(systemName: StatusTone.attention.shape.symbolName)
                    .font(.title2)
                    .foregroundStyle(StatusTone.attention.color.color)
                    .accessibilityHidden(true)
                Text(verbatim: ask.title)
                    .font(.title2.bold())
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("ask-title")
                if let message = ask.message, !AskText.isBlank(message) {
                    Text(verbatim: message)
                        .foregroundStyle(Palette.secondaryText.color)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let input = ask.input {
                    AskAnswerField(input: input, draft: $draft, onSubmit: submit)
                }
                if let list = ask.choices {
                    AskChoiceList(list: list, draft: $draft)
                }
                AskButtons(ask: ask, onPress: press)
                    .padding(.top, Spacing.small)
            }
            .frame(maxWidth: Self.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }

    // what a line field's Return does: press the default button, or the only one that answers
    private var submit: (@MainActor () -> Void)? {
        let answering = ask.buttons.filter { $0.id != ask.cancelId }
        guard let buttonId = ask.defaultId ?? (answering.count == 1 ? answering.first?.id : nil) else {
            return nil
        }
        return { press(buttonId) }
    }

    /// The question of `ask`, its input in `draft`; a button runs `onAnswer` with its answer.
    @preconcurrency
    public init(ask: RemoteAsk, draft: Binding<AskDraft>, onAnswer: @escaping @MainActor (AskReply) -> Void) {
        self.ask = ask
        _draft = draft
        self.onAnswer = onAnswer
    }

    private func press(_ buttonId: String) {
        onAnswer(draft.reply(to: ask, button: buttonId))
    }
}
