import Foundation
import NgtermPlatform

/// The machine form's fields as typed: the port as text, so a half-typed one can be shown and corrected.
public struct MachineDraft: Hashable, Sendable {
    private static let maxPort = Int(UInt16.max)

    /// The user's name for the machine; blank for none.
    public var label: String
    /// Its host name or address.
    public var host: String
    /// Its ssh port, as typed.
    public var port: String
    /// The user it signs in as.
    public var user: String
    /// The key it signs in with; empty for none chosen.
    public var keyId: String

    /// What is wrong with it, at most one problem per field.
    public var problems: [MachineField: FieldProblem] {
        let found = [
            Self.hostProblem(host.trimmingCharacters(in: .whitespacesAndNewlines)),
            Self.portProblem(port.trimmingCharacters(in: .whitespacesAndNewlines)),
            Self.userProblem(user.trimmingCharacters(in: .whitespacesAndNewlines)),
            keyId.isEmpty ? FieldProblem.keyMissing : nil,
        ]
        return Dictionary(uniqueKeysWithValues: found.compactMap(\.self).map { ($0.field, $0) })
    }

    /// An empty draft on the ssh port.
    public init(label: String = "", host: String = "", port: String? = nil, user: String = "", keyId: String = "") {
        self.label = label
        self.host = host
        self.port = port ?? String(StoredMachine.sshPort)
        self.user = user
        self.keyId = keyId
    }

    /// The draft that edits `machine`.
    public init(editing machine: StoredMachine) {
        self.init(
            label: machine.label,
            host: machine.host,
            port: String(machine.port),
            user: machine.user,
            keyId: machine.keyId
        )
    }

    // `host:22`, but not an IPv6 address, which has several colons
    private static func hasPort(_ host: String) -> Bool {
        guard let colon = host.firstIndex(of: ":"), colon == host.lastIndex(of: ":"), colon > host.startIndex else {
            return false
        }
        return host[host.index(after: colon)...].allSatisfy { ("0"..."9").contains($0) }
    }

    private static func portProblem(_ port: String) -> FieldProblem? {
        guard let number = Int(port), (1...maxPort).contains(number) else {
            return .portInvalid
        }
        return nil
    }

    private static func hostProblem(_ host: String) -> FieldProblem? {
        if host.isEmpty {
            return .hostMissing
        }
        if host.contains(where: \.isWhitespace) {
            return .hostHasSpace
        }
        if host.contains("@") {
            return .hostHasUser
        }
        return hasPort(host) ? .hostHasPort : nil
    }

    private static func userProblem(_ user: String) -> FieldProblem? {
        if user.isEmpty {
            return .userMissing
        }
        return user.contains(where: \.isWhitespace) ? .userHasSpace : nil
    }

    /// The machine it describes, as `id`; `nil` while it has ``problems``.
    public func machine(id: String) -> StoredMachine? {
        guard problems.isEmpty, let number = UInt16(port.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            return nil
        }
        return StoredMachine(
            id: id,
            label: label.trimmingCharacters(in: .whitespacesAndNewlines),
            host: host.trimmingCharacters(in: .whitespacesAndNewlines),
            port: number,
            user: user.trimmingCharacters(in: .whitespacesAndNewlines),
            keyId: keyId
        )
    }
}
