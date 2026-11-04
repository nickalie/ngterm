import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// What a machine's sessions screen shows, by its link and listing, as mobile/android's `SessionsBodyTest` has it:
/// companions under their parent, and the Attached group first.
@MainActor
@Suite("the sessions screen's body")
internal struct SessionsBodyTests {
    // `listed` is the machine's listing; with none, the machine hasn't listed its sessions yet
    private func body(
        _ link: Link = .connected,
        listed: [RemoteSession] = [],
        isListed: Bool = true,
        failure: String? = nil,
        attached: Set<String> = [],
        ending: Set<String> = [],
        isOffline: Bool = false
    ) -> SessionsBody {
        SessionsBody(
            MachineSessions(
                machine: TestGraph.machine(keyId: "k1"),
                link: link,
                isListed: isListed,
                sessions: listed,
                failure: failure,
                attached: attached,
                isOffline: isOffline,
                ending: ending
            )
        )
    }

    // the list's two groups, when the body is a list
    private func lines(_ body: SessionsBody) -> (attached: [SessionLine], others: [SessionLine])? {
        guard case .sessions(let attached, let others, _) = body else {
            return nil
        }
        return (attached, others)
    }

    @Test("the link decides until the machine is connected")
    internal func linkDecides() {
        let retryAt = Date(timeIntervalSince1970: 42)
        #expect(body(.offline) == .notConnected(isDeviceOffline: false))
        #expect(body(.offline, isOffline: true) == .notConnected(isDeviceOffline: true))
        #expect(body(.connecting) == .connecting)
        #expect(body(.hostKeyUnknown(fingerprint: "SHA256:x")) == .connecting)
        #expect(body(.signIn(message: "sign in", url: "https://x", isReconnecting: false)) == .connecting)
        #expect(body(.reconnecting(retryAt: retryAt)) == .reconnecting(retryAt: retryAt))
        #expect(body(.unreachable(reason: "refused")) == .unreachable(reason: "refused"))
        #expect(body(.hostKeyChanged(pinned: "a", offered: "b")) == .hostKeyChanged)
        // a listing kept from before doesn't hide that the machine is down
        #expect(
            body(.unreachable(reason: "refused"), listed: [fakeSession(id: "s1")]) == .unreachable(reason: "refused"))
    }

    @Test("a connected machine shows its listing, or why there is none")
    internal func listingOrWhy() {
        #expect(body(isListed: false) == .reading)
        #expect(body(isListed: false, failure: "no answer") == .listFailed(reason: "no answer"))
        let missing = "/bin/sh: 1: ngtermctl: not found"
        #expect(body(isListed: false, failure: missing) == .missingCli(reason: missing))
        #expect(body(listed: []) == .empty(failure: nil))
        // a failed refresh keeps the listing before it, and says so
        #expect(body(listed: [], failure: "no answer") == .empty(failure: "no answer"))
        guard case .sessions(_, _, let failure) = body(listed: [fakeSession(id: "s1")], failure: "no answer") else {
            Issue.record("not a list")
            return
        }
        #expect(failure == "no answer")
    }

    @Test("attached sessions come first, and companions name their parent")
    internal func attachedFirst() throws {
        let listed = [
            fakeSession(id: "a", name: "api"),
            fakeSession(id: "a2", name: "api review", parent: "a"),
            fakeSession(id: "b", name: "build"),
            fakeSession(id: "c", name: "orphan", parent: "gone"),
        ]
        let groups = try #require(lines(body(listed: listed, attached: ["b"], ending: ["a"])))
        #expect(groups.attached.map(\.id) == ["b"])
        #expect(groups.attached.map(\.isAttached) == [true])
        #expect(groups.others.map(\.id) == ["a", "a2", "c"])
        #expect(groups.others.map(\.companionOf) == [nil, "api", "gone"])
        #expect(groups.others.map(\.isEnding) == [true, false, false])
        #expect(groups.others.map(\.line) == ["/home/fake", "Companion of api", "Companion of gone"])
    }

    @Test("companions sit under their parent in the core's order, and go with their family")
    internal func families() throws {
        // the core lists every companion after all the parents
        let listed = [
            fakeSession(id: "a", name: "api"),
            fakeSession(id: "b", name: "build"),
            fakeSession(id: "a2", name: "api review", parent: "a"),
        ]
        let none = try #require(lines(body(listed: listed)))
        #expect(none.attached.isEmpty)
        #expect(none.others.map(\.id) == ["a", "a2", "b"])
        #expect(none.others.map(\.companionOf) == [nil, "api", nil])
        // attaching the parent, or only its companion, takes the whole family to the Attached group
        for attached: Set<String> in [["a"], ["a2"]] {
            let groups = try #require(lines(body(listed: listed, attached: attached)))
            #expect(groups.attached.map(\.id) == ["a", "a2"])
            #expect(groups.attached.map(\.isAttached) == ["a", "a2"].map(attached.contains))
            #expect(groups.others.map(\.id) == ["b"])
        }
    }

    @Test("a loop of parents has no head: its sessions stand alone, and none goes missing")
    internal func parentLoop() throws {
        let listed = [
            fakeSession(id: "x", parent: "y"),
            fakeSession(id: "y", parent: "x"),
            fakeSession(id: "z"),
        ]
        #expect(try #require(lines(body(listed: listed))).others.map(\.id) == ["z", "x", "y"])
    }

    @Test("a row's line is the context, else the working directory")
    internal func rowLine() {
        var session = fakeSession(id: "s1")
        session.context = "Rewrite the guide"
        #expect(
            SessionLine(session: session, companionOf: nil, isAttached: false, isEnding: false).line
                == "Rewrite the guide")
        session.context = "  "
        #expect(
            SessionLine(session: session, companionOf: nil, isAttached: false, isEnding: false).line == "/home/fake")
        session.cwd = ""
        #expect(SessionLine(session: session, companionOf: nil, isAttached: false, isEnding: false).line == nil)
    }

    @Test("a missing ngtermctl is the shell's words for it")
    internal func missingCli() {
        #expect(SessionsBody.isMissingCli("/bin/sh: 1: ngtermctl: not found"))
        #expect(SessionsBody.isMissingCli("bash: line 1: ngtermctl: command not found"))
        #expect(SessionsBody.isMissingCli("env: 'ngtermctl': No such file or directory"))
        #expect(!SessionsBody.isMissingCli("build-01 gave no answer; nothing was created"))
        #expect(!SessionsBody.isMissingCli("session not found"))
    }
}
