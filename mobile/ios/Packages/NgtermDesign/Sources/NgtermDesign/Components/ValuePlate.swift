import SwiftUI

/// A value compared character by character, under its label: a public key's line, a fingerprint, a command to run.
///
/// The value is mono, wraps, and can be selected; the plate's actions (Copy, Share) sit under it.
public struct ValuePlate<Actions: View>: View {
    private let label: String
    private let value: Text
    private let actions: Actions

    /// The body of the view.
    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            Text(label)
                .font(Typography.plateLabel)
                .foregroundStyle(Palette.secondaryText.color)
            value
                .font(Typography.plateValue)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: Spacing.large) {
                actions
            }
        }
        .padding(.horizontal, Spacing.medium)
        .padding(.top, Spacing.medium)
        .padding(.bottom, Spacing.xSmall)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
    }

    /// `value` (drawn as it is) under `label`, with its `actions`.
    public init(_ label: String, value: String, @ViewBuilder actions: () -> Actions) {
        self.label = label
        self.value = Text(verbatim: value)
        self.actions = actions()
    }

    /// `text` (a value with a link in it, say) under `label`, with its `actions`.
    public init(_ label: String, text: Text, @ViewBuilder actions: () -> Actions) {
        self.label = label
        value = text
        self.actions = actions()
    }
}

extension ValuePlate where Actions == EmptyView {
    /// `value` under `label`, with no actions.
    public init(_ label: String, value: String) {
        self.init(label, value: value) {
            EmptyView()
        }
    }
}
