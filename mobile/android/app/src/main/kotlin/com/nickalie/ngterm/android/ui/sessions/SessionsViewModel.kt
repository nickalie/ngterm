package com.nickalie.ngterm.android.ui.sessions

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.Link
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
 * New session (named or not; the dialog's state lives here so a rotation keeps a create under way), End session,
 * attaching and detaching. [opened] lists the machine afresh. [notice] carries what a list can't show (an End or an
 * attach that failed) for a snackbar.
 */
class SessionsViewModel(
    private val graph: AppGraph,
    private val machineId: String,
) : ViewModel() {
    private val refreshing = MutableStateFlow(false)
    private val ending = MutableStateFlow<Set<String>>(emptySet())
    private val newSession = MutableStateFlow<NewSessionState?>(null)
    private val notices = MutableStateFlow<String?>(null)

    // which opening of the New session dialog is showing (main thread): a create answers only the dialog it began in
    private var dialog = 0
    private val connections = graph.connections

    private val machine =
        graph.machines.machines.map { list -> list.firstOrNull { it.id == machineId } }

    private val live =
        combine(connections.links, connections.trees, connections.treeFailures, connections.attached) {
            links,
            trees,
            failures,
            attached,
            ->
            Live(links[machineId] ?: Link.Offline, trees[machineId], failures[machineId], attached[machineId].orEmpty())
        }

    private data class Live(
        val link: Link,
        val sessions: List<RemoteSession>?,
        val failure: String?,
        val attached: Set<String>,
    )

    /** The screen's state. */
    val state: StateFlow<SessionsUiState> =
        combine(machine, live, graph.online, refreshing, ending) { machine, live, online, refreshing, ending ->
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
                        ending = ending,
                    ),
                )
            }
        }.catch { emit(SessionsUiState.Gone) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, SessionsUiState.Loading)

    /** The New session dialog, or null while it is closed. */
    val creating: StateFlow<NewSessionState?> = newSession.asStateFlow()

    /** What the screen should say once in a snackbar, until [noticeShown]. */
    val notice: StateFlow<String?> = notices.asStateFlow()

    /**
     * The screen opened: the machine is listed afresh when it is connected. One that connects while the screen is open
     * is listed by its connect.
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

    /** Opens the New session dialog. */
    fun openNewSession() {
        if (newSession.value != null) return
        dialog += 1
        newSession.value = NewSessionState()
    }

    /**
     * Closes the New session dialog; a create under way goes on, its session shows in the list and its failure in the
     * snackbar, and a dialog opened after it is not its.
     */
    fun closeNewSession() {
        dialog += 1
        newSession.value = null
    }

    /** Creates a session named [name] (blank: the machine names it); the dialog closes once it is listed. */
    fun create(name: String) {
        if (newSession.value?.running == true) return
        val mine = dialog
        newSession.value = NewSessionState(running = true)
        viewModelScope.launch {
            try {
                connections.createSession(machineId, name)
                if (dialog == mine) newSession.value = null
            } catch (error: MobileException) {
                // a dialog closed meanwhile has no one to tell: the failure goes to the snackbar
                val reason = error.reason()
                if (dialog == mine) {
                    newSession.update { it?.copy(running = false, failed = reason) }
                } else {
                    notices.value = reason
                }
            }
        }
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

    /** Attaches the session [sessionId]. */
    fun attach(sessionId: String) {
        viewModelScope.launch {
            try {
                connections.attach(machineId, sessionId)
            } catch (error: MobileException) {
                notices.value = error.reason()
            }
        }
    }

    /** Detaches the session [sessionId]; it goes on running there. */
    fun detach(sessionId: String) = connections.detach(machineId, sessionId)

    /** The [notice] has been shown. */
    fun noticeShown() {
        notices.value = null
    }
}
