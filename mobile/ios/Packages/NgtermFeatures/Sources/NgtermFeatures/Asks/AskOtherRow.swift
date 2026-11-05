import NgtermDesign
import SwiftUI

/// A choice list's free-form row (mobile/android's `OtherRow`): its mark, then a field in place of a label.
///
/// Typing selects it, clearing it gives a radio list's pick back, and its mark takes a tap only while it holds text.
/// Its field is one line, refusing a control character as the answer field does.
internal struct AskOtherRow: View {
    internal let placeholder: String
    internal let isMultiple: Bool
    // the row's place after the choices, which the draft toggles it by
    internal let index: Int
    @Binding internal var draft: AskDraft

    internal var body: some View {
        HStack(spacing: Spacing.small) {
            Button {
                draft = draft.toggle(index)
            } label: {
                AskChoiceMark(isMultiple: isMultiple, isChecked: draft.isOtherSelected, isEnabled: draft.isOtherEnabled)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!draft.isOtherEnabled)
            .accessibilityLabel(label)
            .accessibilityAddTraits(draft.isOtherSelected ? .isSelected : [])
            .accessibilityIdentifier("ask-other-mark")
            TextField(label, text: text)
                .frame(minHeight: Spacing.hitTarget)
                .accessibilityLabel(label)
                .accessibilityIdentifier("ask-other")
        }
    }

    private var label: String {
        AskText.isBlank(placeholder) ? String(localized: "Something else") : placeholder
    }

    private var text: Binding<String> {
        Binding {
            draft.text
        } set: { typed in
            if !AskText.hasForbiddenControls(typed, multiline: false) {
                draft = draft.withText(typed)
            }
        }
    }
}
