import NgtermCore

/// An attached session as its presentation stream left it (mobile/android's `LiveSession`).
///
/// Whether the stream has said a status or a context yet is ``isStatusSaid`` and ``isContextSaid``: once it has, its
/// `nil` is a clear, not a gap the machine's listing fills. ``agents`` are what its panes run (every pane's, shown or
/// not, by id; an origin that predates the chat view announces none, and a stream that drops keeps them, as the origin
/// announces them all again when it is back).
public struct LiveSession: Hashable, Sendable {
    /// The stream's state.
    public var connection: StreamState
    /// The agent's status.
    public var status: RemoteStatus?
    /// What the session is about.
    public var context: String?
    /// Its panes' layout: `nil` before the first.
    public var layout: RemoteLayout?
    /// The HUD over it.
    public var hud: RemoteHud?
    /// Its shown panes, by id.
    public var panes: [String: PaneLive]
    /// Whether the stream has said a status.
    public var isStatusSaid: Bool
    /// Whether the stream has said a context.
    public var isContextSaid: Bool
    /// The agents its panes run, by pane id.
    public var agents: [String: PaneAgent] = [:]

    /// A session nothing has been said of yet: its stream connecting.
    public init() {
        self.init(connection: .connecting)
    }

    /// A session as its stream has told it so far: its `connection`, and what else it has said.
    public init(
        connection: StreamState,
        status: RemoteStatus? = nil,
        context: String? = nil,
        layout: RemoteLayout? = nil,
        hud: RemoteHud? = nil,
        panes: [String: PaneLive] = [:],
        isStatusSaid: Bool = false,
        isContextSaid: Bool = false
    ) {
        self.connection = connection
        self.status = status
        self.context = context
        self.layout = layout
        self.hud = hud
        self.panes = panes
        self.isStatusSaid = isStatusSaid
        self.isContextSaid = isContextSaid
    }
}
