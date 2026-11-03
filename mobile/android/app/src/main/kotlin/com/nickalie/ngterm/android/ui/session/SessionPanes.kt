package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.LiveSessions
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.android.data.session
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlin.coroutines.CoroutineContext

/**
 * The panes this app shows, each with its view's [TerminalState] in [shown]: a pane is shown the first time its view
 * has a size ([show]) and stays shown, keeping its terminal, its lead and its text size, while the user is elsewhere
 * in the app, until its session is detached (the core hides it then) or the session's layout drops it ([hide]s it).
 * Why a pane could not be shown is in [failures] until the next try. Its frames reach its state through [live], the
 * attached sessions' live state.
 * Used on the main thread; [frameContext] is where the states read their frames.
 */
class SessionPanes(
    private val scope: CoroutineScope,
    private val connections: Connections,
    val live: LiveSessions,
    private val frameContext: CoroutineContext = Dispatchers.Default,
) {
    private val shownState = MutableStateFlow<Map<PaneRef, TerminalState>>(emptyMap())
    private val failureState = MutableStateFlow<Map<PaneRef, String>>(emptyMap())
    private val showing = mutableSetOf<PaneRef>()

    /** The shown panes' states, by pane. */
    val shown: StateFlow<Map<PaneRef, TerminalState>> = shownState.asStateFlow()

    /** Why a pane could not be shown (the core's words), by pane, until it is tried again. */
    val failures: StateFlow<Map<PaneRef, String>> = failureState.asStateFlow()

    init {
        scope.launch {
            combine(connections.attached, live.sessions) { attached, sessions -> attached to sessions }
                .collect { (attached, sessions) -> prune(attached, sessions) }
        }
    }

    /** Shows [pane] at [size] unless it is shown or on its way; its state lands in [shown]. */
    fun show(
        pane: PaneRef,
        size: PaneSize,
    ) {
        if (pane in shownState.value || !showing.add(pane)) return
        failureState.update { it - pane }
        scope.launch {
            try {
                val handle = connections.core.showPane(pane.machineId, pane.sessionId, pane.paneId, size)
                val state = TerminalState(handle, frameContext)
                live.route(pane, state)
                shownState.update { it + (pane to state) }
                // detached while it was on its way: the core hid it with the session
                if (!attached(pane.session)) drop(pane)
            } catch (error: MobileException) {
                failureState.update { it + (pane to error.reason()) }
            } finally {
                showing -= pane
            }
        }
    }

    /** Hides [pane]: its channel closes, and its state goes. */
    fun hide(pane: PaneRef) {
        try {
            connections.core.hidePane(pane.machineId, pane.sessionId, pane.paneId)
        } catch (_: MobileException) {
            // the core has stopped: nothing is shown any more
        }
        drop(pane)
    }

    /** Take control of [pane]: it attaches again claiming the lead. */
    fun takeLead(pane: PaneRef) {
        try {
            connections.core.takeLead(pane.machineId, pane.sessionId, pane.paneId)
        } catch (_: MobileException) {
            // the core has stopped: there is nothing to lead
        }
    }

    private fun attached(key: SessionKey) = key.sessionId in connections.attached.value[key.machineId].orEmpty()

    private fun drop(pane: PaneRef) {
        live.route(pane, null)
        shownState.update { it - pane }
        failureState.update { it - pane }
    }

    // a detached session's panes went with it; a pane its layout no longer has is hidden
    private fun prune(
        attached: Map<String, Set<String>>,
        sessions: Map<SessionKey, LiveSession>,
    ) {
        for (pane in shownState.value.keys) {
            val key = pane.session
            val layout = sessions[key]?.layout
            when {
                key.sessionId !in attached[key.machineId].orEmpty() -> drop(pane)
                layout != null && pane.paneId !in layout.panes -> hide(pane)
            }
        }
    }
}
