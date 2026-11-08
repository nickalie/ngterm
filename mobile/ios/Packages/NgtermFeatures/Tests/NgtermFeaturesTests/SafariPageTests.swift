import Foundation
import SafariServices
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// The web pages the app opens in Safari's view (a sign-in link, the server setup page), and that view's Done.
@MainActor
@Suite("Safari's view")
internal struct SafariPageTests {
    // how many layout passes the window gets to put the view's controller in place
    private static let passes = 3

    // the Safari view controllers under `controller`, itself included
    private static func safari(in controller: UIViewController) -> [SFSafariViewController] {
        let own = (controller as? SFSafariViewController).map { [$0] } ?? []
        return own + controller.children.flatMap(safari(in:))
    }

    @Test(
        "only a web page opens in Safari's view, whatever its scheme's case",
        arguments: [
            ("https://login.tailscale.com/a/1f2e3d", true),
            ("http://build-01.lan:8080/setup", true),
            ("HTTPS://example.com", true),
            ("mailto:ops@example.com", false),
            ("tailscale://login", false),
            ("file:///etc/passwd", false),
            ("example.com/login", false),
        ]
    )
    internal func canShow(link: String, isShown: Bool) throws {
        let url = try #require(URL(string: link))
        #expect(SafariPage.canShow(url) == isShown)
    }

    @Test("a page is one sheet per address")
    internal func identity() throws {
        let url = try #require(URL(string: "https://login.tailscale.com/a/1f2e3d"))
        let other = try #require(URL(string: "https://login.tailscale.com/a/9a8b7c"))
        let page = SafariPage(url: url)
        #expect(page.id == url)
        #expect(page == SafariPage(url: url))
        #expect(page.id != SafariPage(url: other).id)
    }

    @Test("the view shows the page with a Done button, and Done closes it")
    internal func done() async throws {
        let url = try #require(URL(string: "https://login.tailscale.com/a/1f2e3d"))
        var closed = 0
        let host = UIHostingController(rootView: SafariView(url: url) { closed += 1 })
        // a window of the test's own (UIView's initializer runs UIWindow's, which has no scene here)
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: ScreenVariant.size))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }
        for _ in 0..<Self.passes {
            window.layoutIfNeeded()
            await Task.yield()
        }
        let shown = try #require(Self.safari(in: host).first)
        #expect(shown.dismissButtonStyle == .done)
        #expect(closed == 0)
        // Safari's view tells its delegate when the user taps Done
        shown.delegate?.safariViewControllerDidFinish?(shown)
        #expect(closed == 1)
    }
}
