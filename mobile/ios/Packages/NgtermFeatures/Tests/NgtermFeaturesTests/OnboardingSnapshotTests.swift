import Foundation
import Testing

@testable import NgtermFeatures

/// Goldens of the first run: the welcome, the import with a problem, and the public key.
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
