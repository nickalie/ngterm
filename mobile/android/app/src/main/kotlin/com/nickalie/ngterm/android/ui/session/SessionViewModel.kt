package com.nickalie.ngterm.android.ui.session

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** An agent's status as the origin sets it: its words, its `#rrggbb` color and its shape's wire name. */
data class AgentStatus(
    val text: String,
    val color: String?,
    val shape: String?,
)

/**
 * An attached session as the app shows it: its [machine]'s name, its [name], [context] and agent [status] (the stream's
 * when it has them, the machine's last listing's otherwise), its machine's [link], and its stream's [live] state.
 */
data class AttachedSession(
    val key: SessionKey,
    val machine: String,
    val name: String,
    val context: String?,
    val status: AgentStatus?,
    val link: Link,
    val live: LiveSession,
)

/** A machine's attached sessions, in its listing's order. */
data class AttachedGroup(
    val machineId: String,
    val machine: String,
    val sessions: List<AttachedSession>,
)

/** The session open in the session view. */
sealed interface OpenSession {
    val key: SessionKey

    /**
     * Attached: [session], with the pane selected ([focused]; `null` before its layout arrives), the one every key
     * goes to.
     */
    data class Live(
        val session: AttachedSession,
        val focused: String?,
    ) : OpenSession {
        override val key: SessionKey get() = session.key
    }

    /** Ended on its machine while it was open, last seen as [name] on [machine]. */
    data class Gone(
        override val key: SessionKey,
        val name: String,
        val machine: String,
    ) : OpenSession
}

/** What the Sessions destination shows: the attached sessions by machine, and the one [open], if any. */
data class AttachedUiState(
    val groups: List<AttachedGroup> = emptyList(),
    val open: OpenSession? = null,
) {
    /** Every attached session, in the order the lists show them. */
    val sessions: List<AttachedSession> get() = groups.flatMap { it.sessions }
}

/**
 * The attached sessions across machines and the session view: which session is open, which of its panes has the
 * focus (kept per session, so a trip elsewhere and back, or a new window size, finds the same pane), showing its
 * panes, Take control, Detach and End session. [notice] carries what failed out of sight (an End) for a snackbar.
 */
class SessionViewModel(
    private val graph: AppGraph,
) : ViewModel() {
    private val openKey = MutableStateFlow<SessionKey?>(null)
    private val focus = MutableStateFlow<Map<SessionKey, String>>(emptyMap())
    private val notices = MutableStateFlow<String?>(null)
    private val connections = graph.connections

    // the last name each session was seen with, for the gone state once the listing has dropped it (main thread)
    private val seen = mutableMapOf<SessionKey, AttachedSession>()

    private val listed =
        combine(graph.machines.machines, connections.attached, connections.trees, connections.links) {
            machines,
            attached,
            trees,
            links,
            ->
            Listed(machines, attached, trees, links)
        }

    /** The destination's state. */
    val state: StateFlow<AttachedUiState> =
        combine(listed, graph.panes.live.sessions, openKey, focus) { listed, live, open, focus ->
            val groups = groups(listed, live, seen)
            AttachedUiState(groups, open?.let { open(it, listed, groups, focus) })
        }.catch { emit(AttachedUiState()) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, AttachedUiState())

    /** The shown panes' states, by pane. */
    val panes = graph.panes.shown

    /** Why a pane could not be shown, by pane. */
    val paneFailures = graph.panes.failures

    /** What the view should say once in a snackbar, until [noticeShown]. */
    val notice: StateFlow<String?> = notices.asStateFlow()

    // the open session: live while attached (unless its panes say it is gone), gone once its machine's listing has
    // dropped it, and closed when it was let go some other way (a disconnect)
    private fun open(
        key: SessionKey,
        listed: Listed,
        groups: List<AttachedGroup>,
        focus: Map<SessionKey, String>,
    ): OpenSession? {
        val session = groups.firstOrNull { it.machineId == key.machineId }?.sessions?.firstOrNull { it.key == key }
        val tree = listed.trees[key.machineId]
        val last = seen[key]
        return when {
            session != null &&
                session.live.panes.values
                    .none { it.end == PaneEnd.Gone }
            -> {
                val panes =
                    session.live.layout
                        ?.panes
                        .orEmpty()
                val focused = focus[key]?.takeIf { it in panes } ?: session.live.layout?.primary
                OpenSession.Live(session, focused)
            }

            session != null || (tree != null && tree.none { it.id == key.sessionId }) -> {
                OpenSession.Gone(key, last?.name ?: key.sessionId, last?.machine ?: key.machineId)
            }

            else -> {
                null
            }
        }
    }

    /** Opens [key]'s session view. */
    fun open(key: SessionKey) {
        openKey.value = key
    }

    /** Closes the session view: back to the list. */
    fun close() {
        openKey.value = null
    }

    /** Gives [paneId] of [key] the focus. */
    fun focus(
        key: SessionKey,
        paneId: String,
    ) {
        focus.update { it + (key to paneId) }
    }

    /** Shows [pane] at [size], as its view first has room for it. */
    fun show(
        pane: PaneRef,
        size: PaneSize,
    ) = graph.panes.show(pane, size)

    /** Tries [pane] again at [size], after it could not be shown: its machine is listed afresh first. */
    fun retry(
        pane: PaneRef,
        size: PaneSize,
    ) = graph.panes.retry(pane, size)

    /** Take control of [pane]. */
    fun takeLead(pane: PaneRef) = graph.panes.takeLead(pane)

    /** Detaches [key]: it goes on running there, and its view closes. */
    fun detach(key: SessionKey) {
        if (openKey.value == key) openKey.value = null
        connections.detach(key.machineId, key.sessionId)
    }

    /** Ends [key] on its machine, with no undo (the view confirms first); its view closes once it has. */
    fun end(key: SessionKey) {
        viewModelScope.launch {
            try {
                connections.endSession(key.machineId, key.sessionId)
                if (openKey.value == key) openKey.value = null
            } catch (error: MobileException) {
                notices.value = error.reason()
            }
        }
    }

    /** The [notice] has been shown. */
    fun noticeShown() {
        notices.value = null
    }
}

/** What the lists are made of: the saved machines, what is attached, each machine's listing and link. */
private data class Listed(
    val machines: List<StoredMachine>,
    val attached: Map<String, Set<String>>,
    val trees: Map<String, List<RemoteSession>>,
    val links: Map<String, Link>,
)

// each machine's attached sessions in its listing's order (any it no longer lists after), remembering in [seen] the
// name each was last shown with
private fun groups(
    listed: Listed,
    live: Map<SessionKey, LiveSession>,
    seen: MutableMap<SessionKey, AttachedSession>,
): List<AttachedGroup> =
    listed.machines.mapNotNull { machine ->
        val ids = listed.attached[machine.id].orEmpty()
        if (ids.isEmpty()) return@mapNotNull null
        val tree = listed.trees[machine.id].orEmpty()
        val order = tree.map { it.id }.filter { it in ids } + ids.filter { id -> tree.none { it.id == id } }
        val sessions = order.map { id -> attached(SessionKey(machine.id, id), machine, listed, live, seen) }
        AttachedGroup(machine.id, machine.title, sessions)
    }

private fun attached(
    key: SessionKey,
    machine: StoredMachine,
    listed: Listed,
    live: Map<SessionKey, LiveSession>,
    seen: MutableMap<SessionKey, AttachedSession>,
): AttachedSession {
    val listing = listed.trees[machine.id]?.firstOrNull { it.id == key.sessionId }
    val stream = live[key] ?: LiveSession()
    val status =
        stream.status?.let { AgentStatus(it.status, it.color, it.shape) }
            ?: listing?.status?.takeIf { it.isNotBlank() }?.let {
                AgentStatus(it, listing.statusColor, listing.statusShape)
            }
    return AttachedSession(
        key = key,
        machine = machine.title,
        name = listing?.name ?: seen[key]?.name ?: key.sessionId,
        context = stream.context ?: listing?.context?.takeIf { it.isNotBlank() },
        status = status,
        link = listed.links[machine.id] ?: Link.Offline,
        live = stream,
    ).also { seen[key] = it }
}
