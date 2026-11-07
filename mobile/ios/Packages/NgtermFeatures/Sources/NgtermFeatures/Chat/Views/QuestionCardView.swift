import NgtermCore
import NgtermDesign
import SwiftUI

/// Claude's `AskUserQuestion`, waiting (mobile/android's `QuestionCardView`): "Claude Code asks", then each question
/// (its header over it, radio rows for a single pick, checkbox rows for several, each option's label over its
/// description, then the "Type an answer" row), and **Send answers** once every question has an answer.
///
/// The draft lives with the card's question, and goes with it. Its answer field's focus follows the selection as the
/// composer's does (`selection`): the chat the selection leaves lets it go.
internal struct QuestionCardView: View {
    /// Whether the card's chat is the selected pane, and what to tell when an answer field takes or lets go the focus.
    internal struct Selection {
        internal let isSelected: Bool
        internal let onFieldFocus: @MainActor (Bool) -> Void
    }

    // the card itself, its draft its own: a card for another question is another view, with a draft of its own
    private struct Card: View {
        let card: OpenQuestion
        let agent: String?
        let isBusy: Bool
        let maxHeight: CGFloat
        let onAnswer: @MainActor ([QuestionAnswer]) -> Void
        let selection: Selection
        @State private var draft: QuestionDraft
        // which question's answer field has the focus
        @FocusState private var answerField: Int?

        var body: some View {
            let answers = draft.answers(for: card.question)
            NeedsYouCard(title: String(localized: "\(ChatStyle.agentName(agent)) asks"), maxHeight: maxHeight) {
                ForEach(Array(card.question.questions.enumerated()), id: \.offset) { index, item in
                    question(item, index)
                }
                CardButtons(
                    labels: [String(localized: "Send answers")],
                    isBusy: isBusy,
                    isEnabled: !answers.isEmpty
                ) { _ in
                    if !answers.isEmpty {
                        onAnswer(answers)
                    }
                }
            }
            .onChange(of: selection.isSelected) { _, selected in
                if !selected {
                    answerField = nil
                }
            }
            .onChange(of: answerField) { _, field in
                selection.onFieldFocus(field != nil)
            }
        }

        init(
            card: OpenQuestion,
            agent: String?,
            isBusy: Bool,
            maxHeight: CGFloat,
            onAnswer: @escaping @MainActor ([QuestionAnswer]) -> Void,
            selection: Selection
        ) {
            self.card = card
            self.agent = agent
            self.isBusy = isBusy
            self.maxHeight = maxHeight
            self.onAnswer = onAnswer
            self.selection = selection
            _draft = State(initialValue: QuestionDraft(for: card.question))
        }

        private func question(_ item: ChatQuestionItem, _ index: Int) -> some View {
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                if let header = item.header, !AskText.isBlank(header) {
                    Text(verbatim: header)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Palette.secondaryText.color)
                }
                Text(verbatim: item.question)
                    .font(.subheadline.weight(.semibold))
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(item.options.enumerated()), id: \.offset) { _, option in
                        optionRow(item, index, option)
                    }
                    typedRow(item, index)
                }
            }
        }

        private func optionRow(_ item: ChatQuestionItem, _ index: Int, _ option: ChatQuestionOption) -> some View {
            let isChecked = draft.picked[index].contains(option.label)
            return Button {
                draft = draft.toggling(index, item, label: option.label)
            } label: {
                HStack(spacing: Spacing.small) {
                    AskChoiceMark(isMultiple: item.multiSelect, isChecked: isChecked, isEnabled: true)
                    VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                        Text(verbatim: option.label)
                            .foregroundStyle(.primary)
                        if let description = option.description, !AskText.isBlank(description) {
                            Text(verbatim: description)
                                .font(.footnote)
                                .foregroundStyle(Palette.secondaryText.color)
                        }
                    }
                    .padding(.vertical, Spacing.small)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(isBusy)
            .accessibilityAddTraits(isChecked ? .isSelected : [])
            .accessibilityAddTraits(item.multiSelect ? .isToggle : [])
        }

        // the free-form row: its mark, then the field in place of a label; typing picks it, clearing it lets it go, and
        // its mark takes a tap only while it holds text
        private func typedRow(_ item: ChatQuestionItem, _ index: Int) -> some View {
            let label = String(localized: "Type an answer")
            let hasText = !AskText.isBlank(draft.other[index])
            return HStack(spacing: Spacing.small) {
                Button {
                    draft = draft.togglingOther(index, item)
                } label: {
                    AskChoiceMark(isMultiple: item.multiSelect, isChecked: draft.otherPicked[index], isEnabled: hasText)
                }
                .buttonStyle(.plain)
                .disabled(!hasText || isBusy)
                .accessibilityLabel(label)
                .accessibilityAddTraits(draft.otherPicked[index] ? .isSelected : [])
                TextField(
                    label,
                    text: Binding {
                        draft.other[index]
                    } set: { text in
                        draft = draft.typing(index, item, text: text.replacing("\n", with: " "))
                    }
                )
                .textFieldStyle(.roundedBorder)
                .focused($answerField, equals: index)
                .disabled(isBusy)
            }
            .frame(minHeight: Spacing.hitTarget + Spacing.small)
        }
    }

    internal let card: OpenQuestion
    internal let agent: String?
    internal let isBusy: Bool
    internal let maxHeight: CGFloat
    internal let onAnswer: @MainActor ([QuestionAnswer]) -> Void
    internal let selection: Selection

    internal var body: some View {
        Card(
            card: card,
            agent: agent,
            isBusy: isBusy,
            maxHeight: maxHeight,
            onAnswer: onAnswer,
            selection: selection
        )
        .id(card.id)
    }
}
