import Foundation
import NgtermDesign
import Testing

/// The terminal palettes are the Android app's, value for value.
///
/// This reads Android's `TerminalPalette.kt` (the simulator sees the checkout) and compares every color, so a change
/// to either side fails here until both agree.
@Suite("terminal palettes")
internal struct TerminalPaletteTests {
    private struct Colors: Equatable, CustomStringConvertible {
        let id: String
        let background: RGB
        let foreground: RGB
        let cursor: RGB
        let selectionBackground: RGB
        let ansi: [RGB]

        var description: String {
            "\(id): \(background) \(foreground) \(cursor) \(selectionBackground) \(ansi)"
        }
    }

    private static let kotlin = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TerminalPaletteTests.swift
        .deletingLastPathComponent()  // NgtermDesignTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // NgtermDesign
        .deletingLastPathComponent()  // Packages
        .deletingLastPathComponent()  // ios
        .appendingPathComponent("android/app/src/main/kotlin/com/nickalie/ngterm/android/ui/theme/TerminalPalette.kt")

    // each `val XTerminal = TerminalPalette(...)` of the Kotlin file, up to the next one
    private static func parse(_ source: String) throws -> [Colors] {
        let starts = source.matches(of: /val \w+Terminal =\s*TerminalPalette\(/)
        let ends = starts.dropFirst().map(\.range.lowerBound) + [source.endIndex]
        return try zip(starts, ends).map { start, end in
            try parsePalette(source[start.range.upperBound..<end])
        }
    }

    private static func parsePalette(_ block: Substring) throws -> Colors {
        func role(_ name: String) throws -> RGB {
            let pattern = try Regex("\(name) = Color\\(0xFF([0-9A-Fa-f]{6})\\)")
            let match = try #require(block.firstMatch(of: pattern), "no \(name) in \(block.prefix(80))")
            let hex = try #require(match.output[1].substring)
            return try #require(RGB(parsing: String(hex)))
        }
        let id = try #require(block.firstMatch(of: /id = "([a-z]+)"/)).output.1
        let list = try #require(block.firstMatch(of: /palette\(([^)]*)\)/)).output.1
        let ansi = try list.matches(of: /0x([0-9A-Fa-f]{6})/).map { hex in
            try #require(RGB(parsing: String(hex.output.1)))
        }
        return Colors(
            id: String(id),
            background: try role("background"),
            foreground: try role("foreground"),
            cursor: try role("cursor"),
            selectionBackground: try role("selectionBackground"),
            ansi: ansi
        )
    }

    @Test("the palettes equal Android's, in its order")
    internal func palettesEqualAndroids() throws {
        let android = try Self.parse(String(contentsOf: Self.kotlin, encoding: .utf8))
        let ios = TerminalPalette.builtIn.map { palette in
            Colors(
                id: palette.id,
                background: palette.background,
                foreground: palette.foreground,
                cursor: palette.cursor,
                selectionBackground: palette.selectionBackground,
                ansi: palette.ansi
            )
        }
        #expect(android.count == 3, "parsed \(android.map(\.id)) from \(Self.kotlin.path)")
        #expect(ios == android)
    }

    @Test("the theme's palette is Copper on the dark side and Drafting on the light")
    internal func matchingTheTheme() {
        #expect(TerminalPalette.matching(isDark: true) == .copper)
        #expect(TerminalPalette.matching(isDark: false) == .drafting)
        #expect(TerminalPalette.builtIn.allSatisfy { $0.ansi.count == TerminalPalette.ansiCount })
    }
}
