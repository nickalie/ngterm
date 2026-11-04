import Foundation

/// A value kept as one JSON file, written whole and atomically (a new file renamed over the old one, so a reader sees
/// the old value or the new, never part of one) under the file protection `completeUntilFirstUserAuthentication`:
/// readable from the device's first unlock on, so a background refresh with the phone locked can read it.
///
/// Nothing a store keeps leaves the device (mobile/android's data extraction rules): each file is excluded from
/// backups, as the keys' private halves and the pins sit in this device's Keychain only, and a restore onto another
/// device that brought the files without them would list keys it can't sign in with and skip the first run.
///
/// A file that no longer decodes reads as none once its bytes are kept beside it as `<name>.bad`; until they can be
/// kept, it reads as one that can't be read, so nothing writes over it.
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

    // an atomic write replaces the file, and the mark with it, so every write marks the new one; a mark that doesn't
    // take fails nothing (the value is written) and the next write tries again
    private static func excludeFromBackup(_ url: URL) {
        var file = url
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? file.setResourceValues(values)
    }

    /// The value in the file: `nil` when there is no file, or when it no longer decodes, in which case its bytes move
    /// to ``badCopy`` (replacing an earlier one) and the file goes.
    ///
    /// - Throws: ``StoreError/unreadable(reason:)`` when the file is there but can't be read, or no longer decodes and
    ///   its bytes can't be copied to ``badCopy``; either way it is left as it is.
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
            try setAside(data)
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
        Self.excludeFromBackup(url)
    }

    // keeps the bytes of a file that no longer decodes, then removes it; when they can't be kept the file stays and
    // reads as unreadable, so that no write replaces it before they are
    private func setAside(_ data: Data) throws(StoreError) {
        do {
            try data.write(to: badCopy, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            throw .unreadable(reason: "it no longer decodes and could not be set aside: \(error.localizedDescription)")
        }
        Self.excludeFromBackup(badCopy)
        do {
            try FileManager.default.removeItem(at: url)
        } catch {
            // the bytes are kept in the copy: the file reads as none, and the next write replaces it
        }
    }
}
