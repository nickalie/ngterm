import SwiftUI
import UIKit

/// Which of the four values of a ``ColorToken`` applies: the light or dark side, with Increase Contrast off or on.
public struct Appearance: Hashable, Sendable, CaseIterable, CustomStringConvertible {
    /// Every appearance a token defines, the four DESIGN.md's promises are checked in.
    public static let allCases: [Self] = [
        Self(isDark: false, isIncreasedContrast: false),
        Self(isDark: true, isIncreasedContrast: false),
        Self(isDark: false, isIncreasedContrast: true),
        Self(isDark: true, isIncreasedContrast: true),
    ]

    /// The dark side.
    public let isDark: Bool
    /// Increase Contrast is on.
    public let isIncreasedContrast: Bool

    /// The traits UIKit resolves a dynamic color with in this appearance.
    @preconcurrency @MainActor public var traits: UITraitCollection {
        UITraitCollection { traits in
            traits.userInterfaceStyle = isDark ? .dark : .light
            traits.accessibilityContrast = isIncreasedContrast ? .high : .normal
        }
    }

    /// The minimum ratio text must reach in this appearance: AA, or AAA with Increase Contrast.
    public var textContrast: Double {
        isIncreasedContrast ? Contrast.increasedText : Contrast.text
    }

    /// "light", "dark", "light, increased contrast" or "dark, increased contrast".
    public var description: String {
        (isDark ? "dark" : "light") + (isIncreasedContrast ? ", increased contrast" : "")
    }

    /// The appearance with these settings.
    public init(isDark: Bool, isIncreasedContrast: Bool) {
        self.isDark = isDark
        self.isIncreasedContrast = isIncreasedContrast
    }

    /// The appearance of `traits`; an unspecified style is the light side.
    public init(traits: UITraitCollection) {
        self.init(
            isDark: traits.userInterfaceStyle == .dark,
            isIncreasedContrast: traits.accessibilityContrast == .high
        )
    }

    /// The appearance of a SwiftUI environment's color scheme and contrast.
    public init(colorScheme: ColorScheme, contrast: ColorSchemeContrast) {
        self.init(isDark: colorScheme == .dark, isIncreasedContrast: contrast == .increased)
    }
}
