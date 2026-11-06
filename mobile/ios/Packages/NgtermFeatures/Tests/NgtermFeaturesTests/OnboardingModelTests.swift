import Foundation
import NgtermFeatures
import NgtermPlatform
import Testing

/// The first run, as mobile/android's `OnboardingViewModelTest` has it.
@MainActor
@Suite("the first run")
internal struct OnboardingModelTests {
    private let test = TestGraph()

    @Test("an app with a key skips the first run")
    internal func hasKey() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        #expect(OnboardingModel(keys: test.graph.keys).step == .done)
    }

    @Test("a process that couldn't read its files as it started reads them again before the first run is decided")
    internal func readAgain() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        try test.seed(TestGraph.machine(keyId: key.id))
        try test.graph.settings.update { $0.theme = .dark }
        try test.graph.notificationPrefs.set(.done, isOn: false)
        let names = [KeyStore.fileName, MachineStore.fileName, SettingsStore.fileName, NotificationPrefsStore.fileName]
        let files = names.map { test.directory.appending(path: $0).path }
        // before the device's first unlock: the files are there but can't be read
        for file in files {
            try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file)
        }
        let early = TestGraph(directory: test.directory)
        #expect(early.graph.keys.keys.isEmpty)
        for file in files {
            try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file)
        }
        early.graph.readUnreadStores()
        #expect(early.graph.keys.keys == [key])
        #expect(early.graph.machines.machines.map(\.id) == ["m1"])
        #expect(early.graph.settings.settings.theme == .dark)
        #expect(early.graph.notificationPrefs.prefs.off == [.done])
        #expect(OnboardingModel(keys: early.graph.keys).step == .done)
    }

    @Test("creating a key shows its public half, then finishes")
    internal func create() async {
        defer { test.remove() }
        let model = OnboardingModel(keys: test.graph.keys)
        #expect(model.step == .start)
        await model.create()
        guard case .keyReady(let key) = model.step else {
            Issue.record("no key: \(model.step)")
            return
        }
        #expect(key.fingerprint == FakeKeyMaker.fingerprint)
        #expect(test.maker.generated == 1)
        model.finish()
        #expect(model.step == .done)
        #expect(test.graph.keys.keys == [key])
    }

    @Test("a create while another is under way, or past the welcome, makes nothing")
    internal func createOnce() async {
        defer { test.remove() }
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        await model.create()
        #expect(test.maker.generated == 0)
        #expect(model.step == .importing(isBusy: false, problem: nil))
    }

    @Test("an import says what it needs until the passphrase opens the key")
    internal func importSteps() async {
        defer { test.remove() }
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        var blank = Data("   ".utf8)
        await model.importKey(privateKey: &blank, passphrase: "")
        #expect(model.step == .importing(isBusy: false, problem: .empty))
        var notAKey = Data("not a key".utf8)
        await model.importKey(privateKey: &notAKey, passphrase: "")
        #expect(model.step == .importing(isBusy: false, problem: .notAKey(reason: "unknown key format")))
        var locked = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &locked, passphrase: "")
        #expect(model.step == .importing(isBusy: false, problem: .needsPassphrase))
        var wrong = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &wrong, passphrase: "wrong")
        #expect(model.step == .importing(isBusy: false, problem: .wrongPassphrase))
        var bytes = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &bytes, passphrase: FakeKeyMaker.passphrase)
        guard case .keyReady(let key) = model.step else {
            Issue.record("no key: \(model.step)")
            return
        }
        #expect(key.isImported)
        #expect(bytes.allSatisfy { $0 == 0 })
    }

    @Test("back from the import returns to the welcome, and an unreadable file says so")
    internal func back() {
        defer { test.remove() }
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        model.unreadable()
        #expect(model.step == .importing(isBusy: false, problem: .unreadable))
        model.back()
        #expect(model.step == .start)
        model.back()
        #expect(model.step == .start)
    }

    @Test("every import problem has words, and only the passphrase's are about the passphrase")
    internal func problemWords() {
        let all: [ImportProblem] = [
            .needsPassphrase, .wrongPassphrase, .notAKey(reason: "r"), .notStored(reason: "r"), .empty, .unreadable,
        ]
        for problem in all {
            #expect(!problem.message.isEmpty)
        }
        #expect(all.filter(\.isAboutPassphrase) == [.needsPassphrase, .wrongPassphrase])
    }
}
