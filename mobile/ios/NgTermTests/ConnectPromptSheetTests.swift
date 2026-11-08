import NgtermCoreFakes
import NgtermFeatures
import NgtermPlatform
import SwiftUI
import Testing
import UIKit

@testable import NgTerm

/// The sheets a connect's question shows in, over a window of the app's scene (a sheet needs one), on the fake core's
/// hosts (``FakeWorld``): which question shows, that the next one follows once it is answered, and what swiping one
/// away answers.
@MainActor
@Suite("the connect questions' sheets")
internal struct ConnectPromptSheetTests {
    private let directory = FileManager.default.temporaryDirectory.appending(path: "ConnectPromptSheetTests-\(UUID())")

    // waits up to 5 s for `condition`
    private static func until(_ condition: () -> Bool) async throws {
        var waited = 0
        while !condition(), waited < 100 {
            waited += 1
            try await Task.sleep(for: .milliseconds(50))
        }
        try #require(condition(), "timed out waiting")
    }

    // a machine of the fake world at `host`, signing in with the fake key
    private static func machine(_ id: String, host: String) -> StoredMachine {
        StoredMachine(id: id, label: id, host: host, port: StoredMachine.sshPort, user: "nick", keyId: "fake-key")
    }

    // the questions of `model` over a window of the app's scene
    private static func show(_ model: MachinesModel) throws -> (UIWindow, UIViewController) {
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: Color.clear.connectPrompts(model))
        window.rootViewController = host
        window.makeKeyAndVisible()
        return (window, host)
    }

    // swipes the sheet `host` shows away, as the user's swipe down ends it
    private static func swipeAway(_ host: UIViewController) throws {
        let sheet = try #require(host.presentedViewController)
        let presentation = try #require(sheet.presentationController)
        let delegate = presentation.delegate
        #expect(delegate?.presentationControllerShouldDismiss?(presentation) ?? true, "the sheet can be swiped away")
        delegate?.presentationControllerWillDismiss?(presentation)
        sheet.dismiss(animated: false)
        delegate?.presentationControllerDidDismiss?(presentation)
    }

    // whether `host` shows a sheet that has come up and isn't going (one question's sheet gives way to the next's)
    private static func showsSheet(_ host: UIViewController) -> Bool {
        guard let sheet = host.presentedViewController else {
            return false
        }
        return !sheet.isBeingPresented && !sheet.isBeingDismissed
    }

    // waits until the sheet `host` shows has gone and a sheet has come up again for the sign-in `model` asks for: a new
    // question is a new sheet
    private static func untilSignInSheet(_ model: MachinesModel, _ host: UIViewController) async throws {
        var hasGone = false
        try await until {
            hasGone = hasGone || !showsSheet(host)
            return hasGone && asksToSignIn(model) && showsSheet(host)
        }
    }

    // whether `model` asks to sign in
    private static func asksToSignIn(_ model: MachinesModel) -> Bool {
        guard case .signIn = model.prompt else {
            return false
        }
        return true
    }

    private static func link(_ model: MachinesModel, _ machineId: String) -> NgtermFeatures.Link? {
        model.items.first { $0.id == machineId }?.link
    }

    // the fake world's hosts, with stores in this test's own directory (removed even when setup fails)
    private func connecting(_ machines: [StoredMachine]) throws -> MachinesModel {
        let pins = MemoryHostKeyPins()
        let core = FakeMobileCore(hosts: pins)
        core.hostKeys = ["build-01.lan": FakeWorld.buildHostKey, "signin.lan": FakeWorld.signInHostKey]
        core.signIns = ["signin.lan": .tailscale]
        let graph = AppGraph(
            machines: MachineStore(directory: directory),
            settings: SettingsStore(directory: directory),
            keys: AppKeys(
                store: KeyStore(directory: directory),
                vault: MemoryPrivateKeys(),
                maker: CoreKeyMaker(),
                comment: "ngterm@test"
            ),
            pins: pins,
            stores: GraphStores(directory: directory),
            core: core,
            notices: FakeNotices()
        )
        for machine in machines {
            try graph.machines.save(machine)
        }
        let model = MachinesModel(graph: graph)
        for machine in machines {
            model.connect(machineId: machine.id)
        }
        return model
    }

    private func remove() {
        guard FileManager.default.fileExists(atPath: directory.path) else {
            return
        }
        do {
            try FileManager.default.removeItem(at: directory)
        } catch {
            Issue.record(error)
        }
    }

    @Test("a host key's sheet, then the sign-in's once the key is trusted, and none once the connect ends")
    internal func oneAfterAnother() async throws {
        defer { remove() }
        let model = try connecting([Self.machine("signin", host: "signin.lan")])
        let (window, host) = try Self.show(model)
        defer { window.isHidden = true }
        try await Self.until { model.prompt != nil && Self.showsSheet(host) }
        let asked = try #require(model.prompt)
        #expect(asked == .hostKey(machine: asked.machine, fingerprint: FakeWorld.signInHostKey))
        #expect(host.presentedViewController != nil, "the host key's question shows in a sheet")

        // trusted (a sheet's Trust and connect answers the same): the sign-in it then asks for shows in its place
        model.answerHostKey(machineId: "signin", fingerprint: FakeWorld.signInHostKey, trust: true)
        try await Self.untilSignInSheet(model, host)
        #expect(Self.asksToSignIn(model))
        #expect(Self.showsSheet(host), "the sign-in shows in a sheet")

        // the connect ends (a sheet's Don't connect answers the same): the sheet goes
        model.disconnect(machineId: "signin")
        try await Self.until { host.presentedViewController == nil }
        #expect(host.presentedViewController == nil, "the sheet goes with its question")
        #expect(model.prompt == nil)
    }

    @Test("swiping a host key's sheet away declines the key, and the connect stops")
    internal func hostKeySwiped() async throws {
        defer { remove() }
        let model = try connecting([Self.machine("build", host: "build-01.lan")])
        let (window, host) = try Self.show(model)
        defer { window.isHidden = true }
        try await Self.until { model.prompt != nil && Self.showsSheet(host) }
        try Self.swipeAway(host)
        try await Self.until { Self.link(model, "build") == .offline }
        #expect(Self.link(model, "build") == .offline)
        #expect(model.prompt == nil)
    }

    @Test("swiping a sign-in's sheet away doesn't connect")
    internal func signInSwiped() async throws {
        defer { remove() }
        let model = try connecting([Self.machine("signin", host: "signin.lan")])
        let (window, host) = try Self.show(model)
        defer { window.isHidden = true }
        try await Self.until { model.prompt != nil && Self.showsSheet(host) }
        model.answerHostKey(machineId: "signin", fingerprint: FakeWorld.signInHostKey, trust: true)
        try await Self.untilSignInSheet(model, host)
        #expect(Self.showsSheet(host), "the sign-in shows in a sheet")
        try Self.swipeAway(host)
        try await Self.until { Self.link(model, "signin") == .offline }
        #expect(Self.link(model, "signin") == .offline)
        #expect(model.prompt == nil)
    }
}
