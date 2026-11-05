/// A run's colors as drawn, `0xAARRGGBB`: the defaults resolved from the frame being drawn, swapped for inverse, the
/// text faded for faint.
public struct RunColors: Hashable, Sendable {
    /// Faint text keeps this alpha: half, as Ghostty's `faint-opacity` default.
    private static let faintAlpha: UInt32 = 0x8000_0000
    private static let rgbMask: UInt32 = 0x00FF_FFFF

    /// The text's color.
    public let foreground: UInt32
    /// The ground's color.
    public let background: UInt32

    /// The terminal's defaults, for cells no run covers.
    public init(defaultsOf header: TerminalHeader) {
        foreground = header.defaultForeground
        background = header.defaultBackground
    }

    /// The colors of `run` under `header`: a `0` is the header's default, never an earlier frame's.
    public init(of run: TerminalRun, under header: TerminalHeader) {
        let text = run.foreground == 0 ? header.defaultForeground : run.foreground
        let ground = run.background == 0 ? header.defaultBackground : run.background
        let inverse = run.attributes.contains(.inverse)
        let shown = inverse ? ground : text
        foreground = run.attributes.contains(.faint) ? shown & Self.rgbMask | Self.faintAlpha : shown
        background = inverse ? text : ground
    }
}
