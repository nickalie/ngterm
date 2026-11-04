import SwiftUI

extension View {
    /// The app's prominent button: filled with the tint, its label in ``Palette/onTint``.
    ///
    /// The label reads in light, dark and Increase Contrast alike, where the system's white would not on the dark
    /// side's copper.
    public func prominentAction() -> some View {
        buttonStyle(.borderedProminent)
            .foregroundStyle(Palette.onTint.color)
            .frame(minHeight: Spacing.hitTarget)
    }

    /// The view under `cover` when there is one: hidden from VoiceOver, which reads the cover instead.
    public func covered(by cover: Cover?) -> some View {
        accessibilityHidden(cover != nil)
            .overlay {
                if let cover {
                    cover
                }
            }
    }

    /// Asks before what can't be undone: a dialog titled `title` with `message`, its destructive button, and Cancel.
    ///
    /// The destructive button repeats the action's words (`action.title`: "End session" is confirmed by "End
    /// session"). Safe actions don't ask.
    public func confirmingDestructive(
        _ title: String,
        isPresented: Binding<Bool>,
        message: String,
        action: RowAction
    ) -> some View {
        confirmationDialog(title, isPresented: isPresented, titleVisibility: .visible) {
            Button(action.title, role: .destructive, action: action.perform)
            Button(String(localized: "Cancel", bundle: .module), role: .cancel) {
                // Cancel only closes the dialog
            }
        } message: {
            Text(message)
        }
    }

    /// The app's look on a view tree: copper as the tint, and the bundled fonts registered.
    public func ngtermTheme() -> some View {
        Fonts.register()
        return tint(Palette.tint.color)
    }
}
