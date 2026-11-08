import NgtermCoreFakes
import Testing
import UIKit

@testable import NgtermTerminal

/// The terminal's own layers: a layer per row of the screen, the composition's, and the copies Core Animation makes of
/// them.
@MainActor
@Suite("the terminal's layers")
internal struct TerminalLayersTests {
    private static let side = 8

    // the color of the pixel at `column`, `row` of `layer` drawn into a bitmap of its size, as 0xRRGGBBAA
    private static func pixel(of layer: CALayer, column: Int, row: Int) throws -> UInt32 {
        let width = Int(layer.bounds.width)
        let height = Int(layer.bounds.height)
        let context = try #require(
            CGContext(
                data: nil,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
        )
        // the layer's y-down drawing, as Core Animation sets it up
        context.translateBy(x: 0, y: CGFloat(height))
        context.scaleBy(x: 1, y: -1)
        layer.draw(in: context)
        let bytes = try #require(context.data).assumingMemoryBound(to: UInt8.self)
        let index = (row * width + column) * 4
        return (0..<4).reduce(UInt32(0)) { color, offset in color << 8 | UInt32(bytes[index + offset]) }
    }

    @Test("Core Animation can copy each layer for display, a row's copy standing for the same row")
    internal func copies() throws {
        let rig = TerminalRig(clipboard: nil)
        let rows = rig.view.layer.sublayers?.compactMap { $0 as? TerminalRowLayer } ?? []
        let third = try #require(rows.dropFirst(2).first)
        #expect(TerminalRowLayer(layer: third).index == third.index)
        #expect(third.index == 2)
        let composition = CompositionLayer()
        _ = CompositionLayer(layer: composition)
        let handles = SelectionHandles()
        _ = SelectionHandles(layer: handles)
    }

    @Test("a composition with nothing composed paints the terminal's ground, with no text or underline")
    internal func emptyComposition() throws {
        let layer = CompositionLayer()
        layer.frame = CGRect(x: 0, y: 0, width: Self.side, height: Self.side)
        layer.ground = CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
        layer.mark = CGColor(srgbRed: 0, green: 0, blue: 1, alpha: 1)
        #expect(try Self.pixel(of: layer, column: Self.side / 2, row: Self.side / 2) == 0xFF00_00FF)
        #expect(try Self.pixel(of: layer, column: Self.side / 2, row: Self.side - 1) == 0xFF00_00FF, "no underline")
    }

    @Test("a smaller screen lets go of the rows it no longer has, and a bigger one adds rows")
    internal func rowsFollowTheScreen() {
        let rows = Locked(10)
        let pane = FakePane { since in
            RowBufferWriter(columns: 20, rows: rows.value, seq: since + 1).text(0, "top").bytes()
        }
        let view = TerminalView(pane: pane, palette: .copper, textSize: 13, clipboard: MemoryClipboard(nil))
        view.frame = CGRect(x: 0, y: 0, width: 390, height: 200)
        view.layoutIfNeeded()
        let shown = { view.layer.sublayers?.compactMap { $0 as? TerminalRowLayer }.map(\.index) ?? [] }
        view.drawFrame()
        #expect(shown() == Array(0..<10))
        rows.value = 4
        view.attached()
        view.drawFrame()
        #expect(shown() == Array(0..<4))
        rows.value = 6
        view.attached()
        view.drawFrame()
        #expect(shown() == Array(0..<6))
        #expect(view.screen.text(0) == "top")
    }
}
