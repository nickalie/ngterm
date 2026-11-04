package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AgentInputOutcome
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.ChatApproval
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.NoHandle
import com.nickalie.ngterm.core.ffi.Pane
import com.nickalie.ngterm.core.ffi.PaneColors
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneMouse
import com.nickalie.ngterm.core.ffi.PaneSendOutcome
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.PaneWheel
import com.nickalie.ngterm.core.ffi.PaneWheelOutcome
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import kotlinx.coroutines.CompletableDeferred
import java.util.Collections

/** One input call a [FakePane] took, with its arguments. */
sealed interface FakeInput {
    data class Key(
        val key: PaneKey,
    ) : FakeInput

    data class Typed(
        val text: String,
    ) : FakeInput

    data class Mouse(
        val mouse: PaneMouse,
    ) : FakeInput

    data class Wheel(
        val wheel: PaneWheel,
    ) : FakeInput

    data class Paste(
        val text: String,
        val allowUnsafe: Boolean,
    ) : FakeInput

    data class Focus(
        val focused: Boolean,
    ) : FakeInput

    data class Select(
        val start: CellPoint,
        val end: CellPoint,
        val rectangle: Boolean,
    ) : FakeInput

    data class SelectWord(
        val at: CellPoint,
    ) : FakeInput

    data class SelectLine(
        val at: CellPoint,
    ) : FakeInput

    data object SelectAll : FakeInput

    data object ClearSelection : FakeInput
}

/** One chat action a [FakePane] took, with its arguments. */
sealed interface FakeAction {
    data class Prompt(
        val text: String,
    ) : FakeAction

    data object Interrupt : FakeAction

    data class Approval(
        val approval: ChatApproval,
        val label: String,
    ) : FakeAction

    data class Question(
        val question: ChatQuestion,
        val answers: List<QuestionAnswer>,
    ) : FakeAction
}

/**
 * A [Pane] with no native library behind it, for the views' tests: [frame] answers with what [frames] packs (a
 * [RowBufferWriter]'s bytes) for the seq asked, and throws [PaneInputException.Closed] while it is `null`; [resize]
 * and [setColors] are recorded in [sizes] and [colors], and each seq asked in [asked]. As the real terminal wakes
 * (and the core reports `PaneDirty`) after a resize, new colors or a selection change, those call [onWake]. Every
 * method may be called from any thread.
 *
 * Input is recorded in [inputs], in order, and answered as the real terminal would: [mouse] reports only while
 * [tracking] (the program asked for the mouse); [wheel] answers [wheelOutcome]; [paste] refuses text with a line
 * break unless allowed ([PaneSendOutcome.Unsafe], as Ghostty's paste protection does) and writes nothing for empty
 * text; [selectionText] is [selection]. Input to a pane with no [frames] throws [PaneInputException.Closed].
 *
 * The chat's actions ([sendPrompt], [interrupt], [answerApproval], [answerQuestion]) are recorded in [actions] and
 * answer [outcome], or throw [actionError] when set; [approvalLabels] answers [labels]. A pane with no [frames] throws
 * [PaneInputException.Closed] for them too, as a hidden pane does.
 */
@Suppress("TooManyFunctions") // the real handle's methods, each faked
open class FakePane(
    private val paneId: String = "pane",
    @Volatile var frames: ((since: ULong) -> ByteArray)? = null,
) : Pane(NoHandle) {
    /** Runs after [resize], [setColors] and the selection calls: a test points it at the view's `dirty`. */
    @Volatile var onWake: (() -> Unit)? = null

    /** Whether the program takes pointer events. */
    @Volatile var tracking: Boolean = false

    /** Where a wheel turn goes. */
    @Volatile var wheelOutcome: PaneWheelOutcome = PaneWheelOutcome.SCROLLED

    /** The selection's text as a copy takes it. */
    @Volatile var selection: String? = null

    /** The grids [resize] was given, in order. */
    val sizes: MutableList<PaneSize> = Collections.synchronizedList(mutableListOf())

    /** The colors [setColors] was given, in order. */
    val colors: MutableList<PaneColors> = Collections.synchronizedList(mutableListOf())

    /** The seqs [frame] was asked since, in order. */
    val asked: MutableList<ULong> = Collections.synchronizedList(mutableListOf())

    /** The input taken, in order. */
    val inputs: MutableList<FakeInput> = Collections.synchronizedList(mutableListOf())

    /** What a chat action answers. */
    @Volatile var outcome: AgentInputOutcome = AgentInputOutcome.SENT

    /** What a chat action throws instead, when set (a refusal, say). */
    @Volatile var actionError: PaneInputException? = null

    /** What [approvalLabels] answers. */
    @Volatile var labels: List<String> = emptyList()

    /** A gate the next chat action waits at once taken, until the test completes it (an action on its way). */
    @Volatile var gate: CompletableDeferred<Unit>? = null

    /** The chat actions taken, in order. */
    val actions: MutableList<FakeAction> = Collections.synchronizedList(mutableListOf())

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

    private fun take(input: FakeInput) {
        if (frames == null) throw PaneInputException.Closed()
        inputs += input
    }

    private fun selected(input: FakeInput) {
        take(input)
        onWake?.invoke()
    }

    override fun key(key: PaneKey): Boolean {
        take(FakeInput.Key(key))
        return true
    }

    override fun typeText(text: String): Boolean {
        take(FakeInput.Typed(text))
        return text.isNotEmpty()
    }

    override fun mouse(mouse: PaneMouse): Boolean {
        take(FakeInput.Mouse(mouse))
        return tracking
    }

    override fun mouseTracking(): Boolean = tracking

    override fun wheel(wheel: PaneWheel): PaneWheelOutcome {
        take(FakeInput.Wheel(wheel))
        return wheelOutcome
    }

    override fun paste(
        text: String,
        allowUnsafe: Boolean,
    ): PaneSendOutcome {
        take(FakeInput.Paste(text, allowUnsafe))
        return when {
            text.isEmpty() -> PaneSendOutcome.EMPTY
            !allowUnsafe && ('\n' in text || '\r' in text) -> PaneSendOutcome.UNSAFE
            else -> PaneSendOutcome.WRITTEN
        }
    }

    override fun focus(focused: Boolean): Boolean {
        take(FakeInput.Focus(focused))
        return false
    }

    override fun select(
        start: CellPoint,
        end: CellPoint,
        rectangle: Boolean,
    ) = selected(FakeInput.Select(start, end, rectangle))

    override fun selectWord(at: CellPoint): Boolean {
        selected(FakeInput.SelectWord(at))
        return true
    }

    override fun selectLine(at: CellPoint): Boolean {
        selected(FakeInput.SelectLine(at))
        return true
    }

    override fun selectAll(): Boolean {
        selected(FakeInput.SelectAll)
        return true
    }

    override fun clearSelection() = selected(FakeInput.ClearSelection)

    override fun selectionText(): String? = selection

    private suspend fun act(action: FakeAction): AgentInputOutcome {
        if (frames == null) throw PaneInputException.Closed()
        actionError?.let { throw it }
        actions += action
        gate?.also { gate = null }?.await()
        return outcome
    }

    override suspend fun sendPrompt(text: String): AgentInputOutcome = act(FakeAction.Prompt(text))

    override suspend fun interrupt(): AgentInputOutcome = act(FakeAction.Interrupt)

    override suspend fun answerApproval(
        approval: ChatApproval,
        label: String,
    ): AgentInputOutcome = act(FakeAction.Approval(approval, label))

    override suspend fun answerQuestion(
        question: ChatQuestion,
        answers: List<QuestionAnswer>,
    ): AgentInputOutcome = act(FakeAction.Question(question, answers))

    override suspend fun approvalLabels(): List<String> {
        if (frames == null) throw PaneInputException.Closed()
        return labels
    }
}
