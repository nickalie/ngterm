import NgtermFeatures
import NgtermPlatform
import Testing

/// The machine form's draft: its validation, as mobile/android's `MachineDraftTest` has it.
@Suite("the machine draft")
internal struct MachineDraftTests {
    private static let good = MachineDraft(label: "", host: "build-01.lan", port: "22", user: "nick", keyId: "k1")

    private static func with(_ change: (inout MachineDraft) -> Void) -> MachineDraft {
        var draft = good
        change(&draft)
        return draft
    }

    @Test("a complete draft has no problems and becomes its machine, trimmed")
    internal func complete() {
        #expect(Self.good.problems.isEmpty)
        let spaced = Self.with { draft in
            draft.host = "  build-01.lan "
            draft.user = " nick"
        }
        #expect(
            spaced.machine(id: "m1")
                == StoredMachine(id: "m1", label: "", host: "build-01.lan", port: 22, user: "nick", keyId: "k1")
        )
    }

    @Test(
        "each field says what is wrong with it",
        arguments: [
            (with { $0.host = "" }, FieldProblem.hostMissing),
            (with { $0.host = "build 01" }, .hostHasSpace),
            (with { $0.host = "nick@build-01" }, .hostHasUser),
            (with { $0.host = "build-01:2222" }, .hostHasPort),
            (with { $0.host = "build-01:" }, .hostHasPort),
            (with { $0.port = "" }, .portInvalid),
            (with { $0.port = "0" }, .portInvalid),
            (with { $0.port = "65536" }, .portInvalid),
            (with { $0.port = "22a" }, .portInvalid),
            (with { $0.user = " " }, .userMissing),
            (with { $0.user = "ni ck" }, .userHasSpace),
            (with { $0.keyId = "" }, .keyMissing),
        ]
    )
    internal func problems(draft: MachineDraft, problem: FieldProblem) {
        #expect(draft.problems == [problem.field: problem])
        #expect(draft.machine(id: "m1") == nil)
    }

    @Test("an IPv6 address is a host, not a host with a port")
    internal func ipv6() {
        #expect(Self.with { $0.host = "fe80::1" }.problems.isEmpty)
        #expect(Self.with { $0.host = "2001:db8::42" }.problems.isEmpty)
    }

    @Test("a new draft is on the ssh port, and a machine edits as the draft it was made from")
    internal func roundTrip() {
        #expect(MachineDraft().port == "22")
        let machine = StoredMachine(
            id: "m1", label: "Build box", host: "build-01.lan", port: 2_222, user: "nick", keyId: "k1"
        )
        #expect(MachineDraft(editing: machine).machine(id: "m1") == machine)
    }

    @Test("every problem has words, about its own field")
    internal func messages() {
        for problem in FieldProblem.allCases {
            #expect(!problem.message.isEmpty)
        }
        #expect(Set(FieldProblem.allCases.map(\.field)) == [.host, .port, .user, .key])
    }
}
