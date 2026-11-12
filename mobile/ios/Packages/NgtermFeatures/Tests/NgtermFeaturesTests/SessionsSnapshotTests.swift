import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of a machine's sessions in each state, and of the New session sheet.
@MainActor
@Suite("sessions snapshots")
internal struct SessionsSnapshotTests {
    /// A state the screen shows instead of the list, and what the core tells to bring it about.
    internal enum Problem: CaseIterable, Sendable {
        case changedKey
        case dropped
        case listFailed
        case missingCli
        case notConnected
        case offline
        case reading
        case unreachable
    }

    private static var sessions: [RemoteSession] {
        var docs = fakeSession(id: "s1", name: "docs-site")
        docs.status = "blocked"
        docs.context = "Rewrite the install guide"
        var review = fakeSession(id: "s2", name: "docs review", parent: "s1")
        review.status = "active"
        var api = fakeSession(id: "s3", name: "api-refactor")
        api.status = "active"
        api.context = "Split the dispatcher by command group"
        var release = fakeSession(id: "s4", name: "release")
        release.status = "completed"
        return [docs, api, fakeSession(id: "s5", name: "shell"), release, review]
    }

    private let test = TestGraph()

    // m1 saved and its connect under way (never run before the golden is taken), so the core's events about it count
    private func machine() throws -> SessionsModel {
        try test.seed(TestGraph.machine(keyId: "fake-key"))
        test.connections.connect(TestGraph.machine(keyId: "fake-key").core).cancel()
        let model = SessionsModel(graph: test.graph, machineId: "m1")
        // the screen's first listing is the test's to tell
        model.opened()?.cancel()
        return model
    }

    // m1 connected and listed with `sessions`, `attached` of them attached here
    private func listed(_ sessions: [RemoteSession], attached: Set<String> = []) throws -> SessionsModel {
        let model = try machine()
        test.connections.receive(.machineState(machineId: "m1", state: .connected))
        test.connections.receive(.tree(machineId: "m1", sessions: sessions))
        test.connections.attached["m1"] = attached.isEmpty ? nil : attached
        return model
    }

    private func screen(_ model: SessionsModel) -> some View {
        NavigationStack {
            SessionsView(
                model: model,
                machines: MachinesModel(graph: test.graph),
                selection: nil,
                onOpen: { _ in
                    // a golden opens nothing
                },
                onGone: ScreenSnapshot.ignore
            )
        }
        .environment(\.fixedNow, test.clock.now)
    }

    private func offline() {
        test.graph.network.start()
        test.path.send(NetworkPath(isSatisfied: false, interfaces: []))
    }

    // the machine connected, and its listing failing for `reason`
    private func listingFails(_ reason: String) async {
        test.connections.receive(.machineState(machineId: "m1", state: .connected))
        test.core.failNext("listSessions", with: .Remote(reason: reason))
        await test.connections.listSessions(machineId: "m1")
    }

    private func tell(_ problem: Problem) async {
        let connections = test.connections
        switch problem {
        case .changedKey:
            connections.receive(.hostKeyChanged(machineId: "m1", pinned: "SHA256:old", offered: "SHA256:new"))

        case .dropped:
            connections.receive(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 5_000)))

        case .listFailed:
            await listingFails("build-01 gave no answer")

        case .missingCli:
            await listingFails("/bin/sh: 1: ngtermctl: not found")

        case .notConnected:
            connections.disconnect(machineId: "m1")

        case .offline:
            connections.disconnect(machineId: "m1")
            offline()

        case .reading:
            connections.receive(.machineState(machineId: "m1", state: .connected))

        case .unreachable:
            connections.receive(
                .machineState(machineId: "m1", state: .unreachable(reason: "build-01.lan:22: connection refused"))
            )
        }
    }

    @Test("the sessions, attached first, companions under their parent", arguments: ScreenVariant.allCases)
    internal func sessions(variant: ScreenVariant) async throws {
        defer { test.remove() }
        let model = try listed(Self.sessions, attached: ["s3"])
        // an End under way shows on its row until the core answers, after the golden
        let gate = test.core.hold("endSession")
        let ending = model.end(sessionId: "s4")
        ScreenSnapshot.check(screen(model), named: "sessions", variant: variant)
        gate.open()
        await ending?.value
    }

    @Test("no sessions yet", arguments: ScreenVariant.allCases)
    internal func empty(variant: ScreenVariant) throws {
        defer { test.remove() }
        ScreenSnapshot.check(screen(try listed([])), named: "sessions-empty", variant: variant)
    }

    @Test("New session under way: a spinner in the +'s place, over the list or no sessions", arguments: [false, true])
    internal func creating(isEmpty: Bool) async throws {
        defer { test.remove() }
        let model = try listed(isEmpty ? [] : Self.sessions)
        let gate = test.core.hold("createSession")
        let creating = model.createAndOpen { _ in
            // a golden opens nothing
        }
        let name = isEmpty ? "sessions-empty-creating" : "sessions-creating"
        ScreenSnapshot.check(screen(model), named: name, variant: .light)
        gate.open()
        await creating?.value
    }

    @Test("offline, a refresh that failed, and an End that failed")
    internal func banners() async throws {
        defer { test.remove() }
        let model = try listed(Self.sessions)
        test.core.failNext("listSessions", with: .Remote(reason: "build-01 gave no answer"))
        await test.connections.listSessions(machineId: "m1")
        model.notice = "no session with that id"
        offline()
        ScreenSnapshot.check(screen(model), named: "sessions-banners", variant: .light)
    }

    @Test("no sessions, and a refresh that failed")
    internal func emptyFailed() async throws {
        defer { test.remove() }
        let model = try listed([])
        test.core.failNext("listSessions", with: .Remote(reason: "build-01 gave no answer"))
        await test.connections.listSessions(machineId: "m1")
        ScreenSnapshot.check(screen(model), named: "sessions-empty-failed", variant: .light)
    }

    @Test("what shows instead of the list", arguments: Problem.allCases)
    internal func problems(problem: Problem) async throws {
        defer { test.remove() }
        let model = try machine()
        await tell(problem)
        ScreenSnapshot.check(screen(model), named: "sessions-\(problem)", variant: .light)
    }

    @Test("New session", arguments: ScreenVariant.allCases)
    internal func newSession(variant: ScreenVariant) {
        let sheet = NewSessionSheet(machine: "build-01", state: NewSessionState()) { _ in
            // a golden creates nothing
        } onCancel: {
            // nor cancels
        }
        ScreenSnapshot.check(sheet, named: "new-session", variant: variant)
    }

    @Test("New session, creating, and after a create that failed")
    internal func newSessionStates() {
        for (name, state) in [
            ("new-session-creating", NewSessionState(isRunning: true)),
            ("new-session-failed", NewSessionState(failure: "build-01 does not run live sessions")),
        ] {
            let sheet = NewSessionSheet(machine: "build-01", state: state) { _ in
                // a golden creates nothing
            } onCancel: {
                // nor cancels
            }
            ScreenSnapshot.check(sheet, named: name, variant: .light)
        }
    }
}
