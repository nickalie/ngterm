/// Why an upload stored nothing.
public enum UploadProblem: Error, Equatable, Sendable {
    /// The machine or the link said no: in its words.
    case refused(reason: String)
    /// Over ``ChatUpload/cap``.
    case tooLarge
    /// The device couldn't read the file it was given.
    case unreadable
}
