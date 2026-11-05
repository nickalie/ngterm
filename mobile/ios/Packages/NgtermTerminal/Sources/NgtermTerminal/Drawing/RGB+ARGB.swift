import NgtermDesign

extension RGB {
    private static let channelMax = 255.0
    private static let opaque: UInt32 = 0xFF00_0000
    private static let redShift: UInt32 = 16
    private static let greenShift: UInt32 = 8

    /// The color as the core's `0xAARRGGBB`, opaque.
    public var argb: UInt32 {
        func byte(_ channel: Double) -> UInt32 {
            UInt32((channel * Self.channelMax).rounded())
        }
        return Self.opaque | byte(red) << Self.redShift | byte(green) << Self.greenShift | byte(blue)
    }
}
