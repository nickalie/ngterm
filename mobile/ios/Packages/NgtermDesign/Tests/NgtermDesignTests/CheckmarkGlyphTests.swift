import NgtermDesign
import SwiftUI
import Testing
import UIKit

/// The checkmark status shape as the badge draws it.
///
/// An origin that sets `--shape checkmark` gets a glyph of its own, not the status's default star or the circle an
/// unknown shape falls back to, light and dark. The bell, another shape, stays its own glyph. The glyph alone is a
/// heavy check: it has ink, it is none of the other shapes, and it paints more than the symbol at the weight of the
/// word.
@MainActor
@Suite("checkmark glyph")
internal struct CheckmarkGlyphTests {
    /// A rendered view: equal only when its size and every pixel are.
    private struct Rendered: Equatable {
        let width: Int
        let height: Int
        let bytes: [UInt8]
    }

    private static let scale: CGFloat = 3
    private static let bytesPerPixel = 4
    private static let bitsPerComponent = 8
    private static let halfAlpha: UInt8 = 128

    // the pixels of `rendered` that are more than half opaque
    private static func ink(of rendered: Rendered) -> Int {
        stride(from: bytesPerPixel - 1, to: rendered.bytes.count, by: bytesPerPixel)
            .count { rendered.bytes[$0] > halfAlpha }
    }

    // the badge drawn at 3x on the grouped ground of `scheme`, as its raw RGBA bytes and size
    private static func pixels(of badge: AgentStatusBadge, in scheme: ColorScheme) throws -> Rendered {
        let rendered = try pixels(
            of: badge.ngtermTheme().padding().background(Color(uiColor: .systemGroupedBackground)),
            in: scheme
        )
        try #require(ink(of: rendered) > 0, "the badge drew nothing, not even its ground")
        return rendered
    }

    // a glyph drawn at 3x on no ground, so that only its own pixels are opaque
    private static func pixels(of glyph: StatusShape, in scheme: ColorScheme) throws -> Rendered {
        try pixels(of: StatusGlyph(glyph, color: .primary).ngtermTheme(), in: scheme)
    }

    // drawn by CoreGraphics: `cgImage` goes through the simulator's Metal, blank on an overloaded host
    private static func pixels(of content: some View, in scheme: ColorScheme) throws -> Rendered {
        let renderer = ImageRenderer(content: content.environment(\.colorScheme, scheme))
        var rendered: Rendered?
        renderer.render(rasterizationScale: Self.scale) { size, draw in
            rendered = bitmap(width: pixelCount(size.width), height: pixelCount(size.height), draw: draw)
        }
        return try #require(rendered, "the view did not render")
    }

    private static func pixelCount(_ points: CGFloat) -> Int {
        Int((points * scale).rounded(.up))
    }

    // the RGBA bytes `draw` paints on a clear bitmap of `width` by `height` pixels, at 3x, or nil without a context
    private static func bitmap(width: Int, height: Int, draw: (CGContext) -> Void) -> Rendered? {
        let bytesPerRow = width * bytesPerPixel
        var bytes = [UInt8](repeating: 0, count: bytesPerRow * height)
        let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress,
                    width: width,
                    height: height,
                    bitsPerComponent: bitsPerComponent,
                    bytesPerRow: bytesPerRow,
                    space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                )
            else {
                return false
            }
            context.scaleBy(x: scale, y: scale)
            draw(context)
            return true
        }
        return drawn ? Rendered(width: width, height: height, bytes: bytes) : nil
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

    @Test("the checkmark glyph alone is a check of its own", arguments: [ColorScheme.light, .dark])
    internal func checkmarkGlyphIsItsOwnShape(scheme: ColorScheme) throws {
        let checkmark = try Self.pixels(of: StatusShape.checkmark, in: scheme)
        #expect(Self.ink(of: checkmark) > 0, "the checkmark glyph drew nothing")
        for other in StatusShape.allCases where other != .checkmark {
            #expect(try Self.pixels(of: other, in: scheme) != checkmark, "the checkmark glyph drew the \(other)")
        }
    }

    @Test("the checkmark glyph is heavier than the symbol at the word's weight", arguments: [ColorScheme.light, .dark])
    internal func checkmarkGlyphIsHeavy(scheme: ColorScheme) throws {
        let heavy = try Self.pixels(of: StatusShape.checkmark, in: scheme)
        let word = try Self.pixels(
            of: Image(systemName: StatusShape.checkmark.symbolName)
                .font(Typography.statusWord)
                .imageScale(.small)
                .foregroundStyle(.primary),
            in: scheme
        )
        let heavyInk = Self.ink(of: heavy)
        let wordInk = Self.ink(of: word)
        #expect(heavyInk > wordInk, "the checkmark glyph is not heavy: \(heavyInk) against \(wordInk)")
    }
}
