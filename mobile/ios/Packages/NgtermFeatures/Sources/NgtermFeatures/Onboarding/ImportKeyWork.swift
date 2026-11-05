import Foundation

/// What the import screen (``ImportKeyView``) works through: the first run's model, or Settings'.
@MainActor
internal struct ImportKeyWork {
    /// Whether an import runs, and what was wrong with the last one: read as the screen draws, so it follows them.
    internal let state: @MainActor () -> (isBusy: Bool, problem: ImportProblem?)
    /// Imports the key in the bytes given (zeroed once read), opened with the passphrase given.
    internal let importKey: @MainActor (inout Data, String) async -> Void
    /// The chosen file could not be read.
    internal let unreadable: @MainActor () -> Void
}
