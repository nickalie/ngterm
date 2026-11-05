/// A buffer that does not follow the row buffer's layout.
public enum MalformedFrame: Error, Hashable, CustomStringConvertible {
    /// It ends before what it says follows.
    case cutShort
    /// It doesn't start with the magic `NGRB`.
    case notARowBuffer
    /// It is another version of the layout than this reader's.
    case version(UInt16)

    /// What is wrong with it, as Android's reader says it.
    public var description: String {
        switch self {
        case .cutShort:
            "row buffer cut short"

        case .notARowBuffer:
            "not a row buffer"

        case .version(let version):
            "row buffer version \(version), not \(RowBufferReader.version)"
        }
    }
}
