import SwiftUI
import UIKit

extension RGB {
    /// The color as a UIKit color, opaque.
    public var uiColor: UIColor {
        UIColor(red: red, green: green, blue: blue, alpha: 1)
    }

    /// The color as a SwiftUI color, opaque.
    public var color: Color {
        Color(uiColor: uiColor)
    }

    /// `color` resolved in `appearance`, with its alpha dropped; black for one UIKit can't give as RGB.
    @preconcurrency
    @MainActor
    public init(resolving color: UIColor, in appearance: Appearance) {
        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0
        let resolved = color.resolvedColor(with: appearance.traits)
        guard resolved.getRed(&red, green: &green, blue: &blue, alpha: &alpha) else {
            self = .black
            return
        }
        self.init(red: red, green: green, blue: blue)
    }
}
