/// An attached session as the app shows it (mobile/android's `AttachedSession`).
///
/// Its machine's name, its name, context and agent status (the stream's once it has said them, the machine's last
/// listing's until then), its machine's link, its stream's live state, its parent's name when it is a companion, and
/// whether it is nested under that parent (attached too, and listed above it).
public struct AttachedSession: Hashable, Sendable {
    /// Which session it is.
    public let key: SessionKey
    /// Its machine's name.
    public let machine: String
    /// Its name.
    public let name: String
    /// What it is about.
    public let context: String?
    /// Its agent's status.
    public let status: AgentStatus?
    /// Its machine's link.
    public let link: Link
    /// What its stream has said.
    public let live: LiveSession
    /// The name of the session it is a companion of (its id when that isn't listed); `nil` for none.
    public let companionOf: String?
    /// Whether it sits under its parent: a companion whose parent is attached and listed above it.
    public let isNested: Bool

    /// The session `key` on `machine`, named `name`, over `link`, its stream at `live`, about `context`, with `status`,
    /// a companion of `companionOf` (or not), nested under it or not.
    public init(
        key: SessionKey,
        machine: String,
        name: String,
        link: Link,
        live: LiveSession,
        context: String? = nil,
        status: AgentStatus? = nil,
        companionOf: String? = nil,
        isNested: Bool = false
    ) {
        self.key = key
        self.machine = machine
        self.name = name
        self.context = context
        self.status = status
        self.link = link
        self.live = live
        self.companionOf = companionOf
        self.isNested = isNested
    }
}
