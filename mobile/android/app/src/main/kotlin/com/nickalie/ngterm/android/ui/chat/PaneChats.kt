package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.LiveSessions
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.session
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.theme.AgentState
import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.filterCommands
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.job
import kotlinx.coroutines.launch

/** How a pane running an agent shows: its chat, or its terminal. */
enum class PaneView { Terminal, Chat }

/**
 * Which panes show as a chat, and their chats. A pane whose origin announces an agent (`LiveSession.agents`) shows as
 * a chat while Settings' Agent chat view is on, unless its terminal was picked: by hand ([pick]), or because its chat
 * ended or the origin couldn't open it (a link that is down only defers the open until it is back). A terminal so
 * picked holds until the pane's next conversation (or the agent leaves); picking the chat again lifts it. With the
 * setting off every pane shows its terminal.
 *
 * [views] has every pane with an agent while the setting is on. A pane in chat view that is shown ([shown]) has its
 * chat open in [chats], one [ChatViewModel] each, so the core's chat channels stay as few as the panes on screen; a
 * pane leaving chat view (or the screen) closes its chat, and the pane's own attach stays open behind it all along.
 * Events arrive on the core's thread; the flows take them from any. Used on the main thread. A chat's `/` menu ranks
 * the agent's commands with [filter] (the core's `filter_commands`; the app's tests, which run without the native
 * library, give their own).
 */
class PaneChats(
    private val scope: CoroutineScope,
    private val connections: Connections,
    live: LiveSessions,
    private val shown: StateFlow<Map<PaneRef, TerminalState>>,
    settings: Flow<AppSettings>,
    private val filter: (String, List<AgentCommand>) -> List<AgentCommand> = ::filterCommands,
) {
    // the panes whose terminal was picked, with the conversation it holds for (`null`: the one still to be named)
    private val holds = MutableStateFlow<Map<PaneRef, Hold>>(emptyMap())
    private val on = MutableStateFlow(AppSettings().agentChat)
    private val viewState = MutableStateFlow<Map<PaneRef, PaneView>>(emptyMap())
    private val chatState = MutableStateFlow<Map<PaneRef, ChatViewModel>>(emptyMap())
    private val sessions = live.sessions

    // how many times each machine's link has come up, by id: a chat asked for while it was down opens on the next
    private val connects = MutableStateFlow<Map<String, Long>>(emptyMap())

    /** How each pane with an agent shows, by pane; a pane not here shows its terminal, with no switch. */
    val views: StateFlow<Map<PaneRef, PaneView>> = viewState.asStateFlow()

    /** The open chats, by pane. */
    val chats: StateFlow<Map<PaneRef, ChatViewModel>> = chatState.asStateFlow()

    init {
        connections.listen(::onEvent)
        scope.launch {
            settings
                .map { it.agentChat }
                .distinctUntilChanged()
                .catch { /* unreadable settings: the default stays */ }
                .collect { on.value = it }
        }
        scope.launch { sessions.collect(::keepHolds) }
        scope.launch {
            combine(sessions, holds, on, ::views).collect { viewState.value = it }
        }
        scope.launch {
            combine(viewState, shown) { views, shown -> views to shown.keys }
                .collect { (views, shown) -> sync(views, shown) }
        }
    }

    /** Shows [pane] as [view]: its terminal held until its next conversation, or its chat again. */
    fun pick(
        pane: PaneRef,
        view: PaneView,
    ) {
        when (view) {
            PaneView.Terminal -> hold(pane)
            PaneView.Chat -> holds.update { it - pane }
        }
    }

    /**
     * Brings [pane]'s open chat to its newest row, the card it waits on pinned under it (a notification's tap opened
     * the pane). A pane with no chat open has nothing to move: it shows its terminal, or its chat opens at its end.
     */
    fun reveal(pane: PaneRef) {
        chatState.value[pane]?.showEnd()
    }

    // a pane whose agent left holds nothing: its next agent starts in chat view
    private fun hold(pane: PaneRef) {
        val agent = sessions.value[pane.session]?.agents?.get(pane.paneId) ?: return
        holds.update { it + (pane to Hold(agent.conversation)) }
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.Chat -> {
                chatState.value[event.pane]?.take(event.events)
            }

            is MobileEvent.ChatState -> {
                chatState.value[event.pane]?.take(event.state)
            }

            is MobileEvent.MachineState -> {
                if (event.state == MachineState.Connected) {
                    connects.update { it + (event.machineId to (it[event.machineId] ?: 0L) + 1) }
                }
            }

            else -> {}
        }
    }

    // a hold lasts while the pane's agent stays in the conversation it was made in; one made before the conversation
    // was named takes the first name it gets
    private fun keepHolds(sessions: Map<SessionKey, LiveSession>) {
        holds.update { held ->
            held
                .mapNotNull { (pane, hold) ->
                    val agent = sessions[pane.session]?.agents?.get(pane.paneId) ?: return@mapNotNull null
                    when (hold.conversation) {
                        null -> pane to Hold(agent.conversation)
                        agent.conversation -> pane to hold
                        else -> null
                    }
                }.toMap()
        }
    }

    // the chats open for exactly the panes on screen in chat view
    private fun sync(
        views: Map<PaneRef, PaneView>,
        shown: Set<PaneRef>,
    ) {
        val wanted = views.filter { (pane, view) -> view == PaneView.Chat && pane in shown }.keys
        val open = chatState.value
        for ((pane, chat) in open) if (pane !in wanted) chat.close()
        val made = (wanted - open.keys).associateWith(::chat)
        chatState.value = open.filterKeys { it in wanted } + made
        made.values.forEach(ChatViewModel::open)
    }

    private fun chat(pane: PaneRef): ChatViewModel =
        ChatViewModel(
            pane = pane,
            core = connections.core,
            handle = { shown.value[pane]?.pane },
            connects = connects,
            working = sessions.map { it[pane.session].works(pane.paneId) }.distinctUntilChanged(),
            filter = filter,
            scope = CoroutineScope(scope.coroutineContext + SupervisorJob(scope.coroutineContext.job)),
            // a chat that ended, or never opened, leaves its terminal on screen until the next conversation
            onEnded = { hold(pane) },
        )
}

// whether the session's agent status says the agent in pane [paneId] works on a turn: the status is the session's,
// naming the pane it came from (none: an origin that doesn't say, taken as the pane's)
private fun LiveSession?.works(paneId: String): Boolean {
    val status = this?.status ?: return false
    return AgentState.fromWire(status.status) == AgentState.Active && (status.pane == null || status.pane == paneId)
}

// a pane's terminal, picked for [conversation]
private data class Hold(
    val conversation: String?,
)

// every pane with an agent, in its view, while the setting is on
private fun views(
    sessions: Map<SessionKey, LiveSession>,
    holds: Map<PaneRef, Hold>,
    on: Boolean,
): Map<PaneRef, PaneView> {
    if (!on) return emptyMap()
    return sessions
        .flatMap { (key, session) ->
            session.agents.map { (paneId, agent) ->
                val pane = PaneRef(key.machineId, key.sessionId, paneId)
                val hold = holds[pane]
                val held = hold != null && (hold.conversation == null || hold.conversation == agent.conversation)
                pane to if (held) PaneView.Terminal else PaneView.Chat
            }
        }.toMap()
}
