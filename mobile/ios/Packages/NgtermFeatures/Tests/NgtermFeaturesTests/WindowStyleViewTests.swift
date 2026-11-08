import Testing
import UIKit

@testable import NgtermFeatures

/// The view that sets its window's light or dark style (Settings' Appearance): the window takes the style as the view
/// joins it and as the style changes, and the system's again once it is unspecified.
@MainActor
@Suite("the window's style")
internal struct WindowStyleViewTests {
    // a window of the test's own (UIView's initializer runs UIWindow's, which has no scene here)
    private static func window() throws -> UIWindow {
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
        return try #require(made as? UIWindow)
    }

    @Test("the window takes the style the view had before it joined")
    internal func joins() throws {
        let window = try Self.window()
        let view = WindowStyleView(frame: .zero)
        view.style = .dark
        #expect(window.overrideUserInterfaceStyle == .unspecified)
        window.addSubview(view)
        #expect(window.overrideUserInterfaceStyle == .dark)
    }

    @Test("the window follows each change of the style, and the system's again once it is unspecified")
    internal func follows() throws {
        let window = try Self.window()
        let view = WindowStyleView(frame: .zero)
        window.addSubview(view)
        #expect(window.overrideUserInterfaceStyle == .unspecified)
        view.style = .light
        #expect(window.overrideUserInterfaceStyle == .light)
        view.style = .dark
        #expect(window.overrideUserInterfaceStyle == .dark)
        view.style = .unspecified
        #expect(window.overrideUserInterfaceStyle == .unspecified)
    }

    @Test("the view takes no touch and is no element VoiceOver reads")
    internal func quiet() {
        let view = WindowStyleView(frame: .zero)
        #expect(!view.isUserInteractionEnabled)
        #expect(!view.isAccessibilityElement)
        #expect(view.accessibilityElementsHidden)
    }
}
