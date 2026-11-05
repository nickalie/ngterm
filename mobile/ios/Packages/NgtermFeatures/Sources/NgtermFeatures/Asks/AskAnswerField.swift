import NgtermCore
import NgtermDesign
import SwiftUI

/// An input ask's field (mobile/android's `AnswerField`): one line, or an editing area that grows from three lines to
/// eight, starting from the origin's text, with its hint ("Your answer" when the origin gives none).
///
/// An edit that would add a control character the field can't hold is refused whole, as desktop refuses it. A line
/// field's Return runs `onSubmit`.
internal struct AskAnswerField: View {
    private static let multilineLines = 3...8

    internal let input: RemoteAskInput
    @Binding internal var draft: AskDraft
    internal let onSubmit: (@MainActor () -> Void)?

    internal var body: some View {
        TextField(hint, text: text, axis: input.multiline ? .vertical : .horizontal)
            .lineLimit(input.multiline ? Self.multilineLines : 1...1)
            .submitLabel(.done)
            .onSubmit {
                onSubmit?()
            }
            .padding(.horizontal, Spacing.medium)
            .padding(.vertical, Spacing.small)
            .frame(minHeight: Spacing.hitTarget)
            .background(
                Color(uiColor: .secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
            )
            .accessibilityLabel(hint)
            .accessibilityIdentifier("ask-answer")
    }

    private var hint: String {
        input.placeholder.flatMap { AskText.isBlank($0) ? nil : $0 } ?? String(localized: "Your answer")
    }

    // a refused edit leaves the draft as it was
    private var text: Binding<String> {
        Binding {
            draft.text
        } set: { typed in
            if !AskText.hasForbiddenControls(typed, multiline: input.multiline) {
                draft = draft.withText(typed)
            }
        }
    }
}
