import SwiftUI
import Testing
import UIKit

/// One golden of the frame: a phone (compact: the stack) or a tablet (regular: the columns), light or dark, at the
/// default or an accessibility text size.
internal struct ScaffoldShot: Sendable, CustomTestStringConvertible {
    /// The device a golden is drawn at.
    internal enum Device: Sendable {
        case phone
        case tablet
    }

    internal static let all = [
        Self(device: .phone, isDark: false, isAccessibility: false),
        Self(device: .phone, isDark: true, isAccessibility: false),
        Self(device: .phone, isDark: false, isAccessibility: true),
        Self(device: .tablet, isDark: false, isAccessibility: false),
        Self(device: .tablet, isDark: true, isAccessibility: false),
    ]

    private static let displayScale: CGFloat = 2
    // an iPhone 17 upright, and an 11-inch iPad on its side
    private static let phoneWidth: CGFloat = 402
    private static let phoneHeight: CGFloat = 874
    private static let tabletWidth: CGFloat = 1_180
    private static let tabletHeight: CGFloat = 820

    internal let device: Device
    internal let isDark: Bool
    internal let isAccessibility: Bool

    internal var testDescription: String {
        "\(device)-\(isAccessibility ? "accessibility" : (isDark ? "dark" : "light"))"
    }

    internal var size: CGSize {
        device == .phone
            ? CGSize(width: Self.phoneWidth, height: Self.phoneHeight)
            : CGSize(width: Self.tabletWidth, height: Self.tabletHeight)
    }

    internal var sizeClass: UserInterfaceSizeClass {
        device == .phone ? .compact : .regular
    }

    @MainActor internal var traits: UITraitCollection {
        UITraitCollection { traits in
            traits.displayScale = Self.displayScale
            traits.userInterfaceStyle = isDark ? .dark : .light
            traits.horizontalSizeClass = device == .phone ? .compact : .regular
            traits.verticalSizeClass = .regular
            traits.userInterfaceIdiom = device == .phone ? .phone : .pad
            traits.preferredContentSizeCategory = isAccessibility ? .accessibilityExtraExtraExtraLarge : .large
        }
    }
}
