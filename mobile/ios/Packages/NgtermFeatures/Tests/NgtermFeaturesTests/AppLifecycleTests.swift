import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The app's lifecycle over its graph, on a grace run out by hand: what it holds is its attached sessions and a
/// machine's sign-in, its suspend is the graph's (asks taken back, the core suspended) and so is its resume; a
/// refresh is asked for while sessions are attached, and runs only while the core is suspended or not yet started.
@MainActor
@Suite("the app's lifecycle over its graph")
internal struct AppLifecycleTests {
    private let test = TestGraph()
    private let grace = TestGrace()
    private let scheduler = TestScheduler()
    // a scene, as the test makes it: only its identity counts (a type's will do)
    private let window = ObjectIdentifier(Int.self)

    private func lifecycle() -> AppLifecycle {
        AppLifecycle(graph: test.graph, grace: grace, scheduler: scheduler)
    }

    private func attach() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
    }

    private func methods() -> [String] {
        test.core.calls.map(\.method).filter { $0 == "suspend" || $0 == "resume" }
    }

    @Test("with a session attached, the grace runs out into the graph's suspend, and the foreground resumes")
    internal func graceSuspendResume() async throws {
        defer { test.remove() }
        let life = lifecycle()
        try await attach()
        #expect(scheduler.isScheduled, "a refresh is asked for while a session is attached")
        test.core.emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        await test.settled()
        life.lifecycle.sceneEntered(window)
        life.lifecycle.sceneLeft(window)
        #expect(grace.isRunning)
        #expect(test.notices.upIds == [Alert.askId(SessionKey(machineId: "m1", sessionId: "s1"), askId: "A1")])
        grace.runOut()
        #expect(methods() == ["suspend"])
        #expect(test.graph.asks.open.isEmpty)
        #expect(test.notices.posted.isEmpty, "the ask's notice is withdrawn as the origin takes it back")
        life.lifecycle.sceneEntered(window)
        #expect(methods() == ["suspend", "resume"])
    }

    @Test("with nothing attached, the app suspends as it leaves, and no refresh is asked for")
    internal func nothingAttached() async throws {
        defer { test.remove() }
        let life = lifecycle()
        try await attach()
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(!scheduler.isScheduled)
        life.lifecycle.sceneEntered(window)
        life.lifecycle.sceneLeft(window)
        #expect(methods() == ["suspend"])
        #expect(!grace.isRunning)
    }

    @Test("a machine waiting for its sign-in keeps the app from suspending until it is done")
    internal func signIn() async throws {
        defer { test.remove() }
        let life = lifecycle()
        let machine = TestGraph.machine(id: "m2", label: "signin", host: "signin.lan", keyId: "k1")
        try test.seed(machine)
        test.core.signIns = ["signin.lan": .tailscale]
        let connecting = test.connections.connect(machine.core)
        await Wait.until { test.connections.isSigningIn }
        life.lifecycle.sceneEntered(window)
        life.lifecycle.sceneLeft(window)
        #expect(methods().isEmpty)
        test.core.finishSignIn(machineId: "m2")
        await connecting.value
        await test.settled()
        #expect(methods() == ["suspend"], "signed in with nothing attached: nothing is left to keep")
    }

    @Test("a refresh runs only while the core is suspended or not yet started, and asks for the next")
    internal func refreshRuns() async throws {
        defer { test.remove() }
        let life = lifecycle()
        try await attach()
        await life.refresh()
        #expect(test.refreshCore.calls.count == 3, "not started: the refresh runs")
        test.graph.start()
        life.lifecycle.sceneEntered(window)
        await life.refresh()
        #expect(test.refreshCore.calls.count == 3, "in the foreground the streams tell the statuses")
        life.lifecycle.sceneLeft(window)
        grace.runOut()
        let asked = scheduler.schedules
        await life.refresh()
        #expect(test.refreshCore.calls.count == 6, "suspended: the refresh runs")
        #expect(scheduler.schedules == asked + 1)
    }
}
