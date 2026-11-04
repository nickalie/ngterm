package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.core.ffi.AgentInputOutcome
import com.nickalie.ngterm.core.ffi.ChatApproval
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatState
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.PaneInterface
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Where a pane's chat stands. */
enum class ChatPhase {
    /** Opened, its backlog on its way. */
    Loading,

    /** Its backlog is read, and new events follow. */
    Live,

    /** Its link went; it resumes on its own from where it stood. */
    Reconnecting,

    /** It ended for good (the agent ended, or the origin offers no chat): the pane shows its terminal. */
    Ended,
}

/** Whether there is more of the conversation before what the chat shows. */
sealed interface OlderPages {
    /** Maybe: scrolling to the top asks for a page. */
    data object More : OlderPages

    data object Loading : OlderPages

    /** The conversation's start is shown. */
    data object AtStart : OlderPages

    /** The last page could not be read, for [reason] (the core's words); scrolling to the top tries again. */
    data class Failed(
        val reason: String,
    ) : OlderPages
}

/**
 * The permission prompt the agent waits on: the chat event's [id], the [approval] its hook reported, and the buttons'
 * [labels] (the dialog's own options, read from the pane's screen, or the agent's set when the screen can't be read;
 * empty until read).
 */
data class ApprovalCard(
    val id: String,
    val approval: ChatApproval,
    val labels: List<String> = emptyList(),
)

/** Why the last chat action did nothing, or not all it meant to; [offersTerminal] when the terminal is the way on. */
sealed interface ChatProblem {
    val offersTerminal: Boolean get() = false

    /** The screen doesn't show what the action answers (answered elsewhere, or unreadable): nothing was sent. */
    data object NotOnScreen : ChatProblem {
        override val offersTerminal: Boolean get() = true
    }

    /** A dialog is open, so a prompt or Stop would answer it: nothing was sent. */
    data object DialogOpen : ChatProblem

    /** Another device kept control of the pane: nothing was sent. */
    data object LeadRefused : ChatProblem

    /** Keys went, but the screen didn't show what they should have done. */
    data object Unconfirmed : ChatProblem {
        override val offersTerminal: Boolean get() = true
    }

    /** The program isn't reading its input: nothing was sent; try again shortly. */
    data object NotReading : ChatProblem

    /** The pane isn't shown any more (hidden, or its link went): nothing was sent. */
    data object Closed : ChatProblem

    /** The core refused the action, for [reason] (its words): nothing was sent. */
    data class Refused(
        val reason: String,
    ) : ChatProblem
}

/**
 * What a pane's chat shows: its rows ([items], from [log]), where it stands ([phase]), the [conversation] its origin
 * named, whether older pages remain ([older]), the row the list keeps in place after a page went in above it
 * ([anchor], the row that was on top before), the cards pinned above the composer ([approval], [question]), whether
 * an action is on its way ([busy]), and why the last one did nothing ([problem], until shown).
 */
data class ChatUiState(
    val log: ChatLog = ChatLog(),
    val phase: ChatPhase = ChatPhase.Loading,
    val conversation: ChatBody.Conversation? = null,
    val older: OlderPages = OlderPages.More,
    val anchor: String? = null,
    val approval: ApprovalCard? = null,
    val question: OpenQuestion? = null,
    val busy: Boolean = false,
    val problem: ChatProblem? = null,
    // the questions whose picker was found answered or gone: their card stays down until the event says so
    internal val dismissed: Set<String> = emptySet(),
    // bumped by each reset, so a page asked for before it is dropped
    internal val epoch: Int = 0,
) {
    val items: List<ChatItem> get() = log.items
}

/**
 * A shown pane's chat, from the moment [PaneChats] opens it until it closes: the events the core streams fold into
 * [state], older pages come in on [loadOlder], and the actions ([send], [stop], [answerApproval], [answerQuestion])
 * go through the pane's handle ([handle]), each claiming the lead first as the core does. Reading never claims it.
 *
 * - An approval card comes up with the `approval` event and goes with its `approval_done`; a question card comes up
 *   with a question still running and goes once it is answered. Either goes too when this device answered it, or
 *   when the core finds its prompt no longer on screen (answered on another device): the card's next check misses.
 * - A `reset` drops what was shown and starts the new conversation under a divider.
 *
 * Events arrive on the core's thread; [state] takes them from any. Its work runs in [scope], which [close] ends.
 */
@Suppress("TooManyFunctions") // the chat's events and each of its actions
class ChatViewModel internal constructor(
    val pane: PaneRef,
    private val core: MobileCoreInterface,
    private val handle: () -> PaneInterface?,
    private val scope: CoroutineScope,
    private val onEnded: () -> Unit,
) {
    private val chat = MutableStateFlow(ChatUiState())

    /** The chat as the screen shows it. */
    val state: StateFlow<ChatUiState> = chat.asStateFlow()

    /** Opens the pane's chat at the core; one that can't be opened ends, so the pane shows its terminal. */
    internal fun open() {
        scope.launch {
            try {
                core.openChat(pane.machineId, pane.sessionId, pane.paneId)
            } catch (_: MobileException) {
                ended()
            }
        }
    }

    /** Closes the chat at the core and stops its work. */
    internal fun close() {
        scope.cancel()
        try {
            core.closeChat(pane.machineId, pane.sessionId, pane.paneId)
        } catch (_: MobileException) {
            // the core has stopped: no chat is open
        }
    }

    /** Takes a batch of the chat's events, in order. */
    internal fun take(events: List<ChatEvent>) {
        val before = chat.value.approval?.id
        chat.update { it.taking(events) }
        val card = chat.value.approval
        if (card != null && card.id != before) readLabels(card.id)
    }

    /** Takes what became of the chat's link. */
    internal fun take(state: ChatState) {
        when (state) {
            ChatState.RECONNECTING -> chat.update { it.copy(phase = ChatPhase.Reconnecting) }
            ChatState.ENDED -> ended()
        }
    }

    private fun ended() {
        chat.update { it.copy(phase = ChatPhase.Ended, busy = false) }
        onEnded()
    }

    /** Asks for the page before the oldest row, once at a time, until the start is shown. */
    fun loadOlder() {
        val asked = chat.value
        if (asked.older == OlderPages.Loading || asked.older == OlderPages.AtStart || asked.log.isEmpty) return
        chat.update { it.copy(older = OlderPages.Loading) }
        scope.launch {
            val page =
                try {
                    core.loadOlder(pane.machineId, pane.sessionId, pane.paneId)
                } catch (_: MobileException.Cancelled) {
                    // the conversation switched meanwhile: its reset already set the pages afresh
                    return@launch
                } catch (error: MobileException) {
                    chat.update {
                        if (it.epoch ==
                            asked.epoch
                        ) {
                            it.copy(older = OlderPages.Failed(error.reason()))
                        } else {
                            it
                        }
                    }
                    return@launch
                }
            chat.update { now -> if (now.epoch == asked.epoch) now.withPage(page) else now }
        }
    }

    /** Sends [text] as a prompt. */
    fun send(text: String) {
        if (text.isNotBlank()) act(null) { it.sendPrompt(text) }
    }

    /** Stop: interrupts the agent's turn. */
    fun stop() = act(null) { it.interrupt() }

    /** Answers the approval card with its button [label]. */
    fun answerApproval(label: String) {
        val card = chat.value.approval ?: return
        act(card.id) { it.answerApproval(label) }
    }

    /** Answers the question card with [answers], one per question. */
    fun answerQuestion(answers: List<QuestionAnswer>) {
        val card = chat.value.question ?: return
        act(card.id) { it.answerQuestion(card.question, answers) }
    }

    /** The [ChatUiState.problem] has been shown. */
    fun problemShown() {
        chat.update { it.copy(problem = null) }
    }

    // one action at a time; [card] is the card it answers, which goes once answered or found gone
    private fun act(
        card: String?,
        call: suspend (PaneInterface) -> AgentInputOutcome,
    ) {
        if (chat.value.busy) return
        val target = handle()
        if (target == null) {
            chat.update { it.copy(problem = ChatProblem.Closed) }
            return
        }
        chat.update { it.copy(busy = true, problem = null) }
        scope.launch {
            val problem =
                try {
                    problem(call(target))
                } catch (error: PaneInputException) {
                    problem(error)
                }
            chat.update { now ->
                val answered = card != null && (problem == null || problem == ChatProblem.NotOnScreen)
                val after = if (answered) now.without(card) else now
                after.copy(busy = false, problem = problem)
            }
        }
    }

    // the dialog's labels as the screen shows them now, for the card still up
    private fun readLabels(id: String) {
        val target = handle() ?: return
        scope.launch {
            val labels =
                try {
                    target.approvalLabels()
                } catch (_: PaneInputException) {
                    return@launch
                }
            chat.update { now ->
                val card = now.approval
                if (card?.id == id) now.copy(approval = card.copy(labels = labels)) else now
            }
        }
    }
}

// a batch of events into the chat's state: the rows a run of them makes go into the log at once
private fun ChatUiState.taking(events: List<ChatEvent>): ChatUiState {
    var state = this
    val rows = mutableListOf<ChatEvent>()
    for (event in events) {
        if (event.body.isRow()) {
            rows += event
            continue
        }
        if (rows.isNotEmpty()) state = state.withLog(state.log.append(rows))
        rows.clear()
        state = state.meta(event)
    }
    return if (rows.isEmpty()) state else state.withLog(state.log.append(rows))
}

private fun ChatBody.isRow(): Boolean =
    when (this) {
        ChatBody.Reset, ChatBody.Live, is ChatBody.Conversation, is ChatBody.Approval, is ChatBody.ApprovalDone -> false
        else -> true
    }

// an event that is no row: the conversation switched or named, the backlog read, a permission prompt up or done
private fun ChatUiState.meta(event: ChatEvent): ChatUiState =
    when (val body = event.body) {
        ChatBody.Reset -> {
            ChatUiState(
                log = log.reset(),
                phase = phase,
                busy = busy,
                problem = problem,
                epoch = epoch + 1,
            )
        }

        ChatBody.Live -> {
            copy(phase = ChatPhase.Live)
        }

        is ChatBody.Conversation -> {
            copy(conversation = body)
        }

        is ChatBody.Approval -> {
            if (approval?.id == event.id) this else copy(approval = ApprovalCard(event.id, body.approval))
        }

        is ChatBody.ApprovalDone -> {
            if (approval?.id == event.id) copy(approval = null) else this
        }

        else -> {
            this
        }
    }

private fun ChatUiState.withLog(log: ChatLog) = copy(log = log, question = log.openQuestion(dismissed))

// an older page in above the rows, the top row kept in place
private fun ChatUiState.withPage(page: List<ChatEvent>): ChatUiState {
    if (page.isEmpty()) return copy(older = OlderPages.AtStart)
    val top = items.firstOrNull { it !is ChatItem.Divider }?.key
    return withLog(log.prepend(page)).copy(older = OlderPages.More, anchor = top)
}

// the card [id] answered, here or elsewhere
private fun ChatUiState.without(id: String): ChatUiState {
    val approval = approval?.takeIf { it.id != id }
    val dismissed = if (question?.id == id) dismissed + id else dismissed
    return copy(approval = approval, dismissed = dismissed, question = log.openQuestion(dismissed))
}

private fun problem(outcome: AgentInputOutcome): ChatProblem? =
    when (outcome) {
        AgentInputOutcome.SENT -> null
        AgentInputOutcome.NOT_ON_SCREEN -> ChatProblem.NotOnScreen
        AgentInputOutcome.DIALOG_OPEN -> ChatProblem.DialogOpen
        AgentInputOutcome.LEAD_REFUSED -> ChatProblem.LeadRefused
        AgentInputOutcome.UNCONFIRMED -> ChatProblem.Unconfirmed
    }

private fun problem(error: PaneInputException): ChatProblem =
    when (error) {
        is PaneInputException.Closed -> ChatProblem.Closed
        is PaneInputException.Backpressure -> ChatProblem.NotReading
        is PaneInputException.Refused -> ChatProblem.Refused(error.reason)
    }
