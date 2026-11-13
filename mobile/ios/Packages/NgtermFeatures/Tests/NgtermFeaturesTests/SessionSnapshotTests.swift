import NgtermCore
import Testing

@testable import NgtermFeatures

/// Goldens of the session view (mobile/android's `SessionScreenshotTest`): one pane, two panes on each axis, a hidden
/// split, a follower, a reconnecting pane, the HUD and the gone state, on a phone (one pane at a time) and a tablet
/// (the split).
@MainActor
@Suite("session snapshots")
internal struct SessionSnapshotTests {
    private static let phone = ScaffoldShot(device: .phone, isDark: false, isAccessibility: false)
    private static let phoneDark = ScaffoldShot(device: .phone, isDark: true, isAccessibility: false)
    private static let phoneLarge = ScaffoldShot(device: .phone, isDark: false, isAccessibility: true)
    private static let tablet = ScaffoldShot(device: .tablet, isDark: false, isAccessibility: false)
    private static let tabletDark = ScaffoldShot(device: .tablet, isDark: true, isAccessibility: false)

    private let world = SessionWorld()

    // the session streaming `layout`, its panes shown, its machine running `peer`
    private func live(_ layout: RemoteLayout, peer: PeerKind? = nil) async throws {
        try await world.attach(["s1"], of: ["s1"], peer: peer)
        await world.stream(layout)
        for paneId in layout.panes {
            await world.show(paneId)
        }
        let status = RemoteStatus(
            status: "working",
            blink: false,
            color: nil,
            shape: nil,
            pane: nil,
            changedAt: nil
        )
        await world.emit(
            .sessionStatus(machineId: "m1", sessionId: "s1", status: status, changed: false),
            .sessionContext(machineId: "m1", sessionId: "s1", context: "Fix the parser")
        )
    }

    @Test("one pane")
    internal func onePane() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(["P1"], axis: nil, shown: true))
        for shot in [Self.phone, Self.phoneDark, Self.phoneLarge, Self.tablet] {
            await SessionShot.check(world, named: "one-pane", shot: shot, hasTerminals: true)
        }
    }

    @Test("two panes side by side: one at a time with the switcher on a phone, the split on a tablet")
    internal func sideBySide() async throws {
        defer { world.remove() }
        try await live(SessionWorld.split("vertical"))
        world.graph.panes.select("P2", of: SessionWorld.key)
        for shot in [Self.phone, Self.tablet, Self.tabletDark] {
            await SessionShot.check(world, named: "side-by-side", shot: shot, hasTerminals: true)
        }
    }

    @Test("two panes stacked")
    internal func stacked() async throws {
        defer { world.remove() }
        try await live(SessionWorld.split("horizontal"))
        await SessionShot.check(world, named: "stacked", shot: Self.tablet, hasTerminals: true)
    }

    @Test("a split the origin hides shows the selected pane alone")
    internal func hiddenSplit() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(SessionWorld.panes, axis: "vertical", shown: false))
        await SessionShot.check(world, named: "hidden-split", shot: Self.tablet, hasTerminals: true)
    }

    @Test("a follower's pane under its cover, with Take control")
    internal func follower() async throws {
        defer { world.remove() }
        try await live(SessionWorld.split("vertical"))
        await world.emit(.paneLead(pane: SessionWorld.pane("P2"), role: .follower, covered: true))
        for shot in [Self.phone, Self.tablet] {
            world.graph.panes.select("P2", of: SessionWorld.key)
            await SessionShot.check(world, named: "follower", shot: shot, hasTerminals: true)
        }
    }

    @Test("a reconnecting pane keeps its last screen under the strip, and the toolbar says so")
    internal func reconnecting() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(["P1"], axis: nil, shown: true))
        await world.emit(
            .paneEnded(pane: SessionWorld.pane("P1"), end: .lost(reason: "connection reset")),
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "connection reset"))
        )
        for shot in [Self.phone, Self.tablet] {
            await SessionShot.check(world, named: "reconnecting", shot: shot, hasTerminals: true)
        }
    }

    @Test("the program exited, and the HUD over the panes")
    internal func exitedWithHud() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(["P1"], axis: nil, shown: true))
        let hud = RemoteHud(
            message: "Deploying api",
            detail: "step 2 of 3",
            spinner: "bar",
            backgroundColor: nil,
            textColor: nil,
            sizePercent: nil,
            position: "top",
            markdown: false,
            fontSize: nil,
            sticky: false,
            frame: true,
            pane: nil,
            generation: 1,
            remaining: nil
        )
        await world.emit(
            .hud(machineId: "m1", sessionId: "s1", hud: hud),
            .paneEnded(pane: SessionWorld.pane("P1"), end: .exited(code: 1, signal: nil))
        )
        await SessionShot.check(world, named: "exited-hud", shot: Self.phone, hasTerminals: true)
    }

    @Test("an agterm session: its badge beside the state, its agent's pane showing the terminal")
    internal func agterm() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(["P1"], axis: nil, shown: true), peer: .agterm)
        await world.emit(
            .paneAgent(machineId: "m1", sessionId: "s1", paneId: "P1", agent: "claude", conversationId: "c1")
        )
        for shot in [Self.phone, Self.phoneDark, Self.phoneLarge, Self.tablet] {
            await SessionShot.check(world, named: "agterm", shot: shot, hasTerminals: true)
        }
    }

    @Test("a session ended on its machine is gone")
    internal func gone() async throws {
        defer { world.remove() }
        try await live(SessionWorld.layout(["P1"], axis: nil, shown: true))
        await world.emit(.paneEnded(pane: SessionWorld.pane("P1"), end: .gone))
        await SessionShot.check(world, named: "gone", shot: Self.phone, hasTerminals: false)
    }
}
