import NgtermCore
import NgtermDesign
import SwiftUI

/// A choice ask's list (mobile/android's `ChoiceList`): a row per choice (its label, its subtitle under it), picked
/// with radio marks or checkboxes, then the free-form row when the ask has one (``AskOtherRow``), as one grouped
/// surface.
internal struct AskChoiceList: View {
    internal let list: RemoteAskChoices
    @Binding internal var draft: AskDraft

    internal var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(list.choices.enumerated()), id: \.element.id) { index, choice in
                if index > 0 {
                    Divider()
                        .padding(.leading, Spacing.hitTarget + Spacing.small)
                }
                row(choice, at: index)
            }
            if let other = list.other {
                if !list.choices.isEmpty {
                    Divider()
                        .padding(.leading, Spacing.hitTarget + Spacing.small)
                }
                AskOtherRow(placeholder: other, isMultiple: list.multiple, index: list.choices.count, draft: $draft)
            }
        }
        .padding(.horizontal, Spacing.small)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
    }

    private func row(_ choice: RemoteAskChoice, at index: Int) -> some View {
        let isChecked = draft.checked.contains(choice.id)
        return Button {
            draft = draft.toggle(index)
        } label: {
            HStack(spacing: Spacing.small) {
                AskChoiceMark(isMultiple: list.multiple, isChecked: isChecked, isEnabled: true)
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    Text(verbatim: choice.label)
                        .foregroundStyle(.primary)
                    if let subtitle = choice.subtitle, !AskText.isBlank(subtitle) {
                        Text(verbatim: subtitle)
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
        .accessibilityAddTraits(isChecked ? .isSelected : [])
        .accessibilityAddTraits(list.multiple ? .isToggle : [])
        .accessibilityIdentifier("ask-choice-\(choice.id)")
    }
}
