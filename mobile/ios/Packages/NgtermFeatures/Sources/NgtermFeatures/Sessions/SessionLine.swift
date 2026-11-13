import NgtermCore

/// A session as the list shows it: the session, its parent's name when it is a companion, what is under way, and
/// whether Duplicate takes a tap.
public struct SessionLine: Hashable, Sendable, Identifiable {
    /// The session.
    public let session: RemoteSession
    /// The name of the session it is a companion of (its id when that isn't listed); `nil` for none.
    public let companionOf: String?
    /// Whether this app has it attached.
    public let isAttached: Bool
    /// Whether its End is under way.
    public let isEnding: Bool
    /// Whether its Duplicate is under way.
    public let isDuplicating: Bool
    /// Whether Duplicate takes a tap: not while a create is under way.
    public let canDuplicate: Bool

    /// The session's id.
    public var id: String {
        session.id
    }

    /// What the row says under the name: "Companion of api" for a companion, else the session's context or, without
    /// one, its working directory.
    public var line: String? {
        if let companionOf {
            return String(localized: "Companion of \(companionOf)")
        }
        if let context = session.context, !context.allSatisfy(\.isWhitespace) {
            return context
        }
        return session.cwd.allSatisfy(\.isWhitespace) ? nil : session.cwd
    }

    /// `session` as a companion of `companionOf` (or not), attached or not, being ended or duplicated or not, and
    /// whether it `canDuplicate`.
    public init(
        session: RemoteSession,
        companionOf: String?,
        isAttached: Bool,
        isEnding: Bool,
        isDuplicating: Bool = false,
        canDuplicate: Bool = true
    ) {
        self.session = session
        self.companionOf = companionOf
        self.isAttached = isAttached
        self.isEnding = isEnding
        self.isDuplicating = isDuplicating
        self.canDuplicate = canDuplicate
    }
}
