package com.nickalie.ngterm.android.ui.session

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.ChatViews
import com.nickalie.ngterm.android.data.Create
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.MachinePeer
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.android.data.badge
import com.nickalie.ngterm.android.data.families
import com.nickalie.ngterm.android.data.offersChat
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.android.ui.chat.PaneView
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
 * when it has them, the machine's last listing's otherwise), its machine's [link], its stream's [live] state,
 * whether its panes running an agent show as a chat ([chatView]: its own choice, else Settings'), whether this
 * device is [taking] its lead, its parent's name when it is a companion ([companionOf]), whether it is [nested]
 * under that parent, attached too and listed above it, whether its Duplicate is under way ([duplicating]),
 * whether Duplicate takes a tap ([canDuplicate]: not while any create is under way, from any screen), and the app its
 * machine runs ([peer], when a listing said; `null`: ngterm, or not said).
 */
data class AttachedSession(
    val key: SessionKey,
    val machine: String,
    val name: String,
    val context: String?,
    val status: AgentStatus?,
    val link: Link,
    val live: LiveSession,
    val chatView: Boolean = ChatViews().default,
    val taking: Boolean = false,
    val companionOf: String? = null,
    val nested: Boolean = false,
    val duplicating: Boolean = false,
    val canDuplicate: Boolean = true,
    val peer: MachinePeer? = null,
) {
    /** The name of the app without a chat view its machine runs, or `null` when the machine has one. */
    val noChatOn: String? get() = peer?.takeUnless { it.offersChat }?.badge
}

/** A machine's attached sessions, in its listing's order by family (a parent, then its companions under it). */
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
     * goes to, and how each of its panes running an agent shows ([views], by pane id; a pane not there shows its
     * terminal, with no switch).
     */
    data class Live(
        val session: AttachedSession,
        val focused: String?,
        val views: Map<String, PaneView> = emptyMap(),
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
 * panes, each pane's chat or terminal view and its chat, Take control, Detach, Duplicate session (the copy opens in
 * control) and End session. [notice] carries what failed out of sight (an End, a Duplicate) for a snackbar.
 */
@Suppress("TooManyFunctions") // a function per thing the session view does
class SessionViewModel(
    private val graph: AppGraph,
) : ViewModel() {
    private val openKey = MutableStateFlow<SessionKey?>(null)
    private val focus = MutableStateFlow<Map<SessionKey, String>>(emptyMap())
    private val notices = MutableStateFlow<String?>(null)
    private val connections = graph.connections
    private val creates = graph.creates

    // a Duplicate opens its copy only if the view has not moved since it began (main thread)
    private var moves = 0

    // the last name each session was seen with, for the gone state once the listing has dropped it (main thread)
    private val seen = mutableMapOf<SessionKey, AttachedSession>()

    private val listed =
        combine(
            graph.machines.machines,
            connections.attached,
            connections.trees,
            connections.links,
            connections.peers,
        ) { machines, attached, trees, links, peers ->
            Listed(machines, attached, trees, links, peers)
        }

    /** The destination's state. */
    val state: StateFlow<AttachedUiState> =
        combine(
            listed,
            graph.panes.live.sessions,
            openKey,
            focus,
            combine(graph.chats.views, graph.chats.chatViews, graph.panes.claims.held, creates.underWay, ::Marks),
        ) { listed, live, open, focus, marks ->
            val groups = groups(listed, live, marks, seen)
            AttachedUiState(groups, open?.let { open(it, listed, groups, focus, marks.views) })
        }.catch { emit(AttachedUiState()) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, AttachedUiState())

    /** The shown panes' states, by pane. */
    val panes = graph.panes.shown

    /** Why a pane could not be shown, by pane. */
    val paneFailures = graph.panes.failures

    /** The open chats of the panes shown in chat view, by pane. */
    val chats = graph.chats.chats

    /** What the view should say once in a snackbar, until [noticeShown]. */
    val notice: StateFlow<String?> = notices.asStateFlow()

    // the open session: live while attached (unless its panes say it is gone), gone once its machine's listing has
    // dropped it, and closed when it was let go some other way (a disconnect)
    private fun open(
        key: SessionKey,
        listed: Listed,
        groups: List<AttachedGroup>,
        focus: Map<SessionKey, String>,
        views: Map<PaneRef, PaneView>,
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
                val shown = views.filterKeys { it.machineId == key.machineId && it.sessionId == key.sessionId }
                OpenSession.Live(session, focused, shown.mapKeys { it.key.paneId })
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
    fun open(key: SessionKey) = view(key)

    /** Closes the session view: back to the list. */
    fun close() = view(null)

    /** The Sessions destination is no longer shown: a Duplicate under way leaves its copy in the list. */
    fun left() {
        moves++
    }

    private fun view(key: SessionKey?) {
        if (openKey.value == key) return
        openKey.value = key
        moves++
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

    /** Shows [pane] as [view]: its chat, or its terminal until the pane's next conversation. */
    fun pickView(
        pane: PaneRef,
        view: PaneView,
    ) = graph.chats.pick(pane, view)

    /** Shows [key]'s panes running an agent as a chat ([on]) or as their terminal: the session's own choice. */
    fun setChatView(
        key: SessionKey,
        on: Boolean,
    ) = graph.chats.setChatView(key, on)

    /** Brings [pane]'s chat, if it shows one, to its newest row and the card it waits on (a notification's tap). */
    fun reveal(pane: PaneRef) = graph.chats.reveal(pane)

    /** Take control of [pane]. */
    fun takeLead(pane: PaneRef) = graph.panes.takeLead(pane)

    /** Detaches [key]: it goes on running there, and its view closes. */
    fun detach(key: SessionKey) {
        if (openKey.value == key) view(null)
        connections.detach(key.machineId, key.sessionId)
    }

    /** Ends [key] on its machine, with no undo (the view confirms first); its view closes once it has. */
    fun end(key: SessionKey) {
        viewModelScope.launch {
            try {
                connections.endSession(key.machineId, key.sessionId)
                if (openKey.value == key) view(null)
            } catch (error: MobileException) {
                notices.value = error.reason()
            }
        }
    }

    /**
     * Duplicates [key] on its machine and opens the copy in control (its pane attaching rather than offering Take
     * control), unless the view moved on meanwhile (another session, the list, another destination): the copy then
     * stays attached in the list.
     */
    fun duplicate(key: SessionKey) {
        val create = Create(key.machineId, key.sessionId)
        if (!creates.begin(create)) return
        val at = moves
        viewModelScope.launch {
            try {
                val copy = SessionKey(key.machineId, connections.duplicateSession(key.machineId, key.sessionId).id)
                connections.attach(copy.machineId, copy.sessionId)
                if (moves == at) {
                    graph.panes.claims.claim(copy)
                    view(copy)
                }
            } catch (error: MobileException) {
                notices.value = error.reason()
            } finally {
                creates.end(create)
            }
        }
    }

    /** The [notice] has been shown. */
    fun noticeShown() {
        notices.value = null
    }
}

private data class Marks(
    val views: Map<PaneRef, PaneView>,
    val chatViews: ChatViews,
    val taking: Set<SessionKey>,
    val create: Create?,
)

/** What the lists are made of: the saved machines, what is attached, each machine's listing, link and app. */
private data class Listed(
    val machines: List<StoredMachine>,
    val attached: Map<String, Set<String>>,
    val trees: Map<String, List<RemoteSession>>,
    val links: Map<String, Link>,
    val peers: Map<String, MachinePeer>,
)

// each machine's attached sessions in its listing's order by family (any it no longer lists after), remembering in
// [seen] the name each was last shown with
private fun groups(
    listed: Listed,
    live: Map<SessionKey, LiveSession>,
    marks: Marks,
    seen: MutableMap<SessionKey, AttachedSession>,
): List<AttachedGroup> =
    listed.machines.mapNotNull { machine ->
        val ids = listed.attached[machine.id].orEmpty()
        if (ids.isEmpty()) return@mapNotNull null
        val tree = listed.trees[machine.id].orEmpty()
        val names = tree.associate { it.id to it.name }
        val order =
            families(tree).flatten().filter { it.id in ids }.map { it.id to it.parent } +
                ids.filter { it !in names }.map { it to null }
        val shown = mutableMapOf<String, String>()
        val sessions =
            order.map { (id, parent) ->
                val key = SessionKey(machine.id, id)
                attached(key, machine, listed, live, seen)
                    .copy(
                        chatView = marks.chatViews.on(key),
                        taking = key in marks.taking,
                        duplicating = marks.create == Create(machine.id, id),
                        canDuplicate = marks.create == null,
                        companionOf = parent?.let { shown[it] ?: names[it] ?: it },
                        nested = parent in shown,
                    ).also { shown[id] = it.name }
            }
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
    // the stream's word wins once it has said one, a clear included; the listing's stands in until then
    val status =
        if (stream.statusSaid) {
            stream.status?.let { AgentStatus(it.status, it.color, it.shape) }
        } else {
            listing?.status?.takeIf { it.isNotBlank() }?.let {
                AgentStatus(it, listing.statusColor, listing.statusShape)
            }
        }
    return AttachedSession(
        key = key,
        machine = machine.title,
        name = listing?.name ?: seen[key]?.name ?: key.sessionId,
        context = if (stream.contextSaid) stream.context else listing?.context?.takeIf { it.isNotBlank() },
        status = status,
        link = listed.links[machine.id] ?: Link.Offline,
        live = stream,
        peer = listed.peers[machine.id],
    ).also { seen[key] = it }
}
