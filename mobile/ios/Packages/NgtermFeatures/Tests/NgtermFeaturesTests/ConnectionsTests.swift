import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The connections over the fake core: each machine's link, listing and attached sessions, as mobile/android's
/// `ConnectionsTest` has them.
@MainActor
@Suite("the connections")
internal struct ConnectionsTests {
    private let machine = Machine(
        id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: "k1"
    )
    private let pins = MemoryHostKeyPins()
    private let clock = TestClock()
    private let core: FakeMobileCore
    private let connections: Connections

    internal init() {
        core = FakeMobileCore(hosts: pins)
        connections = Connections(core: core, clock: clock.read)
    }

    private func settled() async {
        await Wait.settled(connections, core)
    }

    private func calls(_ method: String) -> [FakeCall] {
        core.calls.filter { $0.method == method }
    }

    @Test("a connect goes from connecting to connected and lists the sessions")
    internal func connectLists() async {
        core.sessions["m1"] = [fakeSession(id: "s1")]
        let connecting = connections.connect(machine)
        #expect(connections.links["m1"] == .connecting)
        await connecting.value
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(connections.trees["m1"] == [fakeSession(id: "s1")])
        connections.disconnect(machineId: "m1")
        #expect(connections.links["m1"] == nil)
        #expect(connections.trees["m1"] == nil)
    }

    @Test("a connect cancelled by a suspend is listed when the resumed connection comes")
    internal func cancelledThenResumed() async throws {
        core.sessions["m1"] = [fakeSession(id: "s1")]
        core.failNext("connect", with: .Cancelled)
        await connections.connect(machine).value
        await settled()
        #expect(connections.trees["m1"] == nil)
        // the resume's connect has no caller in the app: the core only reports it (the fake connects as it would)
        try await core.connect(machine: machine)
        await Wait.until { connections.trees["m1"] == [fakeSession(id: "s1")] }
        #expect(connections.links["m1"] == .connected)
    }

    @Test("a connect cancelled after the resumed connection reported is still listed, once")
    internal func cancelledAfterResumed() async throws {
        core.sessions["m1"] = [fakeSession(id: "s1")]
        core.failNext("connect", with: .Cancelled)
        let gate = core.hold("connect")
        let connecting = connections.connect(machine)
        await gate.reached()
        // the resume connects and reports Connected while the cancelled call has yet to return to the app
        try await core.connect(machine: machine)
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(connections.trees["m1"] == nil)
        gate.open()
        await connecting.value
        await settled()
        #expect(connections.trees["m1"] == [fakeSession(id: "s1")])
        #expect(calls("listSessions").count == 1)
    }

    @Test("a failed connect is unreachable with the core's reason")
    internal func failedConnect() async {
        core.failNext("connect", with: .Unreachable(reason: "build-01.lan:22: connection refused"))
        await connections.connect(machine).value
        await settled()
        #expect(connections.links["m1"] == .unreachable(reason: "build-01.lan:22: connection refused"))
    }

    @Test("an unknown host key waits for the answer, and trusting it connects and pins it")
    internal func trustHostKey() async {
        core.hostKeys["build-01.lan"] = "SHA256:new"
        let connecting = connections.connect(machine)
        await Wait.until { connections.links["m1"] == .hostKeyUnknown(fingerprint: "SHA256:new") }
        connections.answerHostKey(machineId: "m1", fingerprint: "SHA256:new", trust: true)
        #expect(connections.links["m1"] == .connecting)
        await connecting.value
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(pins.all == ["m1": "SHA256:new"])
    }

    @Test("a declined host key leaves the machine offline, not failed")
    internal func declineHostKey() async {
        core.hostKeys["build-01.lan"] = "SHA256:new"
        let connecting = connections.connect(machine)
        await Wait.until { connections.links["m1"] == .hostKeyUnknown(fingerprint: "SHA256:new") }
        connections.answerHostKey(machineId: "m1", fingerprint: "SHA256:new", trust: false)
        await connecting.value
        await settled()
        #expect(connections.links["m1"] == nil)
        #expect(pins.all.isEmpty)
    }

    @Test("a sign-in shows while the connect waits, and the connected state ends it")
    internal func signIn() async {
        let signIn = Link.signIn(
            message: FakeSignIn.tailscale.message, url: FakeSignIn.tailscale.url, isReconnecting: false
        )
        core.signIns["build-01.lan"] = .tailscale
        #expect(!connections.isSigningIn)
        _ = connections.connect(machine)
        await Wait.until { connections.links["m1"] == signIn }
        #expect(connections.isSigningIn)
        // connecting again as it is joins the waiting connect: the sign-in stays
        let again = connections.connect(machine)
        #expect(connections.links["m1"] == signIn)
        await Wait.until { core.waitingForSignIn == ["m1"] && calls("connect").count == 2 }
        core.finishSignIn(machineId: "m1")
        await again.value
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(!connections.isSigningIn)
    }

    @Test("a sign-in not finished is unreachable, and a disconnect takes a waiting one away")
    internal func signInUnfinished() async {
        core.signIns["build-01.lan"] = .tailscale
        let first = connections.connect(machine)
        await Wait.until { core.waitingForSignIn == ["m1"] }
        core.finishSignIn(machineId: "m1", signedIn: false)
        await first.value
        await settled()
        #expect(connections.links["m1"] == .unreachable(reason: FakeMobileCore.signInUnfinished))
        let second = connections.connect(machine)
        await Wait.until { core.waitingForSignIn == ["m1"] }
        await settled()
        #expect(connections.isSigningIn)
        connections.disconnect(machineId: "m1")
        await second.value
        await settled()
        #expect(connections.links["m1"] == nil)
        #expect(core.waitingForSignIn.isEmpty)
    }

    @Test("an edited machine waiting for a sign-in connects anew")
    internal func editedDuringSignIn() async {
        core.signIns["build-01.lan"] = .tailscale
        let first = connections.connect(machine)
        await Wait.until { connections.isSigningIn }
        var edited = machine
        edited.host = "build-02.lan"
        let second = connections.connect(edited)
        #expect(connections.links["m1"] == .connecting)
        await second.value
        await settled()
        #expect(connections.links["m1"] == .connected)
        // the overtaken connect, which the real core cancels, is let go here
        core.finishSignIn(machineId: "m1")
        await first.value
    }

    @Test("a changed host key stays changed through the failure that follows it")
    internal func changedHostKey() async {
        pins.pin(machineId: "m1", fingerprint: "SHA256:old")
        core.hostKeys["build-01.lan"] = "SHA256:new"
        await connections.connect(machine).value
        await settled()
        #expect(connections.links["m1"] == .hostKeyChanged(pinned: "SHA256:old", offered: "SHA256:new"))
    }

    @Test("a reconnect counts down from the clock")
    internal func reconnectCountdown() async {
        await connections.connect(machine).value
        core.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 4_000)))
        await settled()
        #expect(connections.links["m1"] == .reconnecting(retryAt: clock.now.addingTimeInterval(4)))
        core.emit(.machineState(machineId: "m1", state: .unreachable(reason: FakeMobileCore.hostKeyChanged)))
        await settled()
        #expect(connections.links["m1"] == .unreachable(reason: FakeMobileCore.hostKeyChanged))
    }

    @Test("a failed listing is kept until one lands, and a machine not connected fails none")
    internal func failedListing() async {
        core.failNext("listSessions", with: .Remote(reason: "/bin/sh: 1: ngtermctl: not found"))
        await connections.connect(machine).value
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(connections.treeFailures["m1"] == "/bin/sh: 1: ngtermctl: not found")
        #expect(connections.trees["m1"] == nil)
        await connections.listSessions(machineId: "m1")
        await settled()
        #expect(connections.treeFailures["m1"] == nil)
        #expect(connections.trees["m1"]?.isEmpty == true)
        connections.disconnect(machineId: "m1")
        await connections.listSessions(machineId: "m1")
        #expect(connections.treeFailures["m1"] == nil)
    }

    @Test("a listing dropped while reconnecting is redone once the machine is back, and only then")
    internal func listingRedoneAfterReconnect() async {
        core.failNext("listSessions", with: .Remote(reason: "build-01 gave no answer"))
        let gate = core.hold("listSessions")
        let connecting = connections.connect(machine)
        await gate.reached()
        // the connection drops before the first listing answers, which then fails while it is down
        core.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 4_000)))
        await settled()
        gate.open()
        await connecting.value
        await settled()
        #expect(connections.treeFailures["m1"] == nil)
        #expect(connections.trees["m1"] == nil)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        core.emit(.machineState(machineId: "m1", state: .connected))
        await Wait.until { connections.trees["m1"] == [fakeSession(id: "s1")] }
        #expect(connections.links["m1"] == .connected)
        // a Connected that changes nothing lists nothing more
        let listings = calls("listSessions").count
        core.emit(.machineState(machineId: "m1", state: .connected))
        await settled()
        #expect(calls("listSessions").count == listings)
    }

    @Test("a reconnect that has to sign in again lists the machine once it is back")
    internal func reconnectSignsInAgain() async {
        let tailscale = FakeSignIn.tailscale
        await connections.connect(machine).value
        // the connection drops, and the core's reconnect meets a check that ran out while it was down
        core.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 4_000)))
        core.emit(.signInRequested(machineId: "m1", message: tailscale.message, url: tailscale.url))
        await settled()
        let again = Link.signIn(message: tailscale.message, url: tailscale.url, isReconnecting: true)
        #expect(connections.links["m1"] == again)
        // a second link while it still waits keeps it a reconnect
        core.emit(.signInRequested(machineId: "m1", message: tailscale.message, url: tailscale.url))
        await settled()
        #expect(connections.links["m1"] == again)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        let listings = calls("listSessions").count
        core.emit(.machineState(machineId: "m1", state: .connected))
        await Wait.until { connections.trees["m1"] == [fakeSession(id: "s1")] }
        #expect(connections.links["m1"] == .connected)
        #expect(calls("listSessions").count == listings + 1)
    }

    @Test("an attached session is let go when it ends, leaves the listing, or its machine disconnects")
    internal func attachedLetGo() async throws {
        core.sessions["m1"] = [fakeSession(id: "s1"), fakeSession(id: "s2"), fakeSession(id: "s3")]
        await connections.connect(machine).value
        await settled()
        try await connections.attach(machineId: "m1", sessionId: "s1")
        try await connections.attach(machineId: "m1", sessionId: "s2")
        #expect(connections.attached["m1"] == ["s1", "s2"])
        try await connections.endSession(machineId: "m1", sessionId: "s1")
        await settled()
        #expect(connections.attached["m1"] == ["s2"])
        // ended elsewhere: the next listing doesn't have it
        core.sessions["m1"]?.removeAll { $0.id == "s2" }
        await connections.listSessions(machineId: "m1")
        await settled()
        #expect(connections.attached["m1"] == nil)
        #expect(calls("detachSession").map { $0.args[1] as? String } == ["s1", "s2"])
        try await connections.attach(machineId: "m1", sessionId: "s3")
        connections.disconnect(machineId: "m1")
        #expect(connections.attached["m1"] == nil)
    }

    @Test("a failed listing overtaken by a newer one is dropped")
    internal func overtakenListing() async {
        await connections.connect(machine).value
        await settled()
        // the older listing fails only after a newer one has landed
        core.failNext("listSessions", with: .Remote(reason: "build-01 gave no answer"))
        let gate = core.hold("listSessions")
        let older = Task {
            await connections.listSessions(machineId: "m1")
        }
        await gate.reached()
        core.sessions["m1"] = [fakeSession(id: "s1")]
        await connections.listSessions(machineId: "m1")
        await settled()
        gate.open()
        await older.value
        await settled()
        #expect(connections.treeFailures["m1"] == nil)
        #expect(connections.trees["m1"] == [fakeSession(id: "s1")])
    }

    @Test("an edited machine lets go of its attached sessions; a renamed one keeps them")
    internal func editedMachine() async throws {
        core.sessions["m1"] = [fakeSession(id: "s1"), fakeSession(id: "s2")]
        await connections.connect(machine).value
        await settled()
        try await connections.attach(machineId: "m1", sessionId: "s1")
        // connected again as it is: the attachment stays
        await connections.connect(machine).value
        #expect(connections.attached["m1"] == ["s1"])
        // renamed: the connection and the attachment stay
        var renamed = machine
        renamed.label = "build-01 (lab)"
        await connections.connect(renamed).value
        #expect(connections.attached["m1"] == ["s1"])
        #expect(calls("detachSession").isEmpty)
        // edited: the core replaces the connection and ends the streams; the new listing still has s1
        var edited = renamed
        edited.port = 2_222
        await connections.connect(edited).value
        await settled()
        #expect(connections.links["m1"] == .connected)
        #expect(connections.trees["m1"]?.map(\.id) == ["s1", "s2"])
        #expect(connections.attached["m1"] == nil)
        #expect(calls("detachSession").map { $0.args[1] as? String } == ["s1"])
    }

    @Test("create and end are the core's, and a blank name is none")
    internal func createAndEnd() async throws {
        await connections.connect(machine).value
        await settled()
        let created = try await connections.createSession(machineId: "m1", name: "  ")
        await settled()
        #expect(connections.trees["m1"] == [created])
        #expect(calls("createSession").last == FakeCall("createSession", ["m1", String?.none]))
        _ = try await connections.createSession(machineId: "m1", name: " api ")
        #expect(calls("createSession").last == FakeCall("createSession", ["m1", String?("api")]))
        try await connections.endSession(machineId: "m1", sessionId: created.id)
        await settled()
        #expect(connections.trees["m1"]?.map(\.name) == ["api"])
    }

    @Test("a core that has stopped leaves every machine offline")
    internal func stoppedCore() async {
        await connections.connect(machine).value
        await settled()
        core.failNext("networkChanged", with: .Stopped)
        connections.networkChanged()
        #expect(connections.links.isEmpty)
    }
}
