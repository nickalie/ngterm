import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A session renamed on its machine (mobile/android's `ConnectionsTest`): an attached session is listed under the name
/// its stream last said, and under its listing's again once it is detached.
@MainActor
@Suite("the connections' stream names")
internal struct ConnectionsNamesTests {
    private let machine = Machine(
        id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: "k1"
    )
    private let core = FakeMobileCore()
    private let connections: Connections

    internal init() {
        connections = Connections(core: core)
    }

    private func settled() async {
        await Wait.settled(connections, core)
    }

    @Test("an attached session is listed under the name its stream last said")
    internal func streamNames() async throws {
        let names = { connections.trees["m1"]?.map(\.name) }
        core.sessions["m1"] = [fakeSession(id: "s1", name: "ngterm"), fakeSession(id: "s2", name: "docs")]
        await connections.connect(machine).value
        await settled()
        try await connections.attach(machineId: "m1", sessionId: "s1")
        // renamed on its machine: the stream says so before any listing does
        core.emit(.sessionName(machineId: "m1", sessionId: "s1", name: "Session Labels"))
        await settled()
        #expect(names() == ["Session Labels", "docs"])
        // a blank name says nothing, and a session not attached here keeps its listing's
        core.emit(.sessionName(machineId: "m1", sessionId: "s1", name: " "))
        core.emit(.sessionName(machineId: "m1", sessionId: "s2", name: "Docs Site"))
        await settled()
        #expect(names() == ["Session Labels", "docs"])
        // a listing read before the rename doesn't bring the old name back
        await connections.listSessions(machineId: "m1")
        await settled()
        #expect(names() == ["Session Labels", "docs"])
        // nor does one after the attached session is attached again, which keeps its stream
        try await connections.attach(machineId: "m1", sessionId: "s1")
        await connections.listSessions(machineId: "m1")
        await settled()
        #expect(names() == ["Session Labels", "docs"])
        // once detached, the listing's name stands again, with no listing read since
        connections.detach(machineId: "m1", sessionId: "s1")
        #expect(names() == ["ngterm", "docs"])
        // an attach again starts from the listing's name, until its own stream says one
        try await connections.attach(machineId: "m1", sessionId: "s1")
        #expect(names() == ["ngterm", "docs"])
    }
}
