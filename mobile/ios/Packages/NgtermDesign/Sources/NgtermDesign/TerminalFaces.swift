/// The PostScript names of the terminal's four faces.
public struct TerminalFaces: Hashable, Sendable {
    /// The regular face.
    public let regular: String
    /// The bold face.
    public let bold: String
    /// The italic face.
    public let italic: String
    /// The bold italic face.
    public let boldItalic: String

    /// Every face, regular first.
    public var all: [String] {
        [regular, bold, italic, boldItalic]
    }
}
