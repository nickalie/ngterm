import SwiftUI

/// The chat's grounds, the system's: the chat on the plain ground, its plates and bubbles one level up.
internal enum ChatGrounds {
    /// The chat's own ground.
    internal static let chat = Color(uiColor: .systemBackground)
    /// A plate: output, code, a diff.
    internal static let plate = Color(uiColor: .secondarySystemBackground)
    /// A prompt's bubble, the composer's pill, a card, the `/` menu.
    internal static let raised = Color(uiColor: .secondarySystemBackground)
    /// The `/` menu's highlighted row.
    internal static let highlighted = Color(uiColor: .tertiarySystemFill)
}
