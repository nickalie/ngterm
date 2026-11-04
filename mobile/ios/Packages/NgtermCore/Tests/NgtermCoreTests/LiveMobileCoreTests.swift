import Foundation
import NgtermCore
import Testing

/// The facade over the linked Rust core: its errors, its events and its end, with no machine to reach (a refusing
/// vault fails a connect before it dials, and a silent listener holds one open).
@Suite("the live core", .timeLimit(.minutes(1)))
internal struct LiveMobileCoreTests {
    private static func machine(port: UInt16 = 22) -> Machine {
        Machine(id: "m1", label: "box", host: "127.0.0.1", port: port, user: "me", keyId: "k1")
    }

    /// A core whose vault refuses with `refusal`: it connects `m1` once (the vault refuses) and is dropped, and only
    /// its event stream is handed back.
    private static func streamOfADroppedCore(refusal: VaultError) async throws -> AsyncStream<MobileEvent> {
        let core = try LiveMobileCore(vault: StubVault(refusal: refusal), hosts: MemoryHostKeys())
        await #expect(throws: MobileError.self) {
            try await core.connect(machine: machine())
        }
        return core.events
    }

    @Test("a call's MobileError is thrown as a Swift error")
    internal func mobileErrorIsThrown() async throws {
        let core = try LiveMobileCore(vault: StubVault(), hosts: MemoryHostKeys())
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.listSessions(machineId: "m1")
        }
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.createSession(machineId: "m1", name: nil)
        }
    }

    @Test(
        "the vault refusing fails the connect with its reason, reported as the machine's state too",
        arguments: [
            (VaultError.Missing(keyId: "k1"), "no key k1"),
            (VaultError.Unavailable(reason: "the Keychain is locked"), "the Keychain is locked"),
        ]
    )
    internal func vaultRefusing(refusal: VaultError, reason: String) async throws {
        let core = try LiveMobileCore(vault: StubVault(refusal: refusal), hosts: MemoryHostKeys())
        await #expect(throws: MobileError.Unreachable(reason: reason)) {
            try await core.connect(machine: Self.machine())
        }
        var events = core.events.makeAsyncIterator()
        #expect(await events.next() == .machineState(machineId: "m1", state: .unreachable(reason: reason)))
    }

    @Test("a disconnect overtaking a connect fails it as cancelled")
    internal func disconnectCancelsTheConnect() async throws {
        let listener = try #require(SilentListener())
        let key = try CoreKeys.generate(comment: "test")
        let core = try LiveMobileCore(vault: StubVault(keys: ["k1": key.privateKey]), hosts: MemoryHostKeys())
        let machine = Self.machine(port: listener.port)
        let connecting = Task {
            try await core.connect(machine: machine)
        }
        #expect(try await listener.waitForConnection(timeout: .seconds(10)))
        try core.disconnect(machineId: "m1")
        await #expect(throws: MobileError.Cancelled) {
            try await connecting.value
        }
        #expect(try await listener.waitForClose(timeout: .seconds(10)))
    }

    @Test("a call whose task is cancelled stops waiting at once, and what it asked for goes on in the core")
    internal func cancelledTaskStopsWaiting() async throws {
        let listener = try #require(SilentListener())
        let key = try CoreKeys.generate(comment: "test")
        let core = try LiveMobileCore(vault: StubVault(keys: ["k1": key.privateKey]), hosts: MemoryHostKeys())
        let machine = Self.machine(port: listener.port)
        let connecting = Task {
            try await core.connect(machine: machine)
        }
        #expect(try await listener.waitForConnection(timeout: .seconds(10)))
        connecting.cancel()
        await #expect(throws: CancellationError.self) {
            try await connecting.value
        }
        // the connect is still waiting on the machine's greeting: only the disconnect ends it
        #expect(try await !listener.waitForClose(timeout: .milliseconds(300)))
        try core.disconnect(machineId: "m1")
        #expect(try await listener.waitForClose(timeout: .seconds(10)))
    }

    @Test("the event stream ends once the core is dropped, after the events it already had")
    internal func streamEndsWhenTheCoreIsDropped() async throws {
        let events = try await Self.streamOfADroppedCore(refusal: .Missing(keyId: "k1"))
        var received: [MobileEvent] = []
        for await event in events {
            received.append(event)
        }
        #expect(received == [.machineState(machineId: "m1", state: .unreachable(reason: "no key k1"))])
    }

    @Test("the core's keys: a new one is an ed25519 key, and an import of anything else is refused")
    internal func keys() throws {
        let key = try CoreKeys.generate(comment: "phone")
        #expect(key.authorizedKey.hasPrefix("ssh-ed25519 "))
        #expect(key.authorizedKey.hasSuffix(" phone"))
        #expect(key.fingerprint.hasPrefix("SHA256:"))
        #expect(try CoreKeys.importKey(privateKey: key.privateKey, passphrase: nil).fingerprint == key.fingerprint)
        #expect(throws: KeyError.self) {
            try CoreKeys.importKey(privateKey: Data("not a key".utf8), passphrase: nil)
        }
    }
}
