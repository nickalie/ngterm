import CoreTransferable
import Foundation
import UniformTypeIdentifiers

/// A photo or video the photo picker hands over as a file of its own, never as bytes in memory: a long video runs to
/// gigabytes, so it is copied to a temporary file, measured, and read no further than an upload takes
/// (``ChatUpload/cap``).
internal struct PickedMedia: Transferable {
    /// The picker's file copied into a directory of its own, as the picker deletes its own once the import returns.
    internal static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .item) { received in
            let folder = FileManager.default.temporaryDirectory
                .appending(path: "picked-\(UUID().uuidString)", directoryHint: .isDirectory)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let copy = folder.appending(path: received.file.lastPathComponent, directoryHint: .notDirectory)
            try FileManager.default.copyItem(at: received.file, to: copy)
            return Self(file: copy)
        }
    }

    /// The copy, which ``read()`` removes.
    internal let file: URL

    /// The file's bytes, refused before a byte is read when it is over the cap; the copy goes either way.
    internal func read() throws -> Data {
        defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
        let size = (try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
        guard size <= ChatUpload.cap else {
            throw UploadProblem.tooLarge
        }
        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }
        return try handle.read(upToCount: ChatUpload.cap + 1) ?? Data()
    }
}
