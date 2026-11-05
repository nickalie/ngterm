import SwiftUI
import Testing
import UIKit

/// One golden of the Settings sheet: a compact width (a phone's sheet) or a regular one (an iPad's form sheet), light
/// or dark, and the phone at an accessibility text size; tall enough to hold the whole form.
internal struct SettingsShot: Sendable, CustomTestStringConvertible {
    internal static let all = [
        Self(isRegular: false, isDark: false, isAccessibility: false),
        Self(isRegular: false, isDark: true, isAccessibility: false),
        Self(isRegular: false, isDark: false, isAccessibility: true),
        Self(isRegular: true, isDark: false, isAccessibility: false),
        Self(isRegular: true, isDark: true, isAccessibility: false),
    ]

    private static let displayScale: CGFloat = 2
    // an iPhone 17 upright's width, and an iPad's form sheet's; each tall enough for every section
    private static let compactWidth: CGFloat = 402
    private static let regularWidth: CGFloat = 580
    private static let height: CGFloat = 3_400
    private static let accessibilityHeight: CGFloat = 7_600

    internal let isRegular: Bool
    internal let isDark: Bool
    internal let isAccessibility: Bool

    internal var testDescription: String {
        "\(isRegular ? "regular" : "compact")-\(isAccessibility ? "accessibility" : (isDark ? "dark" : "light"))"
    }

    internal var size: CGSize {
        CGSize(
            width: isRegular ? Self.regularWidth : Self.compactWidth,
            height: isAccessibility ? Self.accessibilityHeight : Self.height
        )
    }

    internal var colorScheme: ColorScheme {
        isDark ? .dark : .light
    }

    internal var typeSize: DynamicTypeSize {
        isAccessibility ? .accessibility3 : .large
    }

    @MainActor internal var traits: UITraitCollection {
        UITraitCollection { traits in
            traits.displayScale = Self.displayScale
            traits.userInterfaceStyle = isDark ? .dark : .light
            traits.horizontalSizeClass = isRegular ? .regular : .compact
            traits.verticalSizeClass = .regular
            traits.preferredContentSizeCategory = isAccessibility ? .accessibilityExtraExtraExtraLarge : .large
        }
    }
}
