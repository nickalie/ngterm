import Foundation
import NgtermCore
import NgtermPlatform
import Testing

/// The machines' store: its JSON round trip and the changes Android's `MachineStore` makes.
@MainActor
@Suite("the machine store")
internal struct MachineStoreTests {
    private static let box = StoredMachine(
        id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1"
    )
    private static let board = StoredMachine(
        id: "m2", label: "", host: "10.0.0.7", port: 2_222, user: "pi", keyId: "k1"
    )
    private static let nas = StoredMachine(
        id: "m3", label: "nas", host: "nas.lan", port: 22, user: "admin", keyId: "k2"
    )

    @Test("no file is no machines")
    internal func startsEmpty() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        #expect(MachineStore(directory: directory.url).machines.isEmpty)
        #expect(try directory.names().isEmpty)
    }

    @Test("the machines round-trip through the file, in the order they were added")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = MachineStore(directory: directory.url)
        try store.save(Self.box)
        try store.save(Self.board)
        #expect(store.machines == [Self.box, Self.board])
        #expect(MachineStore(directory: directory.url).machines == [Self.box, Self.board])
        #expect(try directory.names() == [MachineStore.fileName])
    }

    @Test("saving a machine with a known id replaces it in place")
    internal func saveReplaces() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = MachineStore(directory: directory.url)
        try store.save(Self.box)
        try store.save(Self.board)
        var renamed = Self.box
        renamed.label = "renamed"
        try store.save(renamed)
        #expect(MachineStore(directory: directory.url).machines == [renamed, Self.board])
        #expect(store.machine(id: "m1") == renamed)
        #expect(store.machine(id: "absent") == nil)
    }

    @Test("a removal undone puts the machine back where it was, or at the end of a shorter list")
    internal func deleteAndRestore() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = MachineStore(directory: directory.url)
        for machine in [Self.box, Self.board, Self.nas] {
            try store.save(machine)
        }
        try store.delete(id: "m2")
        try store.delete(id: "absent")
        #expect(store.machines == [Self.box, Self.nas])
        try store.restore(Self.board, at: 1)
        #expect(MachineStore(directory: directory.url).machines == [Self.box, Self.board, Self.nas])
        try store.delete(id: "m3")
        try store.delete(id: "m2")
        try store.restore(Self.nas, at: 2)
        #expect(store.machines == [Self.box, Self.nas])
    }

    @Test("a key used for every machine points each of them at it")
    internal func useKeyForEveryMachine() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = MachineStore(directory: directory.url)
        try store.save(Self.box)
        try store.save(Self.nas)
        try store.useKey("k9")
        #expect(MachineStore(directory: directory.url).machines.map(\.keyId) == ["k9", "k9"])
    }

    @Test("a stored machine without a label or a port has none and the ssh port")
    internal func lenientMachine() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let json = #"[{"id":"m1","host":"box.example","user":"me","keyId":"k1"}]"#
        try Data(json.utf8).write(to: directory.file(MachineStore.fileName))
        let machine = try #require(MachineStore(directory: directory.url).machines.first)
        #expect(machine.label.isEmpty)
        #expect(machine.port == StoredMachine.sshPort)
        #expect(machine.title == "box.example")
        #expect(machine.address == "me@box.example:22")
    }

    @Test("the core's record carries the title as its label")
    internal func coreRecord() {
        #expect(
            Self.board.core
                == Machine(id: "m2", label: "10.0.0.7", host: "10.0.0.7", port: 2_222, user: "pi", keyId: "k1")
        )
        #expect(Self.box.core.label == "box")
    }
}
