import Foundation
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// Settings' keys, as mobile/android's `SettingsViewModelTest` has them: rotate, then use the new key for every
/// machine; import; and a switch reaching the record of a connected machine.
@MainActor
@Suite("settings' keys")
internal struct SettingsKeysTests {
    private let test = TestGraph()

    @Test("a rotated key is new, and the machines keep the old one until they're switched to it")
    internal func rotate() async throws {
        defer { test.remove() }
        let old = try await test.seedKey()
        try test.seed(TestGraph.machine(keyId: old.id))
        try test.seed(TestGraph.machine(id: "m2", label: "nas", host: "nas.lan", keyId: old.id))
        let model = SettingsModel(graph: test.graph)
        #expect(model.keys.map(\.usedBy) == [["build-01", "nas"]])
        #expect(model.keys.first?.isEveryMachine == true)
        await model.rotate()
        let fresh = try #require(model.newKey)
        #expect(!model.work.isRotating)
        let item = try #require(model.keys.first { $0.id == fresh })
        #expect(item.usedBy.isEmpty)
        #expect(!item.isEveryMachine)
        #expect(!item.key.isImported)
        // the private half is in the vault, never in what Settings shows
        #expect(test.vault.privateKey(keyId: fresh) != nil)
        model.newKeyShown()
        #expect(model.newKey == nil)
        model.useEverywhere(fresh)
        #expect(model.keys.first { $0.id == fresh }?.usedBy == ["build-01", "nas"])
        #expect(model.keys.first { $0.id == fresh }?.isEveryMachine == true)
        #expect(model.keys.first { $0.id == old.id }?.usedBy.isEmpty == true)
        #expect(MachineStore(directory: test.directory).machines.allSatisfy { $0.keyId == fresh })
    }

    @Test("a rotation that fails says why and makes nothing")
    internal func rotateFails() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        test.maker.generateFailure = .Invalid(reason: "no randomness")
        let model = SettingsModel(graph: test.graph)
        await model.rotate()
        #expect(model.work.rotateFailure == "no randomness")
        #expect(!model.work.isRotating)
        #expect(model.newKey == nil)
        #expect(model.keys.count == 1)
        test.maker.generateFailure = nil
        await model.rotate()
        #expect(model.work.rotateFailure == nil)
        #expect(model.keys.count == 2)
    }

    @Test("an import says what was wrong, then lands as a new key")
    internal func importSteps() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let model = SettingsModel(graph: test.graph)
        model.startImport()
        var blank = Data("  \n".utf8)
        await model.importKey(privateKey: &blank, passphrase: "")
        #expect(model.work.importProblem == .empty)
        var wrong = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &wrong, passphrase: "wrong")
        #expect(model.work.importProblem == .wrongPassphrase)
        #expect(wrong.allSatisfy { $0 == 0 })
        var right = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &right, passphrase: FakeKeyMaker.passphrase)
        let fresh = try #require(model.newKey)
        #expect(model.work.importProblem == nil)
        #expect(!model.work.isImporting)
        #expect(model.keys.first { $0.id == fresh }?.key.isImported == true)
        // opening the import again starts clean
        model.unreadable()
        #expect(model.work.importProblem == .unreadable)
        model.startImport()
        #expect(model.work.importProblem == nil)
    }

    @Test("an import while one runs starts no second one")
    internal func importOnce() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let held = test.maker.holdImports()
        let model = SettingsModel(graph: test.graph)
        let first = Task {
            var key = Data("-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END".utf8)
            await model.importKey(privateKey: &key, passphrase: "")
        }
        await Wait.until { model.work.isImporting }
        // Back, then Import a key again, while the first still runs
        model.startImport()
        #expect(model.work.isImporting)
        var second = Data("-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END".utf8)
        await model.importKey(privateKey: &second, passphrase: "")
        #expect(second.allSatisfy { $0 == 0 })
        held.signal()
        await first.value
        #expect(!model.work.isImporting)
        #expect(model.keys.count == 2)
    }

    @Test("switching keys reaches a connected machine's record and keeps its connection and sessions")
    internal func switchConnected() async throws {
        defer { test.remove() }
        let old = try await test.seedKey()
        let machine = TestGraph.machine(keyId: old.id)
        try test.seed(machine)
        test.core.sessions["m1"] = [fakeSession(id: "s1")]
        await test.connections.connect(machine.core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        let fresh = try await test.graph.keys.rotate()
        SettingsModel(graph: test.graph).useEverywhere(fresh.id)
        #expect(test.core.calls.contains(FakeCall("useKey", ["m1", fresh.id])))
        #expect(test.connections.links["m1"] == .connected)
        #expect(test.connections.attached["m1"] == ["s1"])
        #expect(!test.core.calls.contains { $0.method == "disconnect" || $0.method == "detachSession" })
    }
}
