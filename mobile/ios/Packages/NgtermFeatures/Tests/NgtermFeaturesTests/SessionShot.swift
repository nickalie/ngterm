import NgtermDesign
import NgtermTerminal
import SnapshotTesting
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// Records the session view of a ``SessionWorld`` as a golden, in a ``ScaffoldShot``: a phone (one pane at a time) or
/// a tablet (the split), in a navigation stack for its toolbar.
///
/// SwiftUI makes the terminals as it lays the view out; each then reads its pane's first frame and draws it, as its
/// display link would on screen, before the image is taken.
@MainActor
internal enum SessionShot {
    // how many layout passes SwiftUI gets to make the terminals and size them
    private static let passes = 4
    private static let scale: CGFloat = 2
    // how alike a pixel must be to its golden's: text antialiasing differs a little between runs
    private static let perceptualPrecision: Float = 0.98
    // what a golden's actions do: nothing
    private static let ignore: @MainActor () -> Void = {
        // a golden presses nothing
    }

    /// Checks the session view of `world` against the golden `name` in `shot`, recorded on this simulator.
    internal static func check(
        _ world: SessionWorld,
        named name: String,
        shot: ScaffoldShot,
        hasTerminals: Bool
    ) async {
        let host = UIHostingController(rootView: screen(world, shot: shot))
        // a window of the test's own, the frame's (UIView's initializer runs UIWindow's, which has no scene here)
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: shot.size))
        guard let window = made as? UIWindow else {
            Issue.record("no window")
            return
        }
        window.traitOverrides.horizontalSizeClass = shot.sizeClass == .regular ? .regular : .compact
        window.traitOverrides.userInterfaceStyle = shot.isDark ? .dark : .light
        window.traitOverrides.displayScale = scale
        window.traitOverrides.preferredContentSizeCategory =
            shot.isAccessibility
            ? .accessibilityExtraExtraExtraLarge
            : .large
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }
        for _ in 0..<passes {
            window.setNeedsLayout()
            window.layoutIfNeeded()
            await Task.yield()
        }
        let shown = terminals(in: window)
        #expect(shown.isEmpty != hasTerminals, "the terminals on screen")
        for terminal in shown {
            terminal.drawFrame()
            terminal.displayNow()
        }
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        let image = UIGraphicsImageRenderer(bounds: window.bounds, format: format).image { context in
            window.layer.render(in: context.cgContext)
        }
        assertSnapshot(
            of: image,
            as: .image(precision: 1, perceptualPrecision: perceptualPrecision),
            named: "\(name)-\(shot.testDescription)-on-\(SnapshotDevice.current)",
            testName: "session"
        )
    }

    private static func screen(_ world: SessionWorld, shot: ScaffoldShot) -> some View {
        NavigationStack {
            SessionScreen(
                graph: world.graph,
                session: SessionWorld.key,
                actions: SessionActions(close: ignore, open: { _ in ignore() }, openInNewWindow: nil),
                keyboard: TerminalKeyboard(hardware: HardwareKeyboard(fixed: true))
            )
        }
        .ngtermTheme()
        .environment(\.colorScheme, shot.isDark ? .dark : .light)
        .environment(\.dynamicTypeSize, shot.isAccessibility ? .accessibility3 : .large)
        .environment(\.horizontalSizeClass, shot.sizeClass)
    }

    private static func terminals(in view: UIView) -> [TerminalView] {
        if let terminal = view as? TerminalView {
            return [terminal]
        }
        return view.subviews.flatMap(terminals)
    }
}
