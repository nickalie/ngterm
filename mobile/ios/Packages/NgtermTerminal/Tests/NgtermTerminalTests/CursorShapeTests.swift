import NgtermTerminal
import Testing

/// The cursor's shape as a frame's header names it.
@Suite("the cursor's shape")
internal struct CursorShapeTests {
    @Test(
        "a style byte names its shape, and one this build doesn't know is a block",
        arguments: [
            (UInt8(0), CursorShape.block),
            (1, .bar),
            (2, .underline),
            (3, .hollowBlock),
            (4, .block),
            (UInt8.max, .block),
        ]
    )
    internal func shapes(style: UInt8, shape: CursorShape) {
        #expect(CursorShape(style: style) == shape)
    }
}
