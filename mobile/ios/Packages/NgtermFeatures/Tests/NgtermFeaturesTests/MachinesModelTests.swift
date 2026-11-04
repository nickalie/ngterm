import Foundation
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The Machines list on the fake core, as mobile/android's `MachinesViewModelTest` has it.
@MainActor
@Suite("the machines list")
internal struct MachinesModelTests {
    private let test = TestGraph()

    private func link(_ model: MachinesModel, _ machineId: String) -> Link? {
        model.items.first { $0.id == machineId }?.link
    }

    @Test("starting the graph connects every machine once and shows its sessions")
    internal func startConnectsAll() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        try test.seed(TestGraph.machine(keyId: key.id))
        try test.seed(TestGraph.machine(id: "m2", label: "studio", host: "studio.lan", keyId: key.id))
        test.core.sessions["m1"] = [fakeSession(id: "s1")]
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        test.graph.start()
        await Wait.until { model.items.allSatisfy { $0.link == .connected && $0.isListed } }
        #expect(model.items.map(\.machine.title) == ["build-01", "studio"])
        #expect(model.items[0].sessions == [fakeSession(id: "s1")])
        #expect(test.connects() == 2)
        #expect(test.path.isStarted)
    }

    @Test("a saved machine is stored and connected; edited, it connects anew")
    internal func save() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        let model = MachinesModel(graph: test.graph)
        #expect(!model.save(MachineDraft(host: "build 01", user: "nick", keyId: key.id), editing: nil))
        #expect(model.items.isEmpty)
        #expect(model.save(MachineDraft(host: "build-01.lan", user: "nick", keyId: key.id), editing: nil))
        await Wait.until { model.items.first?.link == .connected }
        let machine = try #require(model.items.first?.machine)
        #expect(machine.address == "nick@build-01.lan:22")
        var draft = MachineDraft(editing: machine)
        draft.port = "2222"
        #expect(model.save(draft, editing: machine.id))
        #expect(model.items.map(\.machine.port) == [2_222])
        await Wait.until { test.connects() == 2 }
    }

    @Test("a machine the store can't save says why and stays out of the list")
    internal func saveFails() throws {
        defer { test.remove() }
        // a file where the store's directory should be
        try Data().write(to: test.directory)
        let model = MachinesModel(graph: test.graph)
        #expect(!model.save(MachineDraft(host: "build-01.lan", user: "nick", keyId: "k1"), editing: nil))
        #expect(model.items.isEmpty)
        #expect(model.failure?.hasPrefix("The machine couldn't be saved") == true)
        model.dismissFailure()
        #expect(model.failure == nil)
        #expect(test.connects() == 0)
    }

    @Test("removing a machine forgets it and its pin until undone")
    internal func removeAndUndo() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        try test.seed(TestGraph.machine(id: "a", label: "build-01", host: "build-01.lan", keyId: key.id))
        try test.seed(TestGraph.machine(id: "b", label: "studio", host: "studio.lan", keyId: key.id))
        test.pins.set(machineId: "a", fingerprint: "SHA256:a")
        let model = MachinesModel(graph: test.graph)
        model.remove(machineId: "a")
        #expect(model.items.map(\.id) == ["b"])
        #expect(model.removal?.machine.id == "a")
        #expect(test.pins.pinned(machineId: "a") == nil)
        #expect(test.core.calls.contains(FakeCall("disconnect", ["a"])))
        model.undoRemove()
        #expect(model.items.map(\.id) == ["a", "b"])
        #expect(test.pins.pinned(machineId: "a") == "SHA256:a")
        #expect(model.removal == nil)
        await Wait.until { test.connects() == 1 }
        model.remove(machineId: "b")
        model.forgetRemoval()
        #expect(model.removal == nil)
        model.remove(machineId: "absent")
        #expect(model.removal == nil)
    }

    @Test("a first connect asks about the host key, and trusting it pins it; a later connect keeps the record")
    internal func hostKeyQuestion() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        try test.seed(TestGraph.machine(keyId: key.id))
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        await Wait.until { link(model, "m1") == .hostKeyUnknown(fingerprint: "SHA256:new") }
        let machine = try #require(model.items.first?.machine)
        #expect(model.prompt == .hostKey(machine: machine, fingerprint: "SHA256:new"))
        model.answerHostKey(machineId: "m1", fingerprint: "SHA256:new", trust: true)
        await Wait.until { link(model, "m1") == .connected }
        #expect(model.prompt == nil)
        #expect(test.pins.pinned(machineId: "m1") == "SHA256:new")
        #expect(model.pin(of: "m1") == "SHA256:new")
        // a later connect (Retry now, or Edit saved unchanged) sends the same record, so the core keeps the connection
        model.connect(machineId: "m1")
        await Wait.until { test.connects() == 2 }
        let records = test.core.calls.filter { $0.method == "connect" }.map(\.args)
        #expect(FakeCall("connect", records[0]) == FakeCall("connect", records[1]))
    }

    @Test("a host key sheet swiped away declines the key it asked about, and only that one")
    internal func dismissHostKey() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        try test.seed(TestGraph.machine(keyId: key.id))
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        await Wait.until { model.prompt != nil }
        let asked = try #require(model.prompt)
        let machine = asked.machine
        // a question no longer asked is left alone
        model.dismiss(.hostKey(machine: machine, fingerprint: "SHA256:other"))
        #expect(model.prompt == asked)
        model.dismiss(asked)
        await test.settled()
        #expect(link(model, "m1") == .offline)
        #expect(test.pins.all.isEmpty)
        #expect(test.core.calls.contains(FakeCall("answerHostKey", ["m1", "SHA256:new", false])))
    }

    @Test("a sign-in is asked after the host keys, and swiping it away doesn't connect")
    internal func signInPrompt() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        test.core.signIns["signin.lan"] = .tailscale
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        try test.seed(TestGraph.machine(id: "s", label: "signin", host: "signin.lan", keyId: key.id))
        try test.seed(TestGraph.machine(keyId: key.id))
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        await Wait.until { link(model, "s") != .connecting && link(model, "m1") != .connecting }
        guard case .hostKey(let first, _) = model.prompt else {
            Issue.record("no host key question: \(String(describing: model.prompt))")
            return
        }
        #expect(first.id == "m1")
        model.answerHostKey(machineId: "m1", fingerprint: "SHA256:new", trust: true)
        let signIn = try #require(model.prompt)
        #expect(
            signIn
                == .signIn(
                    machine: signIn.machine,
                    message: FakeSignIn.tailscale.message,
                    url: FakeSignIn.tailscale.url
                )
        )
        #expect(signIn.machine.id == "s")
        model.dismiss(signIn)
        #expect(link(model, "s") == .offline)
        #expect(test.core.waitingForSignIn.isEmpty)
        #expect(model.prompt == nil)
    }

    @Test("a changed host key stops until the pin is removed, then asks about the new key")
    internal func changedHostKey() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        test.pins.set(machineId: "m1", fingerprint: "SHA256:old")
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        try test.seed(TestGraph.machine(keyId: key.id))
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        await Wait.until { link(model, "m1") == .hostKeyChanged(pinned: "SHA256:old", offered: "SHA256:new") }
        #expect(test.pins.pinned(machineId: "m1") == "SHA256:old")
        #expect(model.prompt == nil)
        model.removePin(machineId: "m1")
        await Wait.until { link(model, "m1") == .hostKeyUnknown(fingerprint: "SHA256:new") }
        #expect(test.pins.pinned(machineId: "m1") == nil)
    }

    @Test("the device going offline shows on the list, and a new network nudges the core")
    internal func offline() {
        defer { test.remove() }
        let model = MachinesModel(graph: test.graph)
        test.graph.start()
        #expect(!model.isOffline)
        test.path.send(NetworkPath(isSatisfied: true, interfaces: ["en0"]))
        test.path.send(NetworkPath(isSatisfied: false, interfaces: []))
        #expect(model.isOffline)
        test.path.send(NetworkPath(isSatisfied: true, interfaces: ["en0"]))
        #expect(!model.isOffline)
        #expect(test.core.calls.contains(FakeCall("networkChanged")))
    }

    @Test("Disconnect closes a machine's connection, and the keys are the app's")
    internal func disconnectAndKeys() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        try test.seed(TestGraph.machine(keyId: key.id))
        let model = MachinesModel(graph: test.graph)
        #expect(model.keys == [key])
        #expect(model.machine(id: "m1")?.keyId == key.id)
        model.connect(machineId: "m1")
        await Wait.until { link(model, "m1") == .connected }
        model.disconnect(machineId: "m1")
        #expect(link(model, "m1") == .offline)
        #expect(model.pin(of: "m1") == nil)
    }
}
