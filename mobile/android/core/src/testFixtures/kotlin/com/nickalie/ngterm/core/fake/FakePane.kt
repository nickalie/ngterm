package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.NoHandle
import com.nickalie.ngterm.core.ffi.Pane
import com.nickalie.ngterm.core.ffi.PaneColors
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.PaneSize
import java.util.Collections

/**
 * A [Pane] with no native library behind it, for the views' tests: [frame] answers with what [frames] packs (a
 * [RowBufferWriter]'s bytes) for the seq asked, and throws [PaneInputException.Closed] while it is `null`; [resize]
 * and [setColors] are recorded in [sizes] and [colors], and each seq asked in [asked]. As the real terminal wakes
 * (and the core reports `PaneDirty`) after a resize or new colors, both call [onWake]. Every method may be called
 * from any thread. The input methods are the real handle's, which need the library: a test of input overrides them.
 */
open class FakePane(
    private val paneId: String = "pane",
    @Volatile var frames: ((since: ULong) -> ByteArray)? = null,
) : Pane(NoHandle) {
    /** Runs after [resize] and [setColors]: a test points it at the view's `dirty`. */
    @Volatile var onWake: (() -> Unit)? = null

    /** The grids [resize] was given, in order. */
    val sizes: MutableList<PaneSize> = Collections.synchronizedList(mutableListOf())

    /** The colors [setColors] was given, in order. */
    val colors: MutableList<PaneColors> = Collections.synchronizedList(mutableListOf())

    /** The seqs [frame] was asked since, in order. */
    val asked: MutableList<ULong> = Collections.synchronizedList(mutableListOf())

    override fun id(): String = paneId

    override fun size(): PaneSize = sizes.lastOrNull() ?: PaneSize(0u, 0u, 0u, 0u)

    override fun resize(size: PaneSize) {
        sizes += size
        onWake?.invoke()
    }

    override fun setColors(colors: PaneColors) {
        this.colors += colors
        onWake?.invoke()
    }

    override fun frame(sinceSeq: ULong): ByteArray {
        asked += sinceSeq
        val frames = frames ?: throw PaneInputException.Closed()
        return frames(sinceSeq)
    }
}
