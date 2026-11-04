import NgtermDesign
import SnapshotTesting
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of the frame: a phone and a tablet, light and dark, and the phone at an accessibility text size.
@MainActor
@Suite("scaffold snapshots")
internal struct ScaffoldSnapshotTests {
    private func scaffold() -> some View {
        let model = ScaffoldModel(windows: SessionWindows())
        model.machine = "build-01"
        return AppScaffold(model: model) {
            List {
                MachineRow(name: "build-01", address: "nick@build-01.lan:22", tone: .online) {
                    AgentStatusBadge(status: "blocked")
                }
                MachineRow(name: "nas", address: "admin@nas.local:2222", tone: .offline, detail: "Not connected")
            }
        } sessions: { machine in
            List {
                SessionRow(name: "docs-site", line: "Rewrite the install guide", isAttached: true) {
                    AgentStatusBadge(status: "blocked")
                }
                SessionRow(name: "api-refactor", line: "~/src/api") {
                    AgentStatusBadge(status: "active")
                }
            }
            .navigationTitle(machine)
        } session: { _ in
            EmptyView()
        } settings: {
            EmptyView()
        } onboarding: {
            EmptyView()
        }
    }

    @Test("the frame on a phone and a tablet", arguments: ScaffoldShot.all)
    internal func frame(shot: ScaffoldShot) {
        let view = scaffold()
            .environment(\.colorScheme, shot.isDark ? .dark : .light)
            .environment(\.dynamicTypeSize, shot.isAccessibility ? .accessibility3 : .large)
            .environment(\.horizontalSizeClass, shot.sizeClass)
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
}
