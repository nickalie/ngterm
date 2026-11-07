/// What a line of a unified diff is, for its color.
public enum DiffLineKind: Equatable, Sendable {
    /// An added line.
    case added
    /// A line the change keeps, or anything else.
    case context
    /// A file's header: `diff --git`, `index`, `---`, `+++`.
    case file
    /// A hunk's `@@ … @@` line.
    case hunk
    /// A removed line.
    case removed
}
