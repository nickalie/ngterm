/// A file the composer attaches, on its way to the machine or stopped short of it.
public enum ChatUpload: Equatable, Sendable {
    /// Why the file didn't reach the machine, until the user dismisses it.
    case failed(name: String, problem: UploadProblem)
    /// `sent` of `total` bytes have gone; `isSaving` once the core has told every one gone.
    ///
    /// The machine has the whole file then and stores it, and a cancel could no longer take it back, so none is
    /// offered.
    case sending(name: String, sent: UInt64, total: UInt64, isSaving: Bool)

    /// The most bytes a file the composer attaches may hold: the origin's cap (`ngtermctl agent upload`), 32 MiB.
    public static let cap = 33_554_432

    /// The file's name, as the user picked it.
    public var name: String {
        switch self {
        case .sending(let name, _, _, _), .failed(let name, _):
            name
        }
    }

    /// Whether it is on its way.
    public var isSending: Bool {
        if case .sending = self {
            return true
        }
        return false
    }
}
