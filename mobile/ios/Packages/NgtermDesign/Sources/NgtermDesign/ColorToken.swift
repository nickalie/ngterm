import SwiftUI
import UIKit

/// One of the app's own colors, in its four appearances (``Appearance``).
///
/// It draws as a dynamic color that follows the light or dark side and Increase Contrast by itself.
public struct ColorToken: Hashable, Sendable {
    /// The light side's value.
    public let light: RGB
    /// The dark side's value.
    public let dark: RGB
    /// The light side's value with Increase Contrast on.
    public let lightIncreased: RGB
    /// The dark side's value with Increase Contrast on.
    public let darkIncreased: RGB

    /// The token as a dynamic UIKit color.
    public var uiColor: UIColor {
        let token = self
        return UIColor { traits in token.resolved(in: Appearance(traits: traits)).uiColor }
    }

    /// The token as a SwiftUI color, following the environment's color scheme and contrast.
    public var color: Color {
        Color(uiColor: uiColor)
    }

    /// The token with these values: `light` and `dark`, then the same with Increase Contrast on.
    public init(light: RGB, dark: RGB, lightIncreased: RGB, darkIncreased: RGB) {
        self.light = light
        self.dark = dark
        self.lightIncreased = lightIncreased
        self.darkIncreased = darkIncreased
    }

    /// The token's value in `appearance`.
    public func resolved(in appearance: Appearance) -> RGB {
        switch (appearance.isDark, appearance.isIncreasedContrast) {
        case (false, false):
            light

        case (true, false):
            dark

        case (false, true):
            lightIncreased

        case (true, true):
            darkIncreased
        }
    }
}
