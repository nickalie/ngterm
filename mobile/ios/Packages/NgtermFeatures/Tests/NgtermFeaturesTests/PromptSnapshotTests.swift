import NgtermCoreFakes
import NgtermPlatform
import NgtermTerminal
import Testing

@testable import NgtermFeatures

/// Goldens of the questions a first connect asks (a host key to trust, a sign-in, and a changed host key's stop), and
/// of a program's clipboard read.
@MainActor
@Suite("connect prompt snapshots")
internal struct PromptSnapshotTests {
    private static let machine = StoredMachine(
        id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: "k1"
    )

    @Test("the fingerprint to confirm", arguments: ScreenVariant.allCases)
    internal func hostKey(variant: ScreenVariant) {
        let sheet = HostKeySheet(
            machine: Self.machine,
            fingerprint: "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E"
        ) { _ in
            // a golden answers nothing
        }
        ScreenSnapshot.check(sheet, named: "host-key", variant: variant)
    }

    @Test("the sign-in a machine asks for", arguments: ScreenVariant.allCases)
    internal func signIn(variant: ScreenVariant) {
        let sheet = SignInSheet(
            machine: Self.machine,
            message: FakeSignIn.tailscale.message,
            url: FakeSignIn.tailscale.url,
            onCancel: ScreenSnapshot.ignore
        )
        ScreenSnapshot.check(sheet, named: "sign-in", variant: variant)
    }

    @Test("a program asks to read the clipboard", arguments: ScreenVariant.allCases)
    internal func clipboardRead(variant: ScreenVariant) {
        let question = ClipboardReadQuestion(
            listOnly: false,
            machine: "build-01",
            session: "docs-site",
            onAllow: ScreenSnapshot.ignore,
            onAlways: ScreenSnapshot.ignore,
            onDeny: ScreenSnapshot.ignore
        )
        ScreenSnapshot.check(question, named: "clipboard-read", variant: variant)
    }

    @Test("a program asks only what kinds the clipboard holds")
    internal func clipboardList() {
        let question = ClipboardReadQuestion(
            listOnly: true,
            machine: "build-01",
            session: "docs-site",
            onAllow: ScreenSnapshot.ignore,
            onAlways: ScreenSnapshot.ignore,
            onDeny: ScreenSnapshot.ignore
        )
        ScreenSnapshot.check(question, named: "clipboard-list", variant: .light)
    }

    @Test("a changed host key stops the connection", arguments: ScreenVariant.allCases)
    internal func hostKeyChanged(variant: ScreenVariant) {
        let test = TestGraph()
        defer { test.remove() }
        let item = MachineItem(
            machine: Self.machine,
            link: .hostKeyChanged(
                pinned: "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E",
                offered: "SHA256:Zx9vT2mQ8rLk4wJ1nB6cY3pH7sD0fG5aE2uI9oK8tR4"
            ),
            isListed: false,
            sessions: []
        )
        let sheet = HostKeyChangedSheet(model: MachinesModel(graph: test.graph), item: item)
        ScreenSnapshot.check(sheet, named: "host-key-changed", variant: variant)
    }
}
