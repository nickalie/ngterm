/// Why a store could not read or write its file.
public enum StoreError: Error, Equatable, Sendable, CustomStringConvertible {
    /// The file is there but could not be read (before the device's first unlock, say): the store leaves it as it is.
    case unreadable(reason: String)
    /// The file could not be written: the store keeps what it had.
    case unwritable(reason: String)

    /// What went wrong, in words.
    public var description: String {
        switch self {
        case .unreadable(let reason):
            "the store could not be read: \(reason)"

        case .unwritable(let reason):
            "the store could not be written: \(reason)"
        }
    }
}
