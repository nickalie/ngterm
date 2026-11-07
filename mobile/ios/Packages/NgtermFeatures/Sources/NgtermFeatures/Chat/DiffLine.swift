/// One line of a unified diff, as it reads, with what it is.
public struct DiffLine: Equatable, Sendable {
    /// What the line is.
    public let kind: DiffLineKind
    /// The line, its `+` or `-` kept.
    public let text: String
}
