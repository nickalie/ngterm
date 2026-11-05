import CoreGraphics

extension CGColor {
    private static let channelMax: CGFloat = 255
    private static let byteMask: UInt32 = 0xFF
    private static let alphaShift: UInt32 = 24
    private static let redShift: UInt32 = 16
    private static let greenShift: UInt32 = 8

    /// The sRGB color a row buffer's `0xAARRGGBB` names.
    internal static func argb(_ value: UInt32) -> CGColor {
        func channel(_ shift: UInt32) -> CGFloat {
            CGFloat((value >> shift) & byteMask) / channelMax
        }
        return CGColor(
            srgbRed: channel(redShift),
            green: channel(greenShift),
            blue: channel(0),
            alpha: channel(alphaShift)
        )
    }
}
