import Foundation
import NgtermPlatform
import Testing

/// How the stores keep their files: a corrupt file set aside, an unreadable one left alone, every write atomic and
/// under the file protection the background refresh needs.
@MainActor
@Suite("the stores' files")
internal struct StoreFileTests {
    private static let box = StoredMachine(
        id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1"
    )
    private static let corrupt = Data("{ not json".utf8)

    @Test("a corrupt machines file reads as none, keeps a .bad copy, and the next save starts afresh")
    internal func corruptMachines() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Self.corrupt.write(to: directory.file(MachineStore.fileName))
        let store = MachineStore(directory: directory.url)
        #expect(store.machines.isEmpty)
        #expect(try Data(contentsOf: directory.file("\(MachineStore.fileName).bad")) == Self.corrupt)
        #expect(try directory.names() == ["\(MachineStore.fileName).bad"])
        try store.save(Self.box)
        #expect(MachineStore(directory: directory.url).machines == [Self.box])
        #expect(try Data(contentsOf: directory.file("\(MachineStore.fileName).bad")) == Self.corrupt)
    }

    @Test("a corrupt settings file reads as the defaults and keeps a .bad copy")
    internal func corruptSettings() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Self.corrupt.write(to: directory.file(SettingsStore.fileName))
        #expect(SettingsStore(directory: directory.url).settings == AppSettings())
        #expect(try Data(contentsOf: directory.file("\(SettingsStore.fileName).bad")) == Self.corrupt)
    }

    @Test("a corrupt file whose copy can't be written is left alone until it can be: no save writes over it")
    internal func corruptWithoutCopy() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let file = directory.file(MachineStore.fileName)
        let badCopy = directory.file("\(MachineStore.fileName).bad")
        try Self.corrupt.write(to: file)
        // a directory with something in it where the copy goes: nothing can be renamed over it
        try FileManager.default.createDirectory(at: badCopy, withIntermediateDirectories: false)
        try Data().write(to: badCopy.appending(path: "taken"))
        let store = MachineStore(directory: directory.url)
        #expect(store.machines.isEmpty)
        #expect(throws: StoreError.self) {
            try store.save(Self.box)
        }
        #expect(store.machines.isEmpty)
        #expect(try Data(contentsOf: file) == Self.corrupt)
        // once the copy can be written, the next change keeps the bytes first, then writes afresh
        try FileManager.default.removeItem(at: badCopy)
        try store.save(Self.box)
        #expect(store.machines == [Self.box])
        #expect(try Data(contentsOf: badCopy) == Self.corrupt)
        #expect(MachineStore(directory: directory.url).machines == [Self.box])
    }

    @Test("a write that can't finish leaves the old file whole: it is a new file renamed over the old one")
    internal func atomicWrite() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = MachineStore(directory: directory.url)
        try store.save(Self.box)
        let before = try Data(contentsOf: directory.file(MachineStore.fileName))
        // the file itself stays writable: only a write in place could change it now
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: directory.url.path)
        var renamed = Self.box
        renamed.label = "renamed"
        #expect(throws: StoreError.self) {
            try store.save(renamed)
        }
        #expect(store.machines == [Self.box])
        #expect(try Data(contentsOf: directory.file(MachineStore.fileName)) == before)
    }

    @Test("a write leaves only the store's file behind")
    internal func noLeftovers() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let machines = MachineStore(directory: directory.url)
        let settings = SettingsStore(directory: directory.url)
        for port: UInt16 in [22, 2_222, 22] {
            var machine = Self.box
            machine.port = port
            try machines.save(machine)
            try settings.update { $0.keepScreenOn.toggle() }
        }
        #expect(try directory.names() == [MachineStore.fileName, SettingsStore.fileName].sorted())
    }

    @Test("a file that can't be read is left alone: the store starts empty and refuses to write over it")
    internal func unreadableFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try MachineStore(directory: directory.url).save(Self.box)
        let file = directory.file(MachineStore.fileName)
        let before = try Data(contentsOf: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file.path)
        let store = MachineStore(directory: directory.url)
        #expect(store.machines.isEmpty)
        #expect(throws: StoreError.self) {
            try store.delete(id: "m1")
        }
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        #expect(try Data(contentsOf: file) == before)
        #expect(try directory.names() == [MachineStore.fileName])
        // readable again, the next change starts from what the file holds
        try store.save(StoredMachine(id: "m2", label: "", host: "pi.lan", port: 22, user: "pi", keyId: "k1"))
        #expect(store.machines.map(\.id) == ["m1", "m2"])
    }

    @Test("a file that couldn't be read is read again on asking once it can be, and only then")
    internal func readAgain() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try MachineStore(directory: directory.url).save(Self.box)
        let file = directory.file(MachineStore.fileName)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file.path)
        let store = MachineStore(directory: directory.url)
        store.readIfUnread()
        #expect(store.machines.isEmpty, "still unreadable")
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        store.readIfUnread()
        #expect(store.machines.map(\.id) == ["m1"])
        // read, the store keeps what it has: a file changed behind it isn't read again
        try MachineStore(directory: directory.url).delete(id: "m1")
        store.readIfUnread()
        #expect(store.machines.map(\.id) == ["m1"])
    }

    @Test("the stores create their directory, and write under completeUntilFirstUserAuthentication")
    internal func fileProtection() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let nested = directory.url.appending(path: "Application Support")
        try MachineStore(directory: nested).save(Self.box)
        try SettingsStore(directory: nested).update { $0.theme = .dark }
        for name in [MachineStore.fileName, SettingsStore.fileName] {
            let values = try nested.appending(path: name).resourceValues(forKeys: [.fileProtectionKey])
            #expect(values.fileProtection == .completeUntilFirstUserAuthentication, "\(name)")
        }
    }

    @Test("no store file leaves the device in a backup, written again or set aside")
    internal func excludedFromBackup() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Self.corrupt.write(to: directory.file(SettingsStore.fileName))
        let settings = SettingsStore(directory: directory.url)
        let machines = MachineStore(directory: directory.url)
        for port: UInt16 in [22, 2_222] {
            var machine = Self.box
            machine.port = port
            try machines.save(machine)
            try settings.update { $0.keepScreenOn.toggle() }
        }
        try KeyStore(directory: directory.url)
            .add(AppKey(id: "k1", authorizedKey: "ssh-ed25519 AAAA", fingerprint: "SHA256:k1", isImported: false))
        let names = [MachineStore.fileName, SettingsStore.fileName, "\(SettingsStore.fileName).bad", KeyStore.fileName]
        #expect(try directory.names() == names.sorted())
        for name in names {
            let values = try directory.file(name).resourceValues(forKeys: [.isExcludedFromBackupKey])
            #expect(values.isExcludedFromBackup == true, "\(name)")
        }
    }
}
