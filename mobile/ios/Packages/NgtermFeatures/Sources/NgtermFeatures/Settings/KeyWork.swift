/// Where making or bringing a key stands (mobile/android's `KeyWork`).
public struct KeyWork: Hashable, Sendable {
    /// A new key is being made (Rotate key).
    public var isRotating = false
    /// Why the last rotation failed.
    public var rotateFailure: String?
    /// An import is under way.
    public var isImporting = false
    /// What was wrong with the last import.
    public var importProblem: ImportProblem?

    /// Nothing under way, nothing wrong.
    public init() {
        // every part starts at its default
    }
}
