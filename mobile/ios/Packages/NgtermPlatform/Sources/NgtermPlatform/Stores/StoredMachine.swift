import Foundation
import NgtermCore

/// A machine as the app keeps it: what the user entered and the key it signs in with.
///
/// Its pinned host key is not here: ``KeychainHostKeys`` holds it, the one place the core reads it from.
public struct StoredMachine: Codable, Hashable, Sendable, Identifiable {
    private enum CodingKeys: CodingKey {
        case host
        case id
        case keyId
        case label
        case port
        case user
    }

    /// The ssh port a new machine starts with, and the one a stored machine without a port has.
    public static let sshPort: UInt16 = 22

    /// The machine's identity, which the core and the host-key pins know it by.
    public let id: String
    /// The user's name for it; blank for none.
    public var label: String
    /// Its host name or address.
    public var host: String
    /// Its ssh port.
    public var port: UInt16
    /// The user it signs in as.
    public var user: String
    /// The key it signs in with.
    public var keyId: String

    /// What the list shows for it: its name, or its host when it has none.
    public var title: String {
        label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? host : label
    }

    /// Its ssh address as `user@host:port`, in the form people compare.
    public var address: String {
        "\(user)@\(host):\(port)"
    }

    /// The record the core connects; it never carries the pin, since the core trusts only its host-key store.
    public var core: Machine {
        Machine(id: id, label: title, host: host, port: port, user: user, keyId: keyId)
    }

    /// The machine `id`, reached as `user` at `host`, on `port`, with the key `keyId`.
    public init(id: String, label: String, host: String, port: UInt16, user: String, keyId: String) {
        self.id = id
        self.label = label
        self.host = host
        self.port = port
        self.user = user
        self.keyId = keyId
    }

    /// A machine as stored: a missing label is none, and a missing port the ssh one.
    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        label = try values.decodeIfPresent(String.self, forKey: .label) ?? ""
        host = try values.decode(String.self, forKey: .host)
        port = try values.decodeIfPresent(UInt16.self, forKey: .port) ?? Self.sshPort
        user = try values.decode(String.self, forKey: .user)
        keyId = try values.decode(String.self, forKey: .keyId)
    }
}
