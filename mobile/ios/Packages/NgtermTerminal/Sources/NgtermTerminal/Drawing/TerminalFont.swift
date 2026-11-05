import CoreText
import NgtermCore
import NgtermDesign
import UIKit

/// The terminal's font at one size: a face per text style (regular, bold, italic, bold italic) and the cell they make.
///
/// The cell is as wide as the regular face's advance, unrounded, so a run of the font's own glyphs drawn as one line
/// lands on the grid, and as tall as its ascent and descent, rounded up to whole points, so rows sit on whole points.
/// A character the font lacks (CJK, emoji, most symbols) falls back to the system's fonts, which Core Text does by
/// itself. It is mobile/android's `TerminalFont`.
public final class TerminalFont {
    // the faces' order: regular, bold, italic, bold italic
    private static let boldOffset = 1
    private static let italicOffset = 2
    private static let halves: CGFloat = 2

    /// The text size, in points.
    public let size: CGFloat
    /// A cell's width, in points.
    public let cellWidth: CGFloat
    /// A cell's height, in whole points.
    public let cellHeight: CGFloat
    /// The baseline's distance from a cell's top, in points.
    public let baseline: CGFloat
    private let faces: [CTFont]

    /// The font at `size` points in `faces`, the bundled JetBrains Mono by default; `nil` is the system's monospace.
    public init(size: CGFloat, faces: TerminalFaces? = Fonts.terminalFaces) {
        self.size = size
        let made = faces.map { Self.bundled($0, size: size) } ?? Self.system(size: size)
        self.faces = made
        let regular = made[0]
        let height = CTFontGetAscent(regular) + CTFontGetDescent(regular)
        cellHeight = max(height.rounded(.up), 1)
        baseline = (cellHeight - height) / Self.halves + CTFontGetAscent(regular)
        cellWidth = Self.advance(of: "M", in: regular)
    }

    private static func bundled(_ faces: TerminalFaces, size: CGFloat) -> [CTFont] {
        Fonts.register()
        return faces.all.map { CTFontCreateWithName($0 as CFString, size, nil) }
    }

    private static func system(size: CGFloat) -> [CTFont] {
        let regular = UIFont.monospacedSystemFont(ofSize: size, weight: .regular) as CTFont
        let bold = UIFont.monospacedSystemFont(ofSize: size, weight: .bold) as CTFont
        func italic(_ font: CTFont) -> CTFont {
            CTFontCreateCopyWithSymbolicTraits(font, size, nil, .traitItalic, .traitItalic) ?? font
        }
        return [regular, bold, italic(regular), italic(bold)]
    }

    private static func advance(of character: Character, in font: CTFont) -> CGFloat {
        var characters = Array(String(character).utf16)
        var glyphs = [CGGlyph](repeating: 0, count: characters.count)
        CTFontGetGlyphsForCharacters(font, &characters, &glyphs, characters.count)
        return CTFontGetAdvancesForGlyphs(font, .horizontal, &glyphs, nil, glyphs.count)
    }

    /// The top of row `row`, in points.
    public func top(_ row: Int) -> CGFloat {
        CGFloat(row) * cellHeight
    }

    /// The left edge of column `column`, in points.
    public func left(_ column: Int) -> CGFloat {
        CGFloat(column) * cellWidth
    }

    /// The face for a run's `attributes`: bold and italic pick it.
    public func face(for attributes: TerminalRun.Attributes) -> CTFont {
        let bold = attributes.contains(.bold) ? Self.boldOffset : 0
        let italic = attributes.contains(.italic) ? Self.italicOffset : 0
        return faces[bold + italic]
    }

    /// The grid of whole cells that fits `size` points, its cells measured in pixels at `scale`; `nil` when not one
    /// cell fits.
    public func grid(fitting size: CGSize, scale: CGFloat) -> PaneSize? {
        let most = CGFloat(UInt16.max)
        let columns = min((size.width / cellWidth).rounded(.down), most)
        let rows = min((size.height / cellHeight).rounded(.down), most)
        guard columns >= 1, rows >= 1 else {
            return nil
        }
        return PaneSize(
            cols: UInt16(columns),
            rows: UInt16(rows),
            cellWidth: UInt32((cellWidth * scale).rounded()),
            cellHeight: UInt32((cellHeight * scale).rounded())
        )
    }

    deinit {
        // nothing to release
    }
}
