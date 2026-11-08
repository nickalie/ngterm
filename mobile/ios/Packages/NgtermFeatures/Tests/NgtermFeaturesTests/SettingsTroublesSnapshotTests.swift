import Foundation
import SnapshotTesting
import SwiftUI
import Testing

@testable import NgtermFeatures

/// A golden of Settings when things went wrong (mobile/android's `SettingsScreenshotTest`): notifications turned off in
/// the system's Settings, a key that couldn't be made, no machine trusted with the clipboard, and a change the store
/// couldn't save, said at the foot until dismissed.
@MainActor
@Suite("settings troubles snapshots")
internal struct SettingsTroublesSnapshotTests {
    private static let shot = SettingsShot(isRegular: false, isDark: false, isAccessibility: false)

    // a directory of a fixed name: the failed save's words name it
    private let test = TestGraph(
        directory: FileManager.default.temporaryDirectory.appending(path: "NgtermFeaturesTests-settings-troubles")
    )

    @Test("notifications denied, a failed rotation, no trusted machine and a failed save")
    internal func troubles() async throws {
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: test.directory.path)
            test.remove()
        }
        let made = try await test.seedKey()
        try test.seed(TestGraph.machine(keyId: made.id))
        test.notices.answer = .denied
        let model = SettingsModel(graph: test.graph)
        await model.readPermission()
        test.maker.generateFailure = .Invalid(reason: "no randomness")
        await model.rotate()
        #expect(model.work.rotateFailure != nil)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: test.directory.path)
        model.change { $0.textSize = 20 }
        #expect(model.failure != nil)
        let view = SettingsView(model: model, coreVersion: "0.4.0")
            .environment(\.colorScheme, Self.shot.colorScheme)
            .environment(\.dynamicTypeSize, Self.shot.typeSize)
            .environment(\.horizontalSizeClass, .compact)
        assertSnapshot(
            of: view,
            as: .image(
                precision: 1,
                perceptualPrecision: 0.98,
                layout: .fixed(width: Self.shot.size.width, height: Self.shot.size.height),
                traits: Self.shot.traits
            ),
            named: "\(Self.shot.testDescription)-on-\(SnapshotDevice.current)"
        )
    }
}
