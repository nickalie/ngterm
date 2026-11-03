package com.nickalie.ngterm.android.ui.terminal

import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.PaneColors

/**
 * Known screens for the terminal's screenshots, packed as the core packs them in the colors the view gave the pane
 * (the palette's, resolved as the Rust terminal resolves ANSI colors before packing) and at the grid it gave.
 */
object TerminalScenes {
    /** A fake pane drawing [scene] at whatever grid and colors the view gives it, waking the view as they come. */
    fun pane(scene: (Scene) -> RowBufferWriter): FakePane {
        val pane = FakePane()
        pane.frames = { _ ->
            val size = pane.sizes.lastOrNull()
            val colors = pane.colors.lastOrNull()
            scene(Scene(size?.cols?.toInt() ?: COLUMNS, size?.rows?.toInt() ?: ROWS, colors)).bytes()
        }
        return pane
    }

    /** What a scene is drawn at: the grid and the colors the view gave. */
    class Scene(
        val columns: Int,
        val rows: Int,
        private val colors: PaneColors?,
    ) {
        val foreground: Int = colors?.foreground?.toInt() ?: RowBufferWriter.GHOSTTY_FOREGROUND
        val background: Int = colors?.background?.toInt() ?: RowBufferWriter.GHOSTTY_BACKGROUND

        /** ANSI color [slot], set on the cell (as the Rust terminal packs it, resolved). */
        fun ansi(slot: Int): Int = colors?.palette?.getOrNull(slot)?.toInt() ?: GHOSTTY_ANSI[slot]

        fun writer(cursor: FakeCursor? = null) =
            RowBufferWriter(columns, rows, cursor = cursor).colors(foreground, background, colors?.cursor?.toInt() ?: 0)
    }

    private const val COLUMNS = 40
    private const val ROWS = 12
    private val ANSI_NAMES = listOf("blk", "red", "grn", "yel", "blu", "mag", "cyn", "wht")
    private val GHOSTTY_ANSI =
        listOf(
            0x1D1F21,
            0xCC6666,
            0xB5BD68,
            0xF0C674,
            0x81A2BE,
            0xB294BB,
            0x8ABEB7,
            0xC5C8C6,
            0x666666,
            0xD54E53,
            0xB9CA4A,
            0xE7C547,
            0x7AA6DA,
            0xC397D8,
            0x70C0B1,
            0xEAEAEA,
        ).map { (0xFF000000 or it.toLong()).toInt() }

    /** The 16 ANSI colors as text and as grounds, a 24-bit ramp, and the defaults. */
    fun colors(scene: Scene): RowBufferWriter {
        val writer = scene.writer(FakeCursor(9, 7))
        for (bright in 0..1) {
            val names =
                ANSI_NAMES.mapIndexed {
                    i,
                    name,
                    ->
                    FakeRun(i * 4, name, foreground = scene.ansi(bright * 8 + i))
                }
            writer.row(bright, *names.toTypedArray())
            val grounds = (0..7).map { i -> FakeRun(i * 4, "    ", background = scene.ansi(bright * 8 + i)) }
            writer.row(2 + bright, *grounds.toTypedArray())
        }
        val ramp =
            (0 until 32).map { i ->
                val level = i * 255 / 31
                FakeRun(
                    i,
                    " ",
                    background = (0xFF000000 or (level.toLong() shl 16) or (64L shl 8) or (255L - level)).toInt(),
                )
            }
        writer.row(4, *ramp.toTypedArray())
        writer.row(
            5,
            FakeRun(0, "truecolor", foreground = 0xFFE0A030.toInt()),
            FakeRun(10, "on its own ground", background = 0xFF203040.toInt(), foreground = 0xFFF0F0F0.toInt()),
        )
        writer.text(6, "default text on the default ground")
        writer.row(7, FakeRun(0, "$ make", attributes = FakeRun.BOLD), FakeRun(7, "ok", foreground = scene.ansi(2)))
        return writer
    }

    /** Every text attribute and underline style. */
    fun attributes(scene: Scene): RowBufferWriter {
        val writer = scene.writer(FakeCursor(0, 11))
        writer.row(
            0,
            FakeRun(0, "bold", attributes = FakeRun.BOLD),
            FakeRun(5, "italic", attributes = FakeRun.ITALIC),
            FakeRun(
                12,
                "bold italic",
                attributes =
                    FakeRun.BOLD or FakeRun.ITALIC,
            ),
        )
        writer.row(
            1,
            FakeRun(0, "faint", attributes = FakeRun.FAINT),
            FakeRun(6, "faint red", attributes = FakeRun.FAINT, foreground = scene.ansi(1)),
        )
        writer.row(
            2,
            FakeRun(0, "inverse", attributes = FakeRun.INVERSE),
            FakeRun(8, "inverse red", attributes = FakeRun.INVERSE, foreground = scene.ansi(1)),
        )
        writer.row(
            3,
            FakeRun(0, "invisible:"),
            FakeRun(10, "secret", attributes = FakeRun.INVISIBLE),
            FakeRun(16, ":end"),
        )
        writer.row(
            4,
            FakeRun(0, "strikethrough", attributes = FakeRun.STRIKETHROUGH),
            FakeRun(14, "overline", attributes = FakeRun.OVERLINE),
        )
        val styles = listOf("single", "double", "curly", "dotted", "dashed")
        styles.forEachIndexed { i, style -> writer.row(5 + i, FakeRun(0, "underline $style", underline = i + 1)) }
        writer.row(
            10,
            FakeRun(0, "curly in red", underline = 3, underlineColor = scene.ansi(9)),
            FakeRun(13, "ground", background = scene.ansi(4), foreground = scene.ansi(15)),
        )
        return writer
    }

    /** Wide characters, clusters, emoji and box drawing on the grid. */
    fun wide(scene: Scene): RowBufferWriter {
        val writer = scene.writer(FakeCursor(4, 1))
        writer.row(
            0,
            FakeRun(0, "|"),
            FakeRun(1, "中文字", kind = FakeRun.WIDE),
            FakeRun(7, "|"),
            FakeRun(8, "かな", kind = FakeRun.WIDE),
            FakeRun(12, "|"),
        )
        writer.row(
            1,
            FakeRun(0, "ab"),
            FakeRun(2, "漢", kind = FakeRun.WIDE),
            FakeRun(4, "字", kind = FakeRun.WIDE),
            FakeRun(6, "cd"),
        )
        writer.row(
            2,
            FakeRun(0, "caf"),
            FakeRun(3, "é", kind = FakeRun.CLUSTER),
            FakeRun(4, " n"),
            FakeRun(6, "ö", kind = FakeRun.CLUSTER),
        )
        writer.row(
            3,
            FakeRun(0, "|"),
            FakeRun(1, "👍🏽", kind = FakeRun.WIDE or FakeRun.CLUSTER),
            FakeRun(3, "|"),
            FakeRun(4, "🚀", kind = FakeRun.WIDE),
            FakeRun(6, "|"),
        )
        writer.text(4, "┌──────┬──────┐")
        writer.text(5, "│ box  │ ok ✓ │")
        writer.text(6, "└──────┴──────┘")
        writer.text(7, "▁▂▃▄▅▆▇█ ░▒▓ → ← ↑ ↓")
        writer.text(8, "0O 1lI| {}[]() == != => ->")
        return writer
    }

    /** A few lines selected from the middle of one to the middle of another. */
    fun selection(scene: Scene): RowBufferWriter {
        val writer = scene.writer(FakeCursor(2, 5))
        writer.text(0, "$ git log --oneline")
        writer.row(1, FakeRun(0, "67f0ca4", foreground = scene.ansi(3)), FakeRun(8, "fix: address review"))
        writer.row(2, FakeRun(0, "d9c7ebb", foreground = scene.ansi(3)), FakeRun(8, "feat: sessions list"))
        writer.row(3, FakeRun(0, "e9ee473", foreground = scene.ansi(3)), FakeRun(8, "feat: first connect"))
        writer.text(5, "$ ")
        writer.select(1, 8, scene.columns - 1).select(2, 0, scene.columns - 1).select(3, 0, 11)
        return writer
    }

    /** A prompt with the cursor in [style] (0 block, 1 bar, 2 underline, 3 hollow block) on a wide character's tail. */
    fun cursor(style: Int): (Scene) -> RowBufferWriter =
        { scene ->
            scene
                .writer(FakeCursor(10, 0, style = style))
                .row(
                    0,
                    FakeRun(0, "$ echo hi"),
                    FakeRun(10, "x", foreground = scene.ansi(2)),
                    FakeRun(12, "中", kind = FakeRun.WIDE),
                )
        }

    /** The cursor on the second column of a wide character: drawn over both its columns. */
    fun cursorOnWide(scene: Scene): RowBufferWriter =
        scene
            .writer(FakeCursor(3, 0, wideTail = true))
            .row(0, FakeRun(0, "$ "), FakeRun(2, "中", kind = FakeRun.WIDE), FakeRun(4, "文", kind = FakeRun.WIDE))
}
