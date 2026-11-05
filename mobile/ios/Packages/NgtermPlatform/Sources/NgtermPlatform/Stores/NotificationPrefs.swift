/// The user's notification choices (mobile/android's `NotificationPrefs`): the kinds turned off, and whether the app
/// has asked for the permission to notify, so it asks once.
public struct NotificationPrefs: Codable, Hashable, Sendable {
    private enum CodingKeys: CodingKey {
        case off
        case permissionAsked
    }

    /// The kinds the user turned off.
    public var off: Set<NoticeKind>
    /// Whether the app has asked for the permission to notify (or the user said Not now).
    public var isPermissionAsked: Bool

    /// The kinds that are on.
    public var enabled: Set<NoticeKind> {
        Set(NoticeKind.allCases).subtracting(off)
    }

    /// The choices given: every kind on, and the permission not asked yet, by default.
    public init(off: Set<NoticeKind> = [], isPermissionAsked: Bool = false) {
        self.off = off
        self.isPermissionAsked = isPermissionAsked
    }

    /// The choices as stored: a value missing reads as its default, and a kind this build doesn't know is dropped.
    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let names = (try? values.decodeIfPresent([String].self, forKey: .off)) ?? []
        off = Set(names.compactMap(NoticeKind.stored))
        isPermissionAsked = (try? values.decodeIfPresent(Bool.self, forKey: .permissionAsked)) ?? false
    }

    /// Writes the choices, each kind turned off by its name.
    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(off.map(\.storedName).sorted(), forKey: .off)
        try values.encode(isPermissionAsked, forKey: .permissionAsked)
    }
}
