import NgtermDesign
import SnapshotTesting
import SwiftUI
import Testing

/// Goldens of every component, light and dark, and at an accessibility text size.
@MainActor
@Suite("component snapshots")
internal struct ComponentSnapshotTests {
    private static let phoneWidth: CGFloat = 390
    private static let noAction = RowAction("Retry") {
        // a golden presses nothing
    }

    // records `view`, themed on the grouped ground, as `name` in `variant`: `height` tall, or fitting its content
    private func snapshot(
        _ view: some View,
        named name: String,
        variant: SnapshotVariant,
        height: CGFloat? = nil,
        fileID: StaticString = #fileID,
        filePath: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line,
        column: UInt = #column
    ) {
        let themed =
            view
            .ngtermTheme()
            .frame(width: Self.phoneWidth, height: height)
            .background(Color(uiColor: .systemGroupedBackground))
            .environment(\.colorScheme, variant.colorScheme)
            .environment(\.dynamicTypeSize, variant.typeSize)
        let layout: SwiftUISnapshotLayout =
            if let height { .fixed(width: Self.phoneWidth, height: height) } else { .sizeThatFits }
        assertSnapshot(
            of: themed,
            as: .image(precision: 1, perceptualPrecision: 0.98, layout: layout, traits: variant.traits),
            named: "\(name)-\(variant.name)-on-\(SnapshotDevice.current)",
            fileID: fileID,
            file: filePath,
            testName: testName,
            line: line,
            column: column
        )
    }

    @Test("status badges", arguments: SnapshotVariant.allCases)
    internal func statusBadges(variant: SnapshotVariant) {
        let gallery = VStack(alignment: .leading, spacing: Spacing.small) {
            ForEach(StatusTone.ordered, id: \.self) { tone in StatusBadge(tone) }
            StatusBadge(.connecting, label: "Retrying in 4 s")
            ForEach(AgentState.allCases, id: \.self) { state in AgentStatusBadge(status: state.wireName) }
            AgentStatusBadge(status: "reviewing", colorHex: "#f84050", shape: "star")
            AgentStatusBadge(status: "blocked", colorHex: "#222222", shape: "square")
        }
        .padding(Spacing.large)
        .frame(maxWidth: .infinity, alignment: .leading)
        snapshot(gallery, named: "badges", variant: variant)
    }

    @Test("machine rows", arguments: SnapshotVariant.allCases)
    internal func machineRows(variant: SnapshotVariant) {
        let list = List {
            MachineRow(name: "build-01", address: "nick@build-01.lan:22", tone: .online) {
                HStack(spacing: Spacing.medium) {
                    AgentStatusBadge(status: "active")
                    AgentStatusBadge(status: "blocked")
                }
            }
            MachineRow(
                name: "nas",
                address: "admin@nas.local:2222",
                tone: .connecting,
                toneLabel: "Retrying in 4 s",
                detail: "The connection dropped",
                action: RowAction("Retry now") {
                    // a golden presses nothing
                }
            )
            MachineRow(
                name: "gpu-box",
                address: "nick@10.0.0.7:22",
                tone: .failed,
                detail: "Connection refused by 10.0.0.7",
                action: Self.noAction
            )
            MachineRow(name: "laptop", address: "nick@laptop:22", tone: .offline, detail: "Not connected")
        }
        snapshot(list, named: "machines", variant: variant, height: variant == .accessibility ? 1_400 : 560)
    }

    @Test("a machine's app badge, alone and after a row's address", arguments: SnapshotVariant.allCases)
    internal func peerBadges(variant: SnapshotVariant) {
        let list = List {
            PeerBadge("agterm", note: "Attached from agterm: the agent chat view is not available")
            MachineRow(name: "studio", address: "nick@studio.lan:22", peer: "agterm", tone: .online) {
                AgentStatusBadge(status: "active")
            }
            MachineRow(
                name: "mini",
                address: "nick@mac-mini.example.com:2222",
                peer: "agterm",
                tone: .offline,
                detail: "Not connected",
                action: Self.noAction
            )
        }
        snapshot(list, named: "peer-badges", variant: variant, height: variant == .accessibility ? 1_000 : 400)
    }

    @Test("session rows", arguments: SnapshotVariant.allCases)
    internal func sessionRows(variant: SnapshotVariant) {
        let list = List {
            SessionRow(name: "docs-site", line: "Rewrite the install guide", isAttached: true) {
                AgentStatusBadge(status: "blocked")
            }
            SessionRow(name: "api-refactor", line: "~/src/api") {
                AgentStatusBadge(status: "active")
            }
            SessionRow(name: "api-refactor review", line: "Companion of api-refactor", isCompanion: true) {
                AgentStatusBadge(status: "completed")
            }
            SessionRow(name: "scratch", line: nil, isEnding: true) {
                AgentStatusBadge(status: nil)
            }
        }
        snapshot(list, named: "sessions", variant: variant, height: variant == .accessibility ? 1_200 : 480)
    }

    @Test("state panels", arguments: SnapshotVariant.allCases)
    internal func statePanels(variant: SnapshotVariant) {
        let panels: [(name: String, panel: StatePanel)] = [
            ("loading", .loading("Reading build-01's sessions")),
            (
                "empty",
                .empty(
                    systemImage: "server.rack",
                    title: "No machines yet",
                    message: "Add a machine that runs ngterm to see its sessions here.",
                    action: RowAction("Add machine") {
                        // a golden presses nothing
                    }
                )
            ),
            (
                "error",
                .error(
                    "ngtermctl isn't on build-01",
                    message: "Install ngterm there and make sure ngtermctl is on the PATH of ssh logins.",
                    action: RowAction("Open the README") {
                        // a golden presses nothing
                    },
                    secondaryAction: Self.noAction
                )
            ),
            ("offline", .offline(retry: Self.noAction)),
        ]
        for (name, panel) in panels {
            snapshot(panel, named: "panel-\(name)", variant: variant, height: variant == .accessibility ? 1_000 : 520)
        }
    }

    @Test("banners", arguments: SnapshotVariant.allCases)
    internal func banners(variant: SnapshotVariant) {
        let gallery = VStack(spacing: Spacing.small) {
            Banner(.offline, message: "You're offline", note: "The list is as it was last read.")
            Banner(.failed, message: "Couldn't refresh", note: "Connection reset by peer", action: Self.noAction) {
                // a golden presses nothing
            }
            Banner(.attention, message: "Notifications are off")
        }
        .padding(Spacing.large)
        snapshot(gallery, named: "banners", variant: variant)
    }

    @Test("covers", arguments: SnapshotVariant.allCases)
    internal func covers(variant: SnapshotVariant) {
        let pane = Text(verbatim: String(repeating: "$ cargo build --release\n   Compiling ngterm\n", count: 12))
            .font(.system(.footnote, design: .monospaced))
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .padding(Spacing.xSmall)
        let driven = Cover(
            systemImage: "lock.fill",
            title: "Another device is driving this pane",
            message: "Its size and keys are the other device's. Take control to type here; the other device follows.",
            action: RowAction("Take control") {
                // a golden presses nothing
            }
        )
        let attaching = Cover(systemImage: nil, title: "Attaching", message: "Reading this pane from build-01.")
        let height: CGFloat = variant == .accessibility ? 900 : 520
        snapshot(pane.covered(by: driven), named: "cover", variant: variant, height: height)
        snapshot(pane.covered(by: attaching), named: "cover-attaching", variant: variant, height: height)
    }

    @Test("plates and inline problems", arguments: SnapshotVariant.allCases)
    internal func plates(variant: SnapshotVariant) {
        let gallery = VStack(alignment: .leading, spacing: Spacing.medium) {
            ValuePlate(
                "Public key", value: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForScreenshotsOnly ngterm@iPhone"
            ) {
                CopyButton("ssh-ed25519 AAAA")
            }
            ValuePlate("Fingerprint", value: "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E")
            InlineProblem("Enter a port from 1 to 65535.")
        }
        .padding(Spacing.large)
        snapshot(gallery, named: "plates", variant: variant)
    }
}
