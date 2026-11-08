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

    @Test("keys that can't be read don't show a clean first run")
    internal func unreadableKeys() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let file = test.directory.appending(path: KeyStore.fileName).path
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file)
        }
        let early = TestGraph(directory: test.directory)
        // RootView reads the stores again before deciding the first run; the file still can't be read
        early.graph.readUnreadStores()
        let model = OnboardingModel(keys: early.graph.keys)
        // the app has a key it can't read: neither a clean welcome (which invites a second key) nor done
        #expect(model.step != .start)
        #expect(model.step != .done)
    }

    @Test("keys that can't be read say why, make no key, and Retry reads them once they can be")
    internal func retryAfterReadFailure() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        let file = test.directory.appending(path: KeyStore.fileName).path
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file)
        }
        let early = TestGraph(directory: test.directory)
        let model = OnboardingModel(keys: early.graph.keys)
        guard case .readFailed(let reason) = model.step else {
            Issue.record("not a read failure: \(model.step)")
            return
        }
        #expect(!reason.isEmpty)
        // nothing makes or brings a key over keys it can't read
        await model.create()
        model.startImport()
        #expect(model.step == .readFailed(reason: reason))
        #expect(early.maker.generated == 0)
        model.retry()
        #expect(model.step == .readFailed(reason: reason), "still unreadable")
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file)
        model.retry()
        #expect(model.step == .done)
        #expect(early.graph.keys.keys == [key])
        // a retry past the failure does nothing
        model.retry()
        #expect(model.step == .done)
    }

    @Test("a retry that finds the keys' file gone shows the welcome")
    internal func retryFindsNoKeys() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let url = test.directory.appending(path: KeyStore.fileName)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: url.path)
        let early = TestGraph(directory: test.directory)
        let model = OnboardingModel(keys: early.graph.keys)
        try FileManager.default.removeItem(at: url)
        model.retry()
        #expect(model.step == .start)
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

    @Test("Back and Import again while an import runs starts no second import")
    internal func importOnceThroughBack() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        let first = Task {
            var key = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &key, passphrase: FakeKeyMaker.passphrase)
        }
        await Wait.until { model.step == .importing(isBusy: true, problem: nil) }
        // Back, then Import a key again, while the first still runs
        model.back()
        model.startImport()
        let second = Task {
            var key = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &key, passphrase: FakeKeyMaker.passphrase)
            return key
        }
        held.signal()
        held.signal()
        await first.value
        let secondBytes = await second.value
        #expect(secondBytes.allSatisfy { $0 == 0 })
        #expect(test.graph.keys.keys.count == 1)
    }

    @Test("Back and Create while an import runs makes no second key")
    internal func createDuringImport() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        let first = Task {
            var key = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &key, passphrase: FakeKeyMaker.passphrase)
        }
        await Wait.until { model.step == .importing(isBusy: true, problem: nil) }
        model.back()
        await model.create()
        held.signal()
        await first.value
        #expect(test.maker.generated == 0)
        #expect(test.graph.keys.keys.count == 1)
    }

    @Test("an import that goes on through Back shows on the welcome, and its key lands there")
    internal func importLandsOnWelcome() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        let first = Task {
            var key = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &key, passphrase: FakeKeyMaker.passphrase)
        }
        await Wait.until { model.isImporting }
        model.back()
        #expect(model.step == .start)
        #expect(model.isImporting)
        held.signal()
        await first.value
        #expect(!model.isImporting)
        guard case .keyReady(let key) = model.step else {
            Issue.record("no key: \(model.step)")
            return
        }
        #expect(key.isImported)
    }

    @Test("an import that fails after Back leaves the welcome ready, its problem unsaid")
    internal func importFailsOnWelcome() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = OnboardingModel(keys: test.graph.keys)
        model.startImport()
        let first = Task {
            var key = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &key, passphrase: "wrong")
        }
        await Wait.until { model.isImporting }
        model.back()
        held.signal()
        await first.value
        #expect(model.step == .start)
        #expect(!model.isImporting)
        // the welcome is ready again: Create makes a key
        await model.create()
        #expect(test.maker.generated == 1)
    }

    @Test("the import doesn't open past the welcome")
    internal func importOnlyFromWelcome() async {
        defer { test.remove() }
        let model = OnboardingModel(keys: test.graph.keys)
        await model.create()
        guard case .keyReady = model.step else {
            Issue.record("no key: \(model.step)")
            return
        }
        model.startImport()
        guard case .keyReady = model.step else {
            Issue.record("the import opened over the key: \(model.step)")
            return
        }
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
