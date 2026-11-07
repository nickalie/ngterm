import Foundation

/// A file the user picked to attach: its name, its size when known, and how to read its bytes (off the main actor).
internal struct PickedFile: Sendable {
    internal let name: String
    internal let size: Int?
    internal let read: @Sendable () async throws -> Data
}
