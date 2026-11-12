import Foundation

/// Ghostty's `clipboard-multipart.py` data (crates/ngterm-ghostty-vt/tests/data/clipboard).
internal enum ClipboardFixtures {
    /// A 2x2 red PNG, 73 bytes.
    internal static let redPNG =
        Data(
            base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167"
                + "WwAAAABJRU5ErkJggg=="
        ) ?? Data()
}
