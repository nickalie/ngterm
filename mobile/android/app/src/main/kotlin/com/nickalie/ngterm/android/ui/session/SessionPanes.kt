package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.LiveSessions
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.android.data.session
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.Pane
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
 * Why a pane could not be shown is in [failures] until the next try, or until its machine's link comes back
 * ([Link.Connected]) after it was lost, when its slot tries again; a show that fails on the lost link after it came
 * back is tried again at once. The core finds a pane through its machine's last listing, so a pane added since (a
 * split) is shown after a fresh listing, and [retry] always lists first. Its frames reach its state through [live],
 * the attached sessions' live state.
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

    // each machine's link as last seen, and how many times it came back: a show begun before the last return is stale
    private var links: Map<String, Link> = emptyMap()
    private val returns = mutableMapOf<String, Int>()

    /** The shown panes' states, by pane. */
    val shown: StateFlow<Map<PaneRef, TerminalState>> = shownState.asStateFlow()

    /** Why a pane could not be shown (the core's words), by pane, until it is tried again. */
    val failures: StateFlow<Map<PaneRef, String>> = failureState.asStateFlow()

    init {
        scope.launch {
            combine(connections.attached, live.sessions) { attached, sessions -> attached to sessions }
                .collect { (attached, sessions) -> prune(attached, sessions) }
        }
        scope.launch { connections.links.collect(::linked) }
    }

    /** Shows [pane] at [size] unless it is shown or on its way; its state lands in [shown]. */
    fun show(
        pane: PaneRef,
        size: PaneSize,
    ) = show(pane, size, listFirst = false)

    /** Tries [pane] again at [size] (a cover's Retry): its machine is listed afresh, then it is shown as [show]. */
    fun retry(
        pane: PaneRef,
        size: PaneSize,
    ) = show(pane, size, listFirst = true)

    private fun show(
        pane: PaneRef,
        size: PaneSize,
        listFirst: Boolean,
    ) {
        if (pane in shownState.value || !showing.add(pane)) return
        failureState.update { it - pane }
        val began = returns[pane.machineId]
        scope.launch {
            var again = false
            try {
                val handle = open(pane, size, listFirst)
                val state = TerminalState(handle, frameContext)
                live.route(pane, state)
                shownState.update { it + (pane to state) }
                // detached while it was on its way: the core opened it for a session it no longer streams, so it is
                // hidden again
                if (!attached(pane.session)) hide(pane)
            } catch (error: MobileException) {
                // a session detached while it was on its way keeps no failure to block the show on re-attach, and a
                // failure from before its machine came back is stale: the pane is tried again on the link that's back
                when {
                    !attached(pane.session) -> Unit
                    returns[pane.machineId] == began -> failureState.update { it + (pane to error.reason()) }
                    else -> again = true
                }
            } finally {
                showing -= pane
            }
            if (again) show(pane, size, listFirst)
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

    // the core finds the pane through the machine's last listing: one it doesn't know yet (a split since) is looked
    // for again in a fresh one
    private suspend fun open(
        pane: PaneRef,
        size: PaneSize,
        listFirst: Boolean,
    ): Pane {
        if (listFirst) connections.listSessions(pane.machineId)
        val core = connections.core
        return try {
            core.showPane(pane.machineId, pane.sessionId, pane.paneId, size)
        } catch (error: MobileException) {
            if (listFirst || !error.unlisted()) throw error
            connections.listSessions(pane.machineId)
            core.showPane(pane.machineId, pane.sessionId, pane.paneId, size)
        }
    }

    // a machine whose link comes back after it was lost lets go of its panes' failures, so their slots try again
    private fun linked(now: Map<String, Link>) {
        val back = now.filter { (machine, link) -> link == Link.Connected && links[machine] != Link.Connected }.keys
        links = now
        for (machine in back) {
            returns[machine] = returns.getOrDefault(machine, 0) + 1
            failureState.update { failures -> failures.filterKeys { it.machineId != machine } }
        }
    }

    private fun attached(key: SessionKey) = key.sessionId in connections.attached.value[key.machineId].orEmpty()

    private fun drop(pane: PaneRef) {
        live.route(pane, null)
        shownState.update { it - pane }
        failureState.update { it - pane }
    }

    // a detached session's panes went with it, and why they couldn't be shown; a pane its layout no longer has is
    // hidden
    private fun prune(
        attached: Map<String, Set<String>>,
        sessions: Map<SessionKey, LiveSession>,
    ) {
        failureState.update { failures -> failures.filterKeys { it.sessionId in attached[it.machineId].orEmpty() } }
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

/** The core's words for a pane its session's listing doesn't have (crates/ngterm-mobile, pane.rs's `UNKNOWN_PANE`). */
internal const val UNKNOWN_PANE = "the session has no such pane"

// a failure a fresh listing may cure: the session or the pane is missing from the machine's last listing
private fun MobileException.unlisted() =
    this is MobileException.NotListed || (this is MobileException.Remote && reason == UNKNOWN_PANE)
