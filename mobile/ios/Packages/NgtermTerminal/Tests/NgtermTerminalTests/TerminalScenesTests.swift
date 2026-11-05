import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import Testing

/// The snapshots' known screens follow the layout, in every palette.
@Suite("the terminal's known screens")
internal struct TerminalScenesTests {
    @Test("every scene packs a frame the reader takes, its runs left to right inside the grid")
    internal func scenesFollowTheLayout() throws {
        let themes = [nil] + TerminalPalette.builtIn.map(\.paneColors)
        let scenes =
            [
                TerminalScenes.cursor(0), TerminalScenes.cursor(1), TerminalScenes.cursor(2), TerminalScenes.cursor(3),
            ] + [
                { TerminalScenes.colors($0) },
                { TerminalScenes.attributes($0) },
                { TerminalScenes.wide($0) },
                { TerminalScenes.selection($0) },
                { TerminalScenes.cursorOnWide($0) },
            ]
        for scene in scenes {
            for theme in themes {
                let drawn = TerminalScene(columns: 50, rows: 12, colors: theme, ghosttyANSI: TerminalScenes.ghosttyANSI)
                let frame = try RowBufferReader.read(scene(drawn).bytes())
                let ground = theme?.background ?? RowBufferWriter.ghosttyBackground
                #expect(frame.header.columns == 50)
                #expect(frame.header.defaultBackground == ground)
                for row in frame.rows {
                    let starts = row.runs.map(\.column)
                    let ends = row.runs.map(\.end)
                    let inOrder = zip(ends, starts.dropFirst()).allSatisfy { $0 <= $1 }
                    let inside = (ends.max() ?? 0) <= frame.header.columns
                    #expect(row.index < frame.header.rows)
                    #expect(inOrder)
                    #expect(inside)
                }
            }
        }
    }

    @Test("a scene's ANSI colors are the palette's, or the terminal's own before it has one")
    internal func ansiColors() {
        let themed = TerminalScene(columns: 1, rows: 1, colors: TerminalPalette.copper.paneColors, ghosttyANSI: [])
        let bare = TerminalScene(columns: 1, rows: 1, colors: nil, ghosttyANSI: TerminalScenes.ghosttyANSI)
        let copperRed = TerminalPalette.copper.ansi[1].argb
        let ghosttyRed = TerminalPalette.ghostty.ansi[1].argb
        #expect(themed.ansi(1) == copperRed)
        #expect(bare.ansi(1) == ghosttyRed)
    }
}
