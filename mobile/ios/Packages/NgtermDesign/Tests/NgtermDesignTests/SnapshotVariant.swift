import SwiftUI
import UIKit

/// The variants every component is recorded in.
///
/// Light and dark at the default text size, and light at an accessibility text size (AX3), where layouts that stack or
/// wrap show it. Goldens are recorded on the pinned simulator runtime at a display scale of 2, one set per simulator
/// (``SnapshotDevice``).
internal enum SnapshotVariant: CaseIterable, Sendable {
    case accessibility
    case dark
    case light

    private static let displayScale: CGFloat = 2

    internal var name: String {
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
            traits.preferredContentSizeCategory = self == .accessibility ? .accessibilityExtraExtraExtraLarge : .large
        }
    }
}
