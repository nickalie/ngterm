import Foundation

extension Data {
    /// A copy of `source` in memory of its own, which is zeroed when the copy's last user lets it go.
    internal static func wiping(copyOf source: Data) -> Self {
        let count = source.count
        guard count > 0 else {
            return Self()
        }
        let bytes = UnsafeMutableRawPointer.allocate(byteCount: count, alignment: 1)
        source.copyBytes(to: bytes.assumingMemoryBound(to: UInt8.self), count: count)
        return Self(
            bytesNoCopy: bytes,
            count: count,
            deallocator: .custom { pointer, size in
                memset_s(pointer, size, 0, size)
                pointer.deallocate()
            }
        )
    }

    /// Zeroes the bytes in place, then empties the value.
    internal mutating func wipe() {
        resetBytes(in: startIndex..<endIndex)
        removeAll()
    }
}
