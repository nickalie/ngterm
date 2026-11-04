/// What the app shows of one of its ssh keys: never the private half, which only the Keychain holds
/// (``KeychainKeyVault``).
public struct AppKey: Codable, Hashable, Sendable, Identifiable {
    /// The key's identity: the vault's account for its private half, and what a machine's `keyId` names.
    public let id: String
    /// The public key as a line of `authorized_keys`.
    public let authorizedKey: String
    /// Its OpenSSH `SHA256:` fingerprint.
    public let fingerprint: String
    /// Whether the user brought it rather than the app making it.
    public let isImported: Bool

    /// The key `id`, its public half `authorizedKey` with its `fingerprint`, made here or `isImported`.
    public init(id: String, authorizedKey: String, fingerprint: String, isImported: Bool) {
        self.id = id
        self.authorizedKey = authorizedKey
        self.fingerprint = fingerprint
        self.isImported = isImported
    }
}
