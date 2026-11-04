import SwiftUI

/// A text button without a container (a row's action, a banner's, a panel's quieter one), at least 44 pt tall.
///
/// The least size is the label's, with its whole frame as the shape a tap lands in: a frame around a `Button` adds
/// room, not touch.
internal struct QuietButton: View {
    internal let action: RowAction

    internal var body: some View {
        Button(action: action.perform) {
            // the tint, or the row's own foreground in a selected row, whose ground is the tint
            Text(action.title)
                .rowForeground(Palette.tint.color)
                .frame(minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
    }
}
