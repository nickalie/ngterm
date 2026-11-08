import Foundation
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// Goldens of the first run: the welcome (also while an import goes on after Back), the import with a problem and while
/// it runs, the public key, keys that can't be read, and the end.
@MainActor
@Suite("first run snapshots")
internal struct OnboardingSnapshotTests {
    private let test = TestGraph()

    private func model() -> OnboardingModel {
        OnboardingModel(keys: test.graph.keys)
    }

    @Test("the welcome", arguments: ScreenVariant.allCases)
    internal func welcome(variant: ScreenVariant) {
        defer { test.remove() }
        ScreenSnapshot.check(
            OnboardingView(model: model(), onFinish: ScreenSnapshot.ignore), named: "welcome", variant: variant
        )
    }

    @Test("the welcome after a key couldn't be made")
    internal func welcomeFailed() async {
        defer { test.remove() }
        test.maker.generateFailure = .Invalid(reason: "the system's random source failed")
        let model = model()
        await model.create()
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "welcome-failed", variant: .light
        )
    }

    @Test("the welcome while an import goes on after Back: Create waits, and says why")
    internal func welcomeImporting() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = model()
        model.startImport()
        let reading = Task {
            var bytes = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &bytes, passphrase: FakeKeyMaker.passphrase)
        }
        await Wait.until { model.isImporting }
        model.back()
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "welcome-importing", variant: .light
        )
        held.signal()
        await reading.value
    }

    @Test("keys that can't be read: why, with Retry", arguments: ScreenVariant.allCases)
    internal func keysUnread(variant: ScreenVariant) async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        let file = test.directory.appending(path: KeyStore.fileName).path
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: file)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file)
        }
        let early = TestGraph(directory: test.directory)
        let model = OnboardingModel(keys: early.graph.keys)
        guard case .readFailed = model.step else {
            Issue.record("not a read failure: \(model.step)")
            return
        }
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "keys-unread", variant: variant
        )
    }

    @Test("the import, saying what was wrong with the key", arguments: ScreenVariant.allCases)
    internal func importProblem(variant: ScreenVariant) async {
        defer { test.remove() }
        let model = model()
        model.startImport()
        var bytes = Data("not a key".utf8)
        await model.importKey(privateKey: &bytes, passphrase: "")
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "import", variant: variant
        )
    }

    @Test("the import, saying the key needs its passphrase")
    internal func importPassphrase() async {
        defer { test.remove() }
        let model = model()
        model.startImport()
        var bytes = Data(FakeKeyMaker.encryptedKey.utf8)
        await model.importKey(privateKey: &bytes, passphrase: "")
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "import-passphrase", variant: .light
        )
    }

    @Test("the import while the key is read: the action busy, the screen not taking input")
    internal func importBusy() async {
        defer { test.remove() }
        let held = test.maker.holdImports()
        let model = model()
        model.startImport()
        let reading = Task {
            var bytes = Data(FakeKeyMaker.encryptedKey.utf8)
            await model.importKey(privateKey: &bytes, passphrase: FakeKeyMaker.passphrase)
        }
        await Wait.until { model.step == .importing(isBusy: true, problem: nil) }
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "import-busy", variant: .light
        )
        held.signal()
        await reading.value
    }

    @Test("the first run once it is over shows nothing of it")
    internal func done() async throws {
        defer { test.remove() }
        _ = try await test.seedKey()
        ScreenSnapshot.check(
            OnboardingView(model: model(), onFinish: ScreenSnapshot.ignore), named: "onboarding-done", variant: .light
        )
    }

    @Test("the public key at the end of the first run", arguments: ScreenVariant.allCases)
    internal func publicKey(variant: ScreenVariant) async {
        defer { test.remove() }
        let model = model()
        await model.create()
        ScreenSnapshot.check(
            OnboardingView(model: model, onFinish: ScreenSnapshot.ignore), named: "public-key", variant: variant
        )
    }
}
