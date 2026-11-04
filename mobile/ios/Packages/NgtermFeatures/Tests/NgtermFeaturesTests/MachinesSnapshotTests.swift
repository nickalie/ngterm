import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of the Machines list in each state, and of its form.
@MainActor
@Suite("machines snapshots")
internal struct MachinesSnapshotTests {
    private static var sessions: [RemoteSession] {
        var waiting = fakeSession(id: "s1", name: "docs-site")
        waiting.status = "blocked"
        var working = fakeSession(id: "s2", name: "api-refactor")
        working.status = "active"
        var alsoWaiting = fakeSession(id: "s3", name: "release")
        alsoWaiting.status = "blocked"
        return [waiting, working, alsoWaiting, fakeSession(id: "s4", name: "shell")]
    }

    private let test = TestGraph()

    private func machine(_ id: String, _ label: String, host: String, port: UInt16 = 22) -> StoredMachine {
        StoredMachine(id: id, label: label, host: host, port: port, user: "nick", keyId: "fake-key")
    }

    // every link a machine can have, each told the connections as the core would tell them
    private func everyLink() throws -> MachinesModel {
        let connections = test.connections
        let machines = [
            machine("connected", "build-01", host: "build-01.lan"),
            machine("idle", "studio", host: "studio.lan"),
            machine("offline", "", host: "10.0.0.7", port: 2_222),
            machine("reconnecting", "nas", host: "nas.lan"),
            machine("unreachable", "pi", host: "pi.local"),
            machine("changed", "edge", host: "edge.example"),
        ]
        for machine in machines {
            try test.seed(machine)
        }
        // the core tells only of machines the app connects (the connections drop the rest); the connects themselves
        // never run before the golden is taken
        for machine in machines where machine.id != "offline" {
            connections.connect(machine.core).cancel()
        }
        connections.receive(.machineState(machineId: "connected", state: .connected))
        connections.receive(.tree(machineId: "connected", sessions: Self.sessions))
        connections.receive(.machineState(machineId: "idle", state: .connected))
        connections.receive(.tree(machineId: "idle", sessions: []))
        // the countdown is told from the test's clock (screen(_:)): a retry 5 s off
        connections.receive(.machineState(machineId: "reconnecting", state: .reconnecting(nextInMs: 5_000)))
        connections.receive(
            .machineState(machineId: "unreachable", state: .unreachable(reason: "pi.local:22: connection refused"))
        )
        connections.receive(.hostKeyChanged(machineId: "changed", pinned: "SHA256:old", offered: "SHA256:new"))
        return MachinesModel(graph: test.graph)
    }

    private func screen(_ model: MachinesModel) -> some View {
        NavigationStack {
            MachinesView(model: model, selection: nil) { _ in
                // a golden opens nothing
            }
            .navigationTitle("Machines")
        }
        .environment(\.fixedNow, test.clock.now)
    }

    @Test("no machines yet", arguments: ScreenVariant.allCases)
    internal func empty(variant: ScreenVariant) {
        defer { test.remove() }
        ScreenSnapshot.check(screen(MachinesModel(graph: test.graph)), named: "machines-empty", variant: variant)
    }

    @Test("a machine in each state", arguments: ScreenVariant.allCases)
    internal func states(variant: ScreenVariant) throws {
        defer { test.remove() }
        ScreenSnapshot.check(screen(try everyLink()), named: "machines", variant: variant)
    }

    @Test("connecting, asking about a host key, and waiting for a sign-in")
    internal func asking() throws {
        defer { test.remove() }
        try test.seed(machine("connecting", "build-01", host: "build-01.lan"))
        try test.seed(machine("asking", "studio", host: "studio.lan"))
        try test.seed(machine("signing", "signin", host: "signin.lan"))
        let model = MachinesModel(graph: test.graph)
        // the core tells only of machines the app connects; the connects never run before the golden is taken
        for machine in test.graph.machines.machines {
            test.connections.connect(machine.core).cancel()
        }
        test.connections.receive(.hostKeyUnknown(machineId: "asking", fingerprint: "SHA256:new"))
        let tailscale = FakeSignIn.tailscale
        test.connections.receive(.signInRequested(machineId: "signing", message: tailscale.message, url: tailscale.url))
        ScreenSnapshot.check(screen(model), named: "machines-asking", variant: .light)
    }

    @Test("offline, with a removal to undo and a change that failed")
    internal func notices() throws {
        defer { test.remove() }
        let model = try everyLink()
        test.graph.network.start()
        test.path.send(NetworkPath(isSatisfied: false, interfaces: []))
        model.remove(machineId: "idle")
        model.failure = "The machine couldn't be saved: the store could not be written: the disk is full"
        ScreenSnapshot.check(screen(model), named: "machines-notices", variant: .light)
    }

    @Test("no machines while offline")
    internal func emptyOffline() {
        defer { test.remove() }
        test.graph.network.start()
        test.path.send(NetworkPath(isSatisfied: false, interfaces: []))
        ScreenSnapshot.check(screen(MachinesModel(graph: test.graph)), named: "machines-empty-offline", variant: .light)
    }

    @Test("Add machine, Save tried with problems", arguments: ScreenVariant.allCases)
    internal func addForm(variant: ScreenVariant) async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let model = MachinesModel(graph: test.graph)
        ScreenSnapshot.check(
            MachineFormView(model: model, target: .add, showsProblems: true),
            named: "form-add",
            variant: variant
        )
    }

    @Test("Edit machine, with its pinned host key and two keys to choose from", arguments: ScreenVariant.allCases)
    internal func editForm(variant: ScreenVariant) async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        _ = try await test.graph.keys.rotate()
        try test.seed(
            StoredMachine(id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: key.id)
        )
        test.pins.set(machineId: "m1", fingerprint: "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E")
        let model = MachinesModel(graph: test.graph)
        ScreenSnapshot.check(
            MachineFormView(model: model, target: .edit("m1"), showsProblems: false),
            named: "form-edit",
            variant: variant
        )
    }
}
