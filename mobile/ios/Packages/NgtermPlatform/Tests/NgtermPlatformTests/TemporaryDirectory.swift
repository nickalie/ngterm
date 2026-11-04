import Foundation

/// A directory of the test's own under the temporary directory, which ``remove()`` deletes with what it holds.
internal struct TemporaryDirectory {
    /// The directory.
    internal let url: URL

    /// A new, empty directory.
    internal init() throws {
        url = FileManager.default.temporaryDirectory.appending(path: "NgtermPlatformTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    }

    /// The file `name` in the directory.
    internal func file(_ name: String) -> URL {
        url.appending(path: name, directoryHint: .notDirectory)
    }

    /// The names of the files in the directory, sorted.
    internal func names() throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: url.path).sorted()
    }

    /// Lets the test change the directory again, then deletes it with what it holds.
    internal func remove() {
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        try? FileManager.default.removeItem(at: url)
    }
}
