import NgtermDesign
import NgtermTerminal
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// Goldens of the session view's other covers and views (mobile/android's `SessionScreenshotTest`): a pane that
/// couldn't be shown with Retry, a program stopped by a signal or ended with no code, and a pane in chat view, live and
/// while its link comes back; and a machine whose ngterm can't share live sessions.
///
/// A golden's capture leaves a toolbar's glyphs out, so the switch to a chat's terminal is asserted on the toolbar's
/// items instead.
@MainActor
@Suite("session covers snapshots")
internal struct SessionCoversSnapshotTests {
    private static let phone = ScaffoldShot(device: .phone, isDark: false, isAccessibility: false)
    // how many layout passes SwiftUI gets to build the toolbar
    private static let passes = 4
    // what the session view's actions do: nothing
    private static let ignore: @MainActor () -> Void = {
        // a test presses nothing
    }

    // the titles of the trailing toolbar items of `world`'s session view, laid out in a window of the test's own
    private static func trailingTitles(_ world: SessionWorld) async throws -> [String] {
        let host = UIHostingController(
            rootView: NavigationStack {
                SessionScreen(
                    graph: world.graph,
                    session: SessionWorld.key,
                    actions: SessionActions(close: ignore, open: { _ in ignore() }, openInNewWindow: nil),
                    keyboard: TerminalKeyboard(hardware: HardwareKeyboard(fixed: true))
                )
            }
            .ngtermTheme()
        )
        // the frame's initializer, as SessionShot's: UIView's runs UIWindow's, which has no scene here
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: phone.size))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }
        for _ in 0..<passes {
            window.setNeedsLayout()
            window.layoutIfNeeded()
            await Task.yield()
        }
        return titles(of: host)
    }

    private static func titles(of controller: UIViewController) -> [String] {
        controller.navigationItem.trailingItemGroups.flatMap(\.barButtonItems).compactMap(\.title)
            + controller.children.flatMap(titles)
    }

    @Test("a pane that couldn't be shown says why, with Retry")
    internal func showFailed() async throws {
        let world = SessionWorld()
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        world.core.failNext("showPane", with: .Remote(reason: "the pane is busy"))
        let pane = SessionWorld.pane("P1")
        world.graph.panes.show(pane, size: SessionWorld.size)
        await Wait.until { world.graph.panes.failures[pane] != nil }
        await SessionShot.check(world, named: "show-failed", shot: Self.phone, hasTerminals: false)
    }

    @Test("a program stopped by a signal says which, and one that ended with no code says only that it ended")
    internal func exited() async throws {
        let world = SessionWorld()
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        await world.show("P1")
        let pane = SessionWorld.pane("P1")
        await world.emit(.paneEnded(pane: pane, end: .exited(code: nil, signal: "SIGTERM")))
        await SessionShot.check(world, named: "exited-signal", shot: Self.phone, hasTerminals: true)
        await world.emit(.paneEnded(pane: pane, end: .exited(code: nil, signal: nil)))
        await SessionShot.check(world, named: "exited-no-code", shot: Self.phone, hasTerminals: true)
    }

    @Test("the toolbar of a pane in chat view offers its terminal")
    internal func chatViewSwitch() async throws {
        let world = ChatWorld()
        defer { world.remove() }
        try await world.open()
        #expect(world.graph.chats.view(of: ChatWorld.pane) == .chat)
        #expect(try await Self.trailingTitles(world.world).contains("Show terminal"))
    }

    @Test("a pane in chat view shows its chat")
    internal func chatView() async throws {
        let world = ChatWorld()
        defer { world.remove() }
        try await world.open()
        await world.chat(
            ChatSamples.event("conv", ChatSamples.conversation),
            ChatSamples.event("u1", ChatSamples.user("Fix the install guide.")),
            ChatSamples.event("a1", ChatSamples.reply("I'll look at the installer's PATH check first.")),
            ChatSamples.event("live", .live)
        )
        await Wait.until { world.chat?.snapshot.phase == .live }
        await SessionShot.check(world.world, named: "chat-view", shot: Self.phone, hasTerminals: true)
    }

    @Test("a chat whose link went keeps its rows under the strip, with the last action's problem at its foot")
    internal func chatReconnecting() async throws {
        let world = ChatWorld()
        defer { world.remove() }
        try await world.open()
        await world.chat(
            ChatSamples.event("conv", ChatSamples.conversation),
            ChatSamples.event("u1", ChatSamples.user("Fix the install guide.")),
            ChatSamples.event("live", .live)
        )
        let chat = try #require(world.chat)
        await Wait.until { chat.snapshot.phase == .live }
        await world.world.emit(.chatState(pane: ChatWorld.pane, state: .reconnecting))
        #expect(chat.snapshot.phase == .reconnecting)
        chat.snapshot.problem = .unconfirmed
        await SessionShot.check(world.world, named: "chat-reconnecting", shot: Self.phone, hasTerminals: true)
    }

    @Test("a machine whose ngterm can't share live sessions says to update it, with Detach")
    internal func unsupported() async throws {
        let world = SessionWorld()
        defer { world.remove() }
        try await world.attach()
        await world.emit(.sessionConnection(machineId: "m1", sessionId: "s1", state: .unsupported))
        await SessionShot.check(world, named: "unsupported", shot: Self.phone, hasTerminals: false)
    }
}
