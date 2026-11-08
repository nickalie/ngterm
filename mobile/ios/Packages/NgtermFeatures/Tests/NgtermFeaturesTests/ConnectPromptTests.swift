import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The questions a connect waits on, as the app's sheets tell them apart (the sheets themselves, over a window of the
/// app's scene: NgTermTests' `ConnectPromptSheetTests`).
@MainActor
@Suite("the connect questions")
internal struct ConnectPromptTests {
    @Test("each question about each machine is a sheet of its own; a sign-in told again in other words is the same")
    internal func identity() {
        let first = TestGraph.machine(keyId: "k1")
        let second = TestGraph.machine(id: "m2", label: "studio", host: "studio.lan", keyId: "k1")
        let link = "https://login.tailscale.com/a/1"
        let prompts: [ConnectPrompt] = [
            .hostKey(machine: first, fingerprint: "SHA256:a"),
            .hostKey(machine: first, fingerprint: "SHA256:b"),
            .hostKey(machine: second, fingerprint: "SHA256:a"),
            .signIn(machine: first, message: "Sign in", url: link),
            .signIn(machine: first, message: "Sign in", url: "https://login.tailscale.com/a/2"),
            .signIn(machine: second, message: "Sign in", url: link),
        ]
        #expect(Set(prompts.map(\.id)).count == prompts.count)
        #expect(
            ConnectPrompt.signIn(machine: first, message: "Sign in", url: link).id
                == ConnectPrompt.signIn(machine: first, message: "Again", url: link).id
        )
        #expect(prompts.map(\.machine.id) == ["m1", "m1", "m2", "m1", "m1", "m2"])
    }
}
