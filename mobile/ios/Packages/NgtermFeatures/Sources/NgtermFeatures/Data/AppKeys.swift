import Foundation
import NgtermCore
import NgtermPlatform
import Observation

/// The app's ssh keys: what it shows of them in a ``KeyStore``, their private halves in ``PrivateKeys`` (the Keychain).
///
/// ``generate()``, ``importKey(privateKey:passphrase:)`` and ``rotate()`` put a new key in both, wiping the private
/// bytes once kept; they run the ``KeyMaker`` off the main actor and throw what it throws (``KeyError``), or why the
/// key could not be kept (``StoreError``, the vault's error), in which case nothing of it is kept.
@preconcurrency
@MainActor
@Observable
public final class AppKeys {
    @ObservationIgnored private let store: KeyStore
    @ObservationIgnored private let vault: any PrivateKeys
    @ObservationIgnored private let maker: any KeyMaker
    @ObservationIgnored private let comment: String

    /// The keys, oldest first.
    public var keys: [AppKey] {
        store.keys
    }

    /// Why the keys' file couldn't be read, while it can't be (``KeyStore/readFailure``): ``keys`` is empty then, not
    /// what the app has.
    public var readFailure: StoreError? {
        store.readFailure
    }

    /// The keys shown by `store`, their private halves in `vault`, made or read by `maker`; a key made here has its
    /// public half commented `comment` (`ngterm@iPhone`), which shows in a machine's `authorized_keys`.
    public init(store: KeyStore, vault: any PrivateKeys, maker: any KeyMaker, comment: String) {
        self.store = store
        self.vault = vault
        self.maker = maker
        self.comment = comment
    }

    @concurrent
    nonisolated private static func generate(with maker: any KeyMaker, comment: String) async throws -> KeyPair {
        try maker.generate(comment: comment)
    }

    @concurrent
    nonisolated private static func read(
        _ privateKey: Data,
        passphrase: String?,
        with maker: any KeyMaker
    ) async throws -> KeyPair {
        try maker.importKey(privateKey: privateKey, passphrase: passphrase)
    }

    /// Reads the keys' file again if the store couldn't read it before (``KeyStore/readIfUnread()``).
    public func readIfUnread() {
        store.readIfUnread()
    }

    /// The key `id`, if the app has it.
    public func key(id: String) -> AppKey? {
        store.key(id: id)
    }

    /// A new key made on this device.
    public func generate() async throws -> AppKey {
        var pair = try await Self.generate(with: maker, comment: comment)
        return try add(&pair, isImported: false)
    }

    /// Rotate key: a new key made on this device, beside the others.
    ///
    /// The machines keep signing in with theirs until they are pointed at it (``MachineStore/useKey(_:)``), once its
    /// public half is on them.
    public func rotate() async throws -> AppKey {
        try await generate()
    }

    /// The key the user brought, read from `privateKey` (zeroed once read, whatever the outcome) and opened with
    /// `passphrase` (an empty one is none).
    public func importKey(privateKey: inout Data, passphrase: String?) async throws -> AppKey {
        defer {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
        }
        var pair = try await Self.read(
            privateKey,
            passphrase: passphrase.flatMap { $0.isEmpty ? nil : $0 },
            with: maker
        )
        return try add(&pair, isImported: true)
    }

    // keeps the private half in the vault (which wipes it in place: `pair` is the one holder of its bytes) and the rest
    // in the store; a key the store can't take leaves the vault too
    private func add(_ pair: inout KeyPair, isImported: Bool) throws -> AppKey {
        let key = AppKey(
            id: UUID().uuidString,
            authorizedKey: pair.authorizedKey.trimmingCharacters(in: .whitespacesAndNewlines),
            fingerprint: pair.fingerprint,
            isImported: isImported
        )
        try vault.store(keyId: key.id, privateKey: &pair.privateKey)
        do {
            try store.add(key)
        } catch {
            try? vault.delete(keyId: key.id)
            throw error
        }
        return key
    }

    deinit {
        // nothing to release: the store and the vault keep what they were given
    }
}
