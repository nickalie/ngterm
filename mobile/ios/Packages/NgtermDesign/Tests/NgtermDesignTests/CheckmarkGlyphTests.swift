import NgtermDesign
import SwiftUI
import Testing
import UIKit

/// The checkmark status shape as the badge draws it: an origin that sets `--shape checkmark` gets a glyph of its own,
/// not the status's default star or the circle an unknown shape falls back to, light and dark. The bell, another
/// shape, stays its own glyph.
@MainActor
@Suite("checkmark glyph")
internal struct CheckmarkGlyphTests {
    /// A rendered badge: equal only when its size and every pixel are.
    private struct Rendered: Equatable {
        let width: Int
        let height: Int
        let bytes: [UInt8]
    }

    private static let scale: CGFloat = 3
    private static let bytesPerPixel = 4
    private static let bitsPerComponent = 8

    // the badge drawn at 3x on the grouped ground of `scheme`, as its raw RGBA bytes and size
    private static func pixels(of badge: AgentStatusBadge, in scheme: ColorScheme) throws -> Rendered {
        let view =
            badge
            .ngtermTheme()
            .padding()
            .background(Color(uiColor: .systemGroupedBackground))
            .environment(\.colorScheme, scheme)
        let renderer = ImageRenderer(content: view)
        renderer.scale = Self.scale
        let image = try #require(renderer.cgImage, "the badge did not render")
        let bytesPerRow = image.width * Self.bytesPerPixel
        var bytes = [UInt8](repeating: 0, count: bytesPerRow * image.height)
        let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress,
                    width: image.width,
                    height: image.height,
                    bitsPerComponent: Self.bitsPerComponent,
                    bytesPerRow: bytesPerRow,
                    space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                )
            else {
                return false
            }
            context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            return true
        }
        #expect(drawn, "no bitmap context for the badge")
        return Rendered(width: image.width, height: image.height, bytes: bytes)
    }

    @Test("a checkmark status draws neither the star nor the circle", arguments: [ColorScheme.light, .dark])
    internal func checkmarkHasItsOwnGlyph(scheme: ColorScheme) throws {
        let checkmark = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "checkmark"), in: scheme)
        let star = try Self.pixels(of: AgentStatusBadge(status: "completed"), in: scheme)
        let circle = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "circle"), in: scheme)
        #expect(checkmark != star, "the checkmark drew the status's default star")
        #expect(checkmark != circle, "the checkmark drew the circle of an unknown shape")
        let spelled = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "Checkmark"), in: scheme)
        #expect(checkmark == spelled, "the origin's spelling changed the glyph")
    }

    @Test("a bell status draws the bell, not the checkmark", arguments: [ColorScheme.light, .dark])
    internal func bellStaysTheBell(scheme: ColorScheme) throws {
        let bell = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "bell"), in: scheme)
        let checkmark = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "checkmark"), in: scheme)
        let circle = try Self.pixels(of: AgentStatusBadge(status: "completed", shape: "circle"), in: scheme)
        #expect(bell != checkmark)
        #expect(bell != circle)
    }
}
