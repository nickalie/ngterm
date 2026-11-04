import Foundation

/// A value kept as one JSON file, written whole and atomically (a new file renamed over the old one, so a reader sees
/// the old value or the new, never part of one) under the file protection `completeUntilFirstUserAuthentication`:
/// readable from the device's first unlock on, so a background refresh with the phone locked can read it.
///
/// A file that no longer decodes reads as none, and its bytes are kept beside it as `<name>.bad`.
internal struct JSONFile<Value: Codable>: Sendable {
    /// The file.
    internal let url: URL

    /// Where a file that no longer decodes is kept: `<name>.bad`, beside it.
    internal var badCopy: URL {
        url.appendingPathExtension("bad")
    }

    /// The file `name` in `directory`.
    internal init(directory: URL, name: String) {
        url = directory.appending(path: name, directoryHint: .notDirectory)
    }

    /// The value in the file: `nil` when there is no file, or when it no longer decodes, in which case its bytes move
    /// to ``badCopy`` (replacing an earlier one) and the file goes.
    ///
    /// - Throws: ``StoreError/unreadable(reason:)`` when the file is there but can't be read; it is left as it is.
    internal func read() throws(StoreError) -> Value? {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return nil
        } catch {
            throw .unreadable(reason: error.localizedDescription)
        }
        do {
            return try JSONDecoder().decode(Value.self, from: data)
        } catch {
            setAside(data)
            return nil
        }
    }

    /// Writes `value` as the file's new content, atomically.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't; the file keeps its old content.
    internal func write(_ value: Value) throws(StoreError) {
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode(value)
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true,
                attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
            )
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            throw .unwritable(reason: error.localizedDescription)
        }
    }

    // keeps the bytes of a file that no longer decodes, then removes it; when they can't be kept the file stays
    private func setAside(_ data: Data) {
        do {
            try data.write(to: badCopy, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            try FileManager.default.removeItem(at: url)
        } catch {
            // the bytes stay where they were: the file reads as none all the same
        }
    }
}
