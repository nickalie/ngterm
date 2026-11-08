import Foundation
import Testing

@testable import NgtermFeatures

/// A photo or video the picker handed over as a file of its own: read into an upload, refused before a byte is read
/// when it is over the cap, and its copy removed either way.
@MainActor
@Suite("picked media")
internal struct PickedMediaTests {
    // the picker's copy, in a directory of its own as the import makes it
    private static func copy(named name: String) throws -> URL {
        let folder = FileManager.default.temporaryDirectory
            .appending(path: "PickedMediaTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder.appending(path: name, directoryHint: .notDirectory)
    }

    @Test("a file within the cap is read whole, and its copy goes")
    internal func read() throws {
        let file = try Self.copy(named: "photo.jpg")
        let bytes = Data("a photo's bytes".utf8)
        try bytes.write(to: file)
        #expect(try PickedMedia(file: file).read() == bytes)
        #expect(!FileManager.default.fileExists(atPath: file.deletingLastPathComponent().path()))
    }

    @Test("a file over the cap is refused as too large, and its copy goes")
    internal func tooLarge() throws {
        let file = try Self.copy(named: "video.mov")
        #expect(FileManager.default.createFile(atPath: file.path(), contents: nil))
        // a sparse file a byte over the cap: its size is all the refusal reads
        let handle = try FileHandle(forWritingTo: file)
        try handle.truncate(atOffset: UInt64(ChatUpload.cap) + 1)
        try handle.close()
        #expect(throws: UploadProblem.tooLarge) {
            try PickedMedia(file: file).read()
        }
        #expect(!FileManager.default.fileExists(atPath: file.deletingLastPathComponent().path()))
    }

    @Test("a copy that is gone fails the read, and leaves nothing behind")
    internal func missing() throws {
        let file = try Self.copy(named: "gone.jpg")
        #expect(throws: (any Error).self) {
            try PickedMedia(file: file).read()
        }
        #expect(!FileManager.default.fileExists(atPath: file.deletingLastPathComponent().path()))
    }
}
