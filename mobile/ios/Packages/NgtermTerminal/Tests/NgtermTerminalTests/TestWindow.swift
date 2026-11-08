import Testing
import UIKit

/// A window of a test's own, for what needs one (the display link, a first responder's focus moves).
@MainActor
internal enum TestWindow {
    /// A window of `frame`: UIView's initializer runs UIWindow's, which has no scene here.
    internal static func make(_ frame: CGRect) throws -> UIWindow {
        let made = (UIWindow.self as UIView.Type).init(frame: frame)
        return try #require(made as? UIWindow)
    }
}
