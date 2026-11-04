import SwiftUI
import Testing
import UIKit

/// The variants a screen is recorded in.
///
/// Light and dark at the default text size, and light at an accessibility text size (AX3), each a phone's screen (an
/// iPhone 17 upright) at a display scale of 2. Each simulator keeps its own goldens (``SnapshotDevice``). Public, as
/// ``ScreenSnapshot`` is.
public enum ScreenVariant: CaseIterable, Sendable, CustomTestStringConvertible {
    case accessibility
    case dark
    case light

    private static let displayScale: CGFloat = 2
    // an iPhone 17 upright, in points
    private static let width: CGFloat = 402
    private static let height: CGFloat = 874

    /// A phone's screen: an iPhone 17 upright.
    public static var size: CGSize {
        CGSize(width: width, height: height)
    }

    /// The variant's name in the test report.
    public var testDescription: String {
        "\(self)"
    }

    internal var colorScheme: ColorScheme {
        self == .dark ? .dark : .light
    }

    internal var typeSize: DynamicTypeSize {
        self == .accessibility ? .accessibility3 : .large
    }

    @MainActor internal var traits: UITraitCollection {
        UITraitCollection { traits in
            traits.displayScale = Self.displayScale
            traits.userInterfaceStyle = self == .dark ? .dark : .light
            traits.horizontalSizeClass = .compact
            traits.verticalSizeClass = .regular
            traits.preferredContentSizeCategory = self == .accessibility ? .accessibilityExtraExtraExtraLarge : .large
        }
    }
}
