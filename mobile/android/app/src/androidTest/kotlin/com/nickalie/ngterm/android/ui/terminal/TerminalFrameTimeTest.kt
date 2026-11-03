package com.nickalie.ngterm.android.ui.terminal

import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.FrameMetrics
import android.view.Window
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.PaneInputException
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.Collections
import java.util.concurrent.TimeUnit

/**
 * The frame budget of a full-screen `cat` of a large file (plan > Task 15): the view draws a pane whose every frame
 * scrolls the whole grid by a few lines of a long file, as fast as it can take them, and the window's frame metrics
 * are logged (tag `TerminalFrames`) as percentiles. The pane is a fake that packs frames as the core does, so this is
 * the app's half: reading, laying out and drawing; the core's half is `rows/tests.rs`'s `packing_a_full_screen_cat`.
 * It checks only that frames came; the numbers are for reading.
 */
@RunWith(AndroidJUnit4::class)
class TerminalFrameTimeTest {
    /** A line of the "file": 40 to 120 columns of text, a colored line number every fifth line. */
    private fun line(index: Int): List<FakeRun> {
        val width = 40 + (index * 37) % 81
        val text = String(CharArray(width) { i -> ALPHABET[(index + i * 7) % ALPHABET.length] })
        return if (index % 5 == 0) {
            listOf(FakeRun(0, "%6d".format(index), foreground = COLORS[index % COLORS.size]), FakeRun(7, text))
        } else {
            listOf(FakeRun(0, text))
        }
    }

    /** The grid's rows after [first] lines of the file scrolled by, each line one row, cut at [columns]. */
    private fun screen(
        first: Int,
        columns: Int,
        rows: Int,
        seq: Long,
    ): ByteArray {
        val writer = RowBufferWriter(columns, rows, seq = seq, full = false, cursor = FakeCursor(0, rows - 1))
        var row = 0
        var index = first
        while (row < rows) {
            // a wrapped line's rest is a row of its own; a run past the edge would be the next row's in the real
            // terminal, which this cuts at the edge for a frame of whole rows
            val runs = line(index++).map { run -> run.copy(text = run.text.take(maxOf(0, columns - run.column))) }
            writer.row(row++, *runs.filter { it.text.isNotEmpty() }.toTypedArray())
        }
        return writer.bytes()
    }

    @Test
    fun aFullScreenCatKeepsWithinTheFrameBudget() {
        val pane = FakePane()
        var seq = 1L
        pane.frames = { _ ->
            val grid = pane.sizes.lastOrNull() ?: throw PaneInputException.Closed()
            seq++
            val bytes = screen((seq * LINES_PER_FRAME).toInt(), grid.cols.toInt(), grid.rows.toInt(), seq)
            // the output never stops: the terminal has a new frame as soon as this one is taken
            pane.onWake?.invoke()
            bytes
        }
        val metrics = Collections.synchronizedList(mutableListOf<LongArray>())
        val thread = HandlerThread("frame-metrics").apply { start() }
        val listener =
            Window.OnFrameMetricsAvailableListener { _, frame, _ ->
                metrics +=
                    longArrayOf(
                        frame.getMetric(FrameMetrics.TOTAL_DURATION),
                        frame.getMetric(FrameMetrics.LAYOUT_MEASURE_DURATION) +
                            frame.getMetric(FrameMetrics.DRAW_DURATION),
                        frame.getMetric(FrameMetrics.SYNC_DURATION) +
                            frame.getMetric(FrameMetrics.COMMAND_ISSUE_DURATION),
                        gpu(frame),
                    )
            }
        // the activity's own composition, on the display's real frames (a compose test rule would drive its clock)
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setContent {
                    val state = remember(pane) { TerminalState(pane) }
                    DisposableEffect(pane) {
                        pane.onWake = state::dirty
                        onDispose { pane.onWake = null }
                    }
                    NgtermTheme { TerminalView(state, Modifier.fillMaxSize()) }
                }
            }
            Thread.sleep(TimeUnit.SECONDS.toMillis(SETTLE_SECONDS))
            scenario.onActivity { it.window.addOnFrameMetricsAvailableListener(listener, Handler(thread.looper)) }
            Thread.sleep(TimeUnit.SECONDS.toMillis(RUN_SECONDS))
            scenario.onActivity { it.window.removeOnFrameMetricsAvailableListener(listener) }
        }
        thread.quitSafely()

        val frames = synchronized(metrics) { metrics.toList() }
        assertTrue("frames were drawn: ${frames.size}", frames.size > MIN_FRAMES)
        val grid = pane.sizes.last()
        val report =
            listOf("total", "UI thread (measure, layout, draw)", "render thread (sync, issue)", "GPU")
                .mapIndexed { metric, name -> "$name: ${percentiles(frames.map { it[metric] })}" }
        // the app's work per frame: what the UI and render threads spend; the GPU's is the device's
        val over = frames.count { it[1] + it[2] > BUDGET_NS }
        Log.i(
            TAG,
            "${frames.size} frames in $RUN_SECONDS s at ${grid.cols} x ${grid.rows}, ${seq - 1} pane frames read, " +
                "$over with over 16.7 ms of UI and render thread work\n  " + report.joinToString("\n  "),
        )
    }

    /** The GPU's time on [frame], which Android reports from 12 on. */
    private fun gpu(frame: FrameMetrics): Long =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) frame.getMetric(FrameMetrics.GPU_DURATION) else 0

    private fun percentiles(nanos: List<Long>): String {
        val sorted = nanos.sorted()

        fun at(percent: Int) = "%.1f".format(sorted[(sorted.size - 1) * percent / 100] / 1e6)
        return "p50 ${at(50)} ms, p90 ${at(90)} ms, p99 ${at(99)} ms, max ${at(100)} ms"
    }

    private companion object {
        const val TAG = "TerminalFrames"
        const val SETTLE_SECONDS = 2L
        const val RUN_SECONDS = 10L
        const val MIN_FRAMES = 30
        const val LINES_PER_FRAME = 7
        const val BUDGET_NS = 16_666_667L
        const val ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789 .,;:(){}"
        val COLORS = listOf(0xFFE5675C, 0xFF8CC084, 0xFFE3B65A, 0xFF6FA3C8, 0xFFC08BB5, 0xFF5FBFAA).map { it.toInt() }
    }
}
