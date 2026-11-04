import SwiftUI

/// A text button without a container (a row's action, a banner's, a panel's quieter one), at least 44 pt tall.
///
/// The least size is the label's, with its whole frame as the shape a tap lands in: a frame around a `Button` adds
/// room, not touch.
internal struct QuietButton: View {
    internal let action: RowAction

    internal var body: some View {
        Button(action: action.perform) {
            Text(action.title)
                .frame(minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
    }
}
