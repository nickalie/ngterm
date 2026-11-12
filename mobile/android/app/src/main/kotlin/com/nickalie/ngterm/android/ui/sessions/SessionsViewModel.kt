package com.nickalie.ngterm.android.ui.sessions

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.ChatViews
import com.nickalie.ngterm.android.data.Create
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** What a machine's sessions screen knows: the machine, its link, its sessions and what is under way on them. */
data class MachineSessions(
    val machine: StoredMachine,
    val link: Link,
    // the last listing, or null while there is none
    val sessions: List<RemoteSession>?,
    // why the last listing failed, until one lands
    val failure: String?,
    val attached: Set<String>,
    val offline: Boolean,
    // a pull to refresh is under way
    val refreshing: Boolean = false,
    // the sessions whose End is under way
    val ending: Set<String> = emptySet(),
    // the session whose Duplicate is under way
    val duplicating: String? = null,
    // a create is under way, from any screen: neither New session nor Duplicate takes another tap
    val creating: Boolean = false,
    // whether each session's panes running an agent show as a chat
    val chatViews: ChatViews = ChatViews(),
)

/** What a machine's sessions screen shows. */
sealed interface SessionsUiState {
    /** The machine is being read. */
    data object Loading : SessionsUiState

    /** The machine is no longer saved (removed while its screen was open). */
    data object Gone : SessionsUiState

    data class Ready(
        val sessions: MachineSessions,
    ) : SessionsUiState
}

/** The New session dialog while it is open: whether the create is [running], and why the last one [failed]. */
data class NewSessionState(
    val running: Boolean = false,
    val failed: String? = null,
)

/**
 * A machine's sessions: its listing with each session's status and whether this app has it attached, refreshing it,
 * New session (named or not; the dialog's state lives here so a rotation keeps a create under way), Duplicate session
 * (landing as New session does), End session,
 * attaching and detaching. [opened] lists the machine afresh. [notice] carries what a list can't show (a create, an
 * End or an attach that failed) for a snackbar.
 */
@Suppress("TooManyFunctions") // a function per thing the screen does
class SessionsViewModel(
    private val graph: AppGraph,
    private val machineId: String,
) : ViewModel() {
    private val refreshing = MutableStateFlow(false)
    private val ending = MutableStateFlow<Set<String>>(emptySet())
    private val newSession = MutableStateFlow<NewSessionState?>(null)
    private val lands = MutableStateFlow<SessionKey?>(null)
    private val notices = MutableStateFlow<String?>(null)

    // which opening of the New session dialog is showing (main thread): a create answers only the dialog it began in
    private var dialog = 0
    private val connections = graph.connections

    private val machine =
        graph.machines.machines.map { list -> list.firstOrNull { it.id == machineId } }

    private val live =
        combine(
            connections.links,
            connections.trees,
            connections.treeFailures,
            connections.attached,
            graph.chats.chatViews,
        ) { links, trees, failures, attached, chatViews ->
            Live(
                links[machineId] ?: Link.Offline,
                trees[machineId],
                failures[machineId],
                attached[machineId].orEmpty(),
                chatViews,
            )
        }

    private data class Live(
        val link: Link,
        val sessions: List<RemoteSession>?,
        val failure: String?,
        val attached: Set<String>,
        val chatViews: ChatViews,
    )

    private val creates = graph.creates

    private val underWay = combine(ending, creates.underWay, ::UnderWay)

    private data class UnderWay(
        val ending: Set<String>,
        val create: Create?,
    )

    /** The screen's state. */
    val state: StateFlow<SessionsUiState> =
        combine(machine, live, graph.online, refreshing, underWay) { machine, live, online, refreshing, underWay ->
            if (machine == null) {
                SessionsUiState.Gone
            } else {
                SessionsUiState.Ready(
                    MachineSessions(
                        machine = machine,
                        link = live.link,
                        sessions = live.sessions,
                        failure = live.failure,
                        attached = live.attached,
                        offline = !online,
                        refreshing = refreshing,
                        ending = underWay.ending,
                        duplicating = underWay.create?.takeIf { it.machineId == machineId }?.source,
                        creating = underWay.create != null,
                        chatViews = live.chatViews,
                    ),
                )
            }
        }.catch { emit(SessionsUiState.Gone) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, SessionsUiState.Loading)

    /** The New session dialog, or null while it is closed. */
    val creating: StateFlow<NewSessionState?> = newSession.asStateFlow()

    val busy: StateFlow<Boolean> =
        creates.underWay
            .map { it == Create(machineId) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, false)

    val landing: StateFlow<SessionKey?> = lands.asStateFlow()

    /** What the screen should say once in a snackbar, until [noticeShown]. */
    val notice: StateFlow<String?> = notices.asStateFlow()

    /**
     * The screen opened: the machine is listed afresh when it is connected. One that connects or reconnects while the
     * screen is open is listed by [Connections][com.nickalie.ngterm.android.data.Connections] then.
     */
    fun opened() {
        viewModelScope.launch {
            val first = state.first { it !is SessionsUiState.Loading }
            if ((first as? SessionsUiState.Ready)?.sessions?.link == Link.Connected) connections.listSessions(machineId)
        }
    }

    /** Lists the sessions again (pull to refresh, or Retry after a failed listing). */
    fun refresh() {
        if (refreshing.value) return
        refreshing.value = true
        viewModelScope.launch {
            try {
                connections.listSessions(machineId)
            } finally {
                refreshing.value = false
            }
        }
    }

    /** Connects the machine, or retries it now. */
    fun connect() {
        viewModelScope.launch { graph.machines.get(machineId)?.let { connections.connect(it.toCore()) } }
    }

    fun newSession() {
        val create = Create(machineId)
        if (!creates.begin(create)) return
        viewModelScope.launch {
            try {
                land(connections.createSession(machineId, "").id)
            } catch (error: MobileException) {
                notices.value = error.reason()
            } finally {
                creates.end(create)
            }
        }
    }

    fun openNamedSession() {
        if (newSession.value != null) return
        dialog += 1
        newSession.value = NewSessionState()
    }

    /** A create under way goes on and still lands; its failure goes to the snackbar, not to a dialog opened later. */
    fun closeNewSession() {
        dialog += 1
        newSession.value = null
    }

    fun create(name: String) {
        val create = Create(machineId)
        if (newSession.value?.running == true || !creates.begin(create)) return
        val mine = dialog
        newSession.value = NewSessionState(running = true)
        viewModelScope.launch {
            try {
                val id =
                    try {
                        connections.createSession(machineId, name).id
                    } catch (error: MobileException) {
                        // a dialog closed meanwhile has no one to tell: the failure goes to the snackbar
                        val reason = error.reason()
                        if (dialog == mine) {
                            newSession.update { it?.copy(running = false, failed = reason) }
                        } else {
                            notices.value = reason
                        }
                        return@launch
                    }
                if (dialog == mine) newSession.value = null
                try {
                    land(id)
                } catch (error: MobileException) {
                    notices.value = error.reason()
                }
            } finally {
                creates.end(create)
            }
        }
    }

    /** Duplicates [sessionId] there and opens the copy in control, as New session does with the one it creates. */
    fun duplicate(sessionId: String) {
        val create = Create(machineId, sessionId)
        if (!creates.begin(create)) return
        viewModelScope.launch {
            try {
                land(connections.duplicateSession(machineId, sessionId).id)
            } catch (error: MobileException) {
                notices.value = error.reason()
            } finally {
                creates.end(create)
            }
        }
    }

    private suspend fun land(sessionId: String) {
        connections.attach(machineId, sessionId)
        lands.value = SessionKey(machineId, sessionId)
    }

    // claimed only as it opens: a page left meanwhile never opens it, and must not take its lead later
    fun landed() {
        val key = lands.value ?: return
        graph.panes.claims.claim(key)
        lands.value = null
    }

    /** Ends the session [sessionId] there, with no undo (the screen confirms first). */
    fun end(sessionId: String) {
        if (sessionId in ending.value) return
        ending.update { it + sessionId }
        viewModelScope.launch {
            try {
                connections.endSession(machineId, sessionId)
            } catch (error: MobileException) {
                notices.value = error.reason()
            } finally {
                ending.update { it - sessionId }
            }
        }
    }

    /** Attaches the session [sessionId], then runs [onAttached]. */
    fun attach(
        sessionId: String,
        onAttached: () -> Unit = {},
    ) {
        viewModelScope.launch {
            try {
                connections.attach(machineId, sessionId)
                onAttached()
            } catch (error: MobileException) {
                notices.value = error.reason()
            }
        }
    }

    /** Detaches the session [sessionId]; it goes on running there. */
    fun detach(sessionId: String) = connections.detach(machineId, sessionId)

    /** Shows [sessionId]'s panes running an agent as a chat ([on]) or as their terminal: the session's own choice. */
    fun setChatView(
        sessionId: String,
        on: Boolean,
    ) = graph.chats.setChatView(SessionKey(machineId, sessionId), on)

    /** The [notice] has been shown. */
    fun noticeShown() {
        notices.value = null
    }
}
