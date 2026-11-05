/// An attached session as the app shows it (mobile/android's `AttachedSession`).
///
/// Its machine's name, its name, context and agent status (the stream's once it has said them, the machine's last
/// listing's until then), its machine's link, and its stream's live state.
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

    /// The session `key` on `machine`, named `name`, over `link`, its stream at `live`, about `context`, with `status`.
    public init(
        key: SessionKey,
        machine: String,
        name: String,
        link: Link,
        live: LiveSession,
        context: String? = nil,
        status: AgentStatus? = nil
    ) {
        self.key = key
        self.machine = machine
        self.name = name
        self.context = context
        self.status = status
        self.link = link
        self.live = live
    }
}
