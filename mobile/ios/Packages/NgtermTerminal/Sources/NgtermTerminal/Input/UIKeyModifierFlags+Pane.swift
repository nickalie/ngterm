import UIKit

extension UIKeyModifierFlags {
    private static let shiftBit: UInt8 = 1
    private static let controlBit: UInt8 = 2
    private static let altBit: UInt8 = 4
    private static let superBit: UInt8 = 8

    /// The modifiers held as a pane's bits: shift 1, control 2, option (alt) 4, command (super) 8.
    public var paneBits: UInt8 {
        var bits: UInt8 = 0
        if contains(.shift) {
            bits |= Self.shiftBit
        }
        if contains(.control) {
            bits |= Self.controlBit
        }
        if contains(.alternate) {
            bits |= Self.altBit
        }
        if contains(.command) {
            bits |= Self.superBit
        }
        return bits
    }
}
