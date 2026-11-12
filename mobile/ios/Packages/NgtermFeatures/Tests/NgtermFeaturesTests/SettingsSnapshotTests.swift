import Foundation
import NgtermCore
import SnapshotTesting
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of Settings in a compact and a regular width, and of a new key's public half with the switch under it.
@MainActor
@Suite("settings snapshots")
internal struct SettingsSnapshotTests {
    private let test = TestGraph()

    // two keys (one made here, one imported), two machines on the first, one of them trusted with the clipboard, and
    // the question about notifying not answered yet
    private func world() async throws -> SettingsModel {
        let made = try await test.seedKey()
        var imported = Data(FakeKeyMaker.encryptedKey.utf8)
        _ = try await test.graph.keys.importKey(privateKey: &imported, passphrase: FakeKeyMaker.passphrase)
        try test.seed(TestGraph.machine(keyId: made.id))
        try test.seed(TestGraph.machine(id: "m2", label: "nas", host: "nas.lan", keyId: made.id))
        let pane = PaneRef(machineId: "m2", sessionId: "s1", paneId: "P1")
        test.core.emit(
            .clipboardReadRequest(
                pane: pane,
                readId: 1,
                clipboard: .standard,
                listOnly: false,
                mimes: ["text/plain"],
                list: false,
                granted: false,
                canRemember: false
            )
        )
        await test.settled()
        let reads = test.graph.clipboardReads
        reads.allowAlways(try #require(reads.pending.first))
        let model = SettingsModel(graph: test.graph)
        await model.readPermission()
        return model
    }

    @Test("Settings in a compact and a regular width", arguments: SettingsShot.all)
    internal func settings(shot: SettingsShot) async throws {
        defer { test.remove() }
        let view = SettingsView(model: try await world(), coreVersion: "0.4.0")
            .environment(\.colorScheme, shot.colorScheme)
            .environment(\.dynamicTypeSize, shot.typeSize)
            .environment(\.horizontalSizeClass, shot.isRegular ? .regular : .compact)
        assertSnapshot(
            of: view,
            as: .image(
                precision: 1,
                perceptualPrecision: 0.98,
                layout: .fixed(width: shot.size.width, height: shot.size.height),
                traits: shot.traits
            ),
            named: "\(shot.testDescription)-on-\(SnapshotDevice.current)"
        )
    }

    @Test("a new key's public half, with the machines still to switch", arguments: ScreenVariant.allCases)
    internal func newKey(variant: ScreenVariant) async throws {
        defer { test.remove() }
        let model = try await world()
        await model.rotate()
        let item = try #require(model.keys.first { $0.id == model.newKey })
        let page = NavigationStack {
            PublicKeyView(key: item.key) {
                SwitchMachines(item: item, onUse: ScreenSnapshot.ignore)
            }
        }
        ScreenSnapshot.check(page, named: "settings-new-key", variant: variant)
    }
}
