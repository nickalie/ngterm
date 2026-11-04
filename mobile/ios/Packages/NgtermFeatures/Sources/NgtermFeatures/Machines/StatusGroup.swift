import NgtermCore

/// The sessions of a machine that share an agent status: the status, the first of them (whose color and shape the
/// badge takes), and how many there are.
internal struct StatusGroup {
    internal let status: String
    internal let first: RemoteSession
    internal var count = 1
}
