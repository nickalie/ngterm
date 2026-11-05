import NgtermPlatform
import SwiftUI
import UIKit

/// Settings' appearance on a window: the system's, or light or dark whatever the system's.
///
/// It sets the window's own style (`overrideUserInterfaceStyle`), so everything in the window follows, the sheets
/// over it and the terminal's palette (the theme's: Copper when dark, Drafting when light) included.
internal struct AppAppearance: UIViewRepresentable {
    internal let theme: ThemeChoice

    internal func makeUIView(context _: Context) -> WindowStyleView {
        WindowStyleView(frame: .zero)
    }

    internal func updateUIView(_ view: WindowStyleView, context _: Context) {
        switch theme {
        case .system:
            view.style = .unspecified

        case .light:
            view.style = .light

        case .dark:
            view.style = .dark
        }
    }
}

extension View {
    /// Draws this view's window in the appearance chosen in the settings of `graph`.
    public func appAppearance(_ graph: AppGraph) -> some View {
        background {
            AppAppearance(theme: graph.settings.settings.theme)
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
        }
    }
}
