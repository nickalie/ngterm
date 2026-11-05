import Foundation

/// Little-endian, unaligned numbers and UTF-8 text read off the front of a buffer, as the row buffer packs them.
///
/// It reads the bytes in place, so it lives only inside the `withUnsafeBytes` that lends them.
internal struct ByteReader {
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0

    /// The bytes not read yet.
    internal var remaining: Int {
        bytes.count - offset
    }

    /// A reader of `bytes` from their start.
    internal init(_ bytes: UnsafeRawBufferPointer) {
        self.bytes = bytes
    }

    /// Fails unless `count` more bytes follow.
    internal func need(_ count: Int) throws(MalformedFrame) {
        guard count >= 0, remaining >= count else {
            throw MalformedFrame.cutShort
        }
    }

    /// The next `count` bytes, as they are.
    internal mutating func bytes(_ count: Int) throws(MalformedFrame) -> UnsafeRawBufferPointer {
        try need(count)
        defer { offset += count }
        return UnsafeRawBufferPointer(rebasing: bytes[offset..<(offset + count)])
    }

    /// The next number of type `type`.
    internal mutating func number<Number: FixedWidthInteger>(_ type: Number.Type) throws(MalformedFrame) -> Number {
        try need(MemoryLayout<Number>.size)
        defer { offset += MemoryLayout<Number>.size }
        return Number(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: type))
    }

    /// The next `u8`.
    internal mutating func u8() throws(MalformedFrame) -> UInt8 {
        try number(UInt8.self)
    }

    /// The next `u16`, as an `Int`.
    internal mutating func u16() throws(MalformedFrame) -> Int {
        Int(try number(UInt16.self))
    }

    /// The next `u32`.
    internal mutating func u32() throws(MalformedFrame) -> UInt32 {
        try number(UInt32.self)
    }

    /// The next `u64`.
    internal mutating func u64() throws(MalformedFrame) -> UInt64 {
        try number(UInt64.self)
    }

    /// The next `count` bytes as UTF-8 text; U+FFFD for ill-formed text, which the Rust writer never packs.
    internal mutating func text(_ count: Int) throws(MalformedFrame) -> String {
        String(bytes: try bytes(count), encoding: .utf8) ?? "\u{FFFD}"
    }
}
