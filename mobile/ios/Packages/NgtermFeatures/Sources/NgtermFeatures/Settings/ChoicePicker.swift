import NgtermDesign
import SwiftUI

/// A setting's values as the rows of its section, the chosen one checked in the tint: each its title over what it
/// means, with what `trailing` draws for it at the row's end (a palette's swatch).
internal struct ChoicePicker<Value: Hashable & Sendable, Trailing: View>: View {
    private let title: String
    private let choices: [SettingsChoice<Value>]
    private let selection: Binding<Value>
    private let trailing: (Value) -> Trailing

    internal var body: some View {
        Picker(selection: selection) {
            ForEach(choices, id: \.value) { choice in
                HStack(spacing: Spacing.medium) {
                    ChoiceLabel(choice.title, note: choice.note)
                    Spacer(minLength: 0)
                    trailing(choice.value)
                }
                .tag(choice.value)
            }
        } label: {
            Text(title)
        }
        .pickerStyle(.inline)
        .labelsHidden()
    }

    /// The choices of the setting `title`, bound to `selection`, each row ending in what `trailing` draws.
    internal init(
        _ title: String,
        choices: [SettingsChoice<Value>],
        selection: Binding<Value>,
        @ViewBuilder trailing: @escaping (Value) -> Trailing
    ) {
        self.title = title
        self.choices = choices
        self.selection = selection
        self.trailing = trailing
    }
}

extension ChoicePicker where Trailing == EmptyView {
    /// The choices of the setting `title`, bound to `selection`.
    internal init(_ title: String, choices: [SettingsChoice<Value>], selection: Binding<Value>) {
        self.init(title, choices: choices, selection: selection) { _ in
            EmptyView()
        }
    }
}
