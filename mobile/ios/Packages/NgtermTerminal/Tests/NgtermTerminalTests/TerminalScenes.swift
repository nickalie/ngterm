import NgtermCoreFakes
import Testing

/// Known screens for the terminal's snapshots (mobile/android's `TerminalScenes`), each a fake pane drawing it at the
/// grid and in the colors the view gives the pane; `TerminalScenesTests` holds each scene to the layout.
///
/// A suite with no tests of its own, so its screens' columns and colors read as test data.
@Suite("terminal scenes")
internal enum TerminalScenes {
    /// A scene: what to pack at a grid and in colors.
    internal typealias Scene = @Sendable (TerminalScene) -> RowBufferWriter

    private static let defaultColumns = 40
    private static let defaultRows = 12
    private static let names = ["blk", "red", "grn", "yel", "blu", "mag", "cyn", "wht"]
    // libghostty-vt's built-in 16 colors, the terminal's before the view gives it a palette
    internal static let ghosttyANSI: [UInt32] = [
        0xFF1D_1F21, 0xFFCC_6666, 0xFFB5_BD68, 0xFFF0_C674, 0xFF81_A2BE, 0xFFB2_94BB, 0xFF8A_BEB7, 0xFFC5_C8C6,
        0xFF66_6666, 0xFFD5_4E53, 0xFFB9_CA4A, 0xFFE7_C547, 0xFF7A_A6DA, 0xFFC3_97D8, 0xFF70_C0B1, 0xFFEA_EAEA,
    ]

    /// A fake pane drawing `scene` at whatever grid and colors the view gives it.
    internal static func pane(_ scene: @escaping Scene) -> FakePane {
        let pane = FakePane()
        pane.frames = { [weak pane] _ in
            let size = pane?.sizes.last
            let drawn = TerminalScene(
                columns: size.map { Int($0.cols) } ?? defaultColumns,
                rows: size.map { Int($0.rows) } ?? defaultRows,
                colors: pane?.colors.last,
                ghosttyANSI: ghosttyANSI
            )
            return scene(drawn).bytes()
        }
        return pane
    }

    /// The 16 ANSI colors as text and as grounds, a 24-bit ramp, and the defaults.
    internal static func colors(_ scene: TerminalScene) -> RowBufferWriter {
        var writer = scene.writer(cursor: FakeCursor(column: 9, row: 7))
        for bright in 0...1 {
            let words = names.indices.map { slot in
                FakeRun(slot * 4, names[slot], foreground: scene.ansi(bright * 8 + slot))
            }
            let grounds = names.indices.map { slot in
                FakeRun(slot * 4, "    ", background: scene.ansi(bright * 8 + slot))
            }
            writer = writer.row(bright, words).row(2 + bright, grounds)
        }
        let ramp = (0..<32).map { step in
            let level = UInt32(step * 255 / 31)
            return FakeRun(step, " ", background: 0xFF00_0000 | level << 16 | 64 << 8 | (255 - level))
        }
        return
            writer
            .row(4, ramp)
            .row(
                5,
                FakeRun(0, "truecolor", foreground: 0xFFE0_A030),
                FakeRun(10, "on its own ground", foreground: 0xFFF0_F0F0, background: 0xFF20_3040)
            )
            .text(6, "default text on the default ground")
            .row(7, FakeRun(0, "$ make", attributes: FakeRun.bold), FakeRun(7, "ok", foreground: scene.ansi(2)))
    }

    /// Every text attribute and underline style.
    internal static func attributes(_ scene: TerminalScene) -> RowBufferWriter {
        let styles = ["single", "double", "curly", "dotted", "dashed"]
        var writer = scene.writer(cursor: FakeCursor(column: 0, row: 11))
            .row(
                0,
                FakeRun(0, "bold", attributes: FakeRun.bold),
                FakeRun(5, "italic", attributes: FakeRun.italic),
                FakeRun(12, "bold italic", attributes: FakeRun.bold | FakeRun.italic)
            )
            .row(
                1,
                FakeRun(0, "faint", attributes: FakeRun.faint),
                FakeRun(6, "faint red", attributes: FakeRun.faint, foreground: scene.ansi(1))
            )
            .row(
                2,
                FakeRun(0, "inverse", attributes: FakeRun.inverse),
                FakeRun(8, "inverse red", attributes: FakeRun.inverse, foreground: scene.ansi(1))
            )
            .row(
                3,
                FakeRun(0, "invisible:"),
                FakeRun(10, "secret", attributes: FakeRun.invisible),
                FakeRun(16, ":end")
            )
            .row(
                4,
                FakeRun(0, "strikethrough", attributes: FakeRun.strikethrough),
                FakeRun(14, "overline", attributes: FakeRun.overline)
            )
        for (index, style) in styles.enumerated() {
            writer = writer.row(5 + index, FakeRun(0, "underline \(style)", underline: UInt8(index + 1)))
        }
        return writer.row(
            10,
            FakeRun(0, "curly in red", underline: 3, underlineColor: scene.ansi(9)),
            FakeRun(13, "ground", foreground: scene.ansi(15), background: scene.ansi(4))
        )
    }

    /// Wide characters, clusters, emoji and box drawing on the grid.
    internal static func wide(_ scene: TerminalScene) -> RowBufferWriter {
        scene.writer(cursor: FakeCursor(column: 4, row: 1))
            .row(
                0,
                FakeRun(0, "|"),
                FakeRun(1, "中文字", kind: FakeRun.wide),
                FakeRun(7, "|"),
                FakeRun(8, "かな", kind: FakeRun.wide),
                FakeRun(12, "|")
            )
            .row(
                1,
                FakeRun(0, "ab"),
                FakeRun(2, "漢", kind: FakeRun.wide),
                FakeRun(4, "字", kind: FakeRun.wide),
                FakeRun(6, "cd")
            )
            .row(
                2,
                FakeRun(0, "caf"),
                FakeRun(3, "e\u{301}", kind: FakeRun.cluster),
                FakeRun(4, " n"),
                FakeRun(6, "o\u{308}", kind: FakeRun.cluster)
            )
            .row(
                3,
                FakeRun(0, "|"),
                FakeRun(1, "\u{1F44D}\u{1F3FD}", kind: FakeRun.wide | FakeRun.cluster),
                FakeRun(3, "|"),
                FakeRun(4, "\u{1F680}", kind: FakeRun.wide),
                FakeRun(6, "|")
            )
            .text(4, "┌──────┬──────┐")
            .text(5, "│ box  │ ok ✓ │")
            .text(6, "└──────┴──────┘")
            .text(7, "▁▂▃▄▅▆▇█ ░▒▓ → ← ↑ ↓")
            .text(8, "0O 1lI| {}[]() == != => ->")
    }

    /// A few lines selected from the middle of one to the middle of another.
    internal static func selection(_ scene: TerminalScene) -> RowBufferWriter {
        scene.writer(cursor: FakeCursor(column: 2, row: 5))
            .text(0, "$ git log --oneline")
            .row(1, FakeRun(0, "67f0ca4", foreground: scene.ansi(3)), FakeRun(8, "fix: address review"))
            .row(2, FakeRun(0, "d9c7ebb", foreground: scene.ansi(3)), FakeRun(8, "feat: sessions list"))
            .row(3, FakeRun(0, "e9ee473", foreground: scene.ansi(3)), FakeRun(8, "feat: first connect"))
            .text(5, "$ ")
            .select(1, first: 8, last: scene.columns - 1)
            .select(2, first: 0, last: scene.columns - 1)
            .select(3, first: 0, last: 11)
    }

    /// A prompt with the cursor in `style` (0 block, 1 bar, 2 underline, 3 hollow block) on a colored character.
    internal static func cursor(_ style: UInt8) -> Scene {
        { scene in
            scene.writer(cursor: FakeCursor(column: 10, row: 0, style: style))
                .row(
                    0,
                    FakeRun(0, "$ echo hi"),
                    FakeRun(10, "x", foreground: scene.ansi(2)),
                    FakeRun(12, "中", kind: FakeRun.wide)
                )
        }
    }

    /// The cursor on the second column of a wide character: drawn over both its columns.
    internal static func cursorOnWide(_ scene: TerminalScene) -> RowBufferWriter {
        scene.writer(cursor: FakeCursor(column: 3, row: 0, wideTail: true))
            .row(0, FakeRun(0, "$ "), FakeRun(2, "中", kind: FakeRun.wide), FakeRun(4, "文", kind: FakeRun.wide))
    }
}
