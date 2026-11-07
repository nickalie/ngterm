package com.nickalie.ngterm.android.ui.machines

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import java.util.UUID

/** A machine on the Machines home: the machine, its link, and its sessions when it has listed them. */
data class MachineItem(
    val machine: StoredMachine,
    val link: Link,
    val sessions: List<RemoteSession>?,
)

/** What the Machines home shows. */
sealed interface MachinesUiState {
    /** The machines are being read. */
    data object Loading : MachinesUiState

    /** The machines could not be read, for [reason]. */
    data class Failed(
        val reason: String,
    ) : MachinesUiState

    /** The machines, and whether the device is [offline]. */
    data class Ready(
        val items: List<MachineItem>,
        val offline: Boolean,
    ) : MachinesUiState
}

/** A machine just removed, which [MachinesViewModel.undoRemove] puts back where it was, with its pin. */
data class Removal(
    val machine: StoredMachine,
    val index: Int,
    val pin: String?,
)

/**
 * The Machines home and its machine form: the list with each machine's link and sessions, adding, editing and removing
 * machines (with undo), and the host-key decisions. It connects every machine once when the list is first read, and
 * each machine it saves.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MachinesViewModel(
    private val graph: AppGraph,
) : ViewModel() {
    private val reads = MutableStateFlow(0)
    private val removal = MutableStateFlow<Removal?>(null)
    private var connectedOnce = false

    /** The home's state. */
    val state: StateFlow<MachinesUiState> =
        reads
            .flatMapLatest {
                combine(
                    graph.machines.machines,
                    graph.connections.links,
                    graph.connections.trees,
                    graph.online,
                ) { machines, links, trees, online ->
                    val items =
                        machines.map { MachineItem(it, links[it.id] ?: Link.Offline, trees[it.id]) }
                    MachinesUiState.Ready(items, offline = !online) as MachinesUiState
                }.catch { emit(MachinesUiState.Failed(it.message ?: it.javaClass.simpleName)) }
            }.stateIn(viewModelScope, SharingStarted.Eagerly, MachinesUiState.Loading)

    /** The app's keys, for the form's key choice; `null` until they have been read. */
    val keys: StateFlow<List<AppKey>?> =
        graph.keys.keys
            .catch { emit(emptyList()) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /** The pinned host keys by machine id, for the form. */
    val pins: StateFlow<Map<String, String>> =
        graph.pins.all
            .catch { emit(emptyMap()) }
            .stateIn(viewModelScope, SharingStarted.Eagerly, emptyMap())

    /** The machine last removed, while its undo is on offer. */
    val removed: StateFlow<Removal?> = removal.asStateFlow()

    init {
        viewModelScope.launch {
            val first = state.first { it !is MachinesUiState.Loading }
            if (first is MachinesUiState.Ready && !connectedOnce) {
                connectedOnce = true
                first.items.filter { it.link == Link.Offline }.forEach { connect(it.machine) }
            }
        }
    }

    /** Reads the machines again after a failure. */
    fun retry() {
        reads.value += 1
    }

    /** Connects (or retries) the machine [id]. */
    fun connect(id: String) {
        viewModelScope.launch { graph.machines.get(id)?.let { connect(it) } }
    }

    private suspend fun connect(machine: StoredMachine) {
        graph.connections.connect(machine.toCore())
    }

    /** Closes the connection of the machine [id]. */
    fun disconnect(id: String) = graph.connections.disconnect(id)

    /** Saves [draft] as a new machine, or as the machine [editing], and connects it. */
    fun save(
        draft: MachineDraft,
        editing: String?,
    ) {
        val machine = draft.toMachine(editing ?: UUID.randomUUID().toString())
        viewModelScope.launch {
            graph.machines.save(machine)
            connect(machine)
        }
    }

    /** Removes the machine [id] and its pin, keeping both for [undoRemove]. */
    fun remove(id: String) {
        viewModelScope.launch {
            val list = graph.machines.machines.first()
            val index = list.indexOfFirst { it.id == id }
            if (index < 0) return@launch
            val pin = graph.pins.get(id)
            graph.connections.disconnect(id)
            graph.pins.remove(id)
            graph.machines.delete(id)
            // offered only now: an undo must not reach the machine store ahead of the delete
            removal.value = Removal(list[index], index, pin)
        }
    }

    /** Puts the machine last removed back, with its pin, and connects it again. */
    fun undoRemove() {
        val undone = removal.value ?: return
        removal.value = null
        viewModelScope.launch {
            undone.pin?.let { graph.pins.set(undone.machine.id, it) }
            // the pin first: the machine is shown, and connected, with it
            graph.machines.restore(undone.machine, undone.index)
            connect(undone.machine)
        }
    }

    /** The removal's undo is no longer on offer. */
    fun forgetRemoval() {
        removal.value = null
    }

    /** The user's answer about the unknown host key [fingerprint] of the machine [id]. */
    fun answerHostKey(
        id: String,
        fingerprint: String,
        trust: Boolean,
    ) = graph.connections.answerHostKey(id, fingerprint, trust)

    /**
     * Removes the pin of the machine [id] (its host key changed and the user knows why) and connects it again, which
     * asks about the key it offers now.
     */
    fun removePin(id: String) {
        viewModelScope.launch {
            graph.pins.remove(id)
            graph.connections.disconnect(id)
            graph.machines.get(id)?.let { connect(it) }
        }
    }
}

/** The machines that wait on the user's answer about a host key, first first. */
fun MachinesUiState.hostKeyQuestions(): List<Pair<StoredMachine, Link.HostKeyUnknown>> =
    (this as? MachinesUiState.Ready)
        ?.items
        ?.mapNotNull { item -> (item.link as? Link.HostKeyUnknown)?.let { item.machine to it } }
        .orEmpty()

/** The machines whose connect waits for the user to sign in, first first. */
fun MachinesUiState.signIns(): List<Pair<StoredMachine, Link.SignIn>> =
    (this as? MachinesUiState.Ready)
        ?.items
        ?.mapNotNull { item -> (item.link as? Link.SignIn)?.let { item.machine to it } }
        .orEmpty()
