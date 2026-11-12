package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList

/** Where a machine's connection stands, as the app shows it. A machine with none is [Offline]. */
sealed interface Link {
    /** Not connected and not trying: never connected, disconnected, or a host key the user declined. */
    data object Offline : Link

    /** A connect the app asked for is on its way. */
    data object Connecting : Link

    data object Connected : Link

    /** Lost after being connected; the core tries again at [retryAt] (the [Connections.clock]'s milliseconds). */
    data class Reconnecting(
        val retryAt: Long,
    ) : Link

    /** Down until the user tries again, for [reason] (the core's words). */
    data class Unreachable(
        val reason: String,
    ) : Link

    /** The machine has no pinned key and offered [fingerprint]: its connect waits for the user's answer. */
    data class HostKeyUnknown(
        val fingerprint: String,
    ) : Link

    /**
     * The machine asks the user to sign in at [url] (from its [message], as Tailscale SSH does for a `check`) before it
     * lets the connect in: the connect waits for that. The machine's next state ends it, or a disconnect.
     * [reconnecting] when the wait is the core's reconnect of a lost connection, not a connect the app asked for: the
     * machine is listed afresh once it is back, as after any reconnect.
     */
    data class SignIn(
        val message: String,
        val url: String,
        val reconnecting: Boolean = false,
    ) : Link

    /** The machine offered [offered], not the [pinned] key: the connection stopped until the pin is removed. */
    data class HostKeyChanged(
        val pinned: String,
        val offered: String,
    ) : Link
}

/**
 * The app's side of the mobile core: it makes the core with itself as the [EventSink] ([coreFactory]), turns the
 * core's events and its connect answers into each machine's [Link] in [links], keeps each machine's last listing in
 * [trees] (and why the last one failed in [treeFailures]; a connect and a reconnect each list anew), and the sessions
 * this app has attached in [attached]. An attached session's listing carries the name its stream last said, so a
 * rename on its machine shows everywhere at once, though the machine is listed only now and then.
 * Events arrive on the core's thread; the state flows take them from any thread. Calls are made from the main thread
 * and the suspending ones run in [scope] or the caller's, except [detach], which a listing event also makes on the
 * core's thread (it only posts to the core, so it must never block on it).
 */
@Suppress("TooManyFunctions") // the app's one wrapper of the core: a function per core call and per event it keeps
class Connections(
    private val scope: CoroutineScope,
    val clock: () -> Long,
    coreFactory: (EventSink) -> MobileCoreInterface,
) : EventSink {
    private val linkState = MutableStateFlow<Map<String, Link>>(emptyMap())
    private val treeState = MutableStateFlow<Map<String, List<RemoteSession>>>(emptyMap())
    private val treeFailureState = MutableStateFlow<Map<String, String>>(emptyMap())
    private val attachedState = MutableStateFlow<Map<String, Set<String>>>(emptyMap())

    // the record each machine was last connected with (main thread): a connect with another replaces the connection
    private val records = mutableMapOf<String, Machine>()

    // each machine's listing generation, bumped by every listing that lands and every connection that goes: a listing
    // failure counts only while no newer listing or connection has overtaken the call that failed. The lock keeps the
    // bump and the failure's check-and-set from interleaving across the core's thread and the callers'.
    private val listingLock = Any()
    private val generations = mutableMapOf<String, Long>()

    // the name each attached session's stream last said, by session: it outranks its listing's while it stays attached
    private val streamNames = ConcurrentHashMap<SessionKey, String>()

    // each machine's last listing as the core gave it (under listingLock): [trees] is it with the stream names over it,
    // so a session that stops being attached shows its listing's name again
    private val listings = mutableMapOf<String, List<RemoteSession>>()

    // the machines whose unknown host key the user declined: the failure their connect reports is no failure to them
    private val declined = MutableStateFlow<Set<String>>(emptySet())

    // the machines whose connect was cancelled before it listed them: the connection the core makes for them later
    // (a resume's) is listed when it reports Connected. Whoever takes a machine's marker out lists it, so a Connected
    // that lands before the cancellation is handled is listed once, by the handler or by the event
    private val unlisted: MutableSet<String> = ConcurrentHashMap.newKeySet()

    /** Each machine's link, by id; a machine missing from it is [Link.Offline]. */
    val links: StateFlow<Map<String, Link>> = linkState.asStateFlow()

    /** Whether any machine's connect waits for the user to sign in ([Link.SignIn]). */
    val signingIn: Flow<Boolean> =
        linkState
            .map { links ->
                links.values.any { it is Link.SignIn }
            }.distinctUntilChanged()

    /** Each connected machine's sessions as it last listed them, by id. */
    val trees: StateFlow<Map<String, List<RemoteSession>>> = treeState.asStateFlow()

    /** Why a machine's last listing failed (the core's words), by id, until a listing lands. */
    val treeFailures: StateFlow<Map<String, String>> = treeFailureState.asStateFlow()

    /** The sessions this app has attached, by machine id: each machine's session ids. */
    val attached: StateFlow<Map<String, Set<String>>> = attachedState.asStateFlow()

    // the app's other parts that take the core's events (clipboard reads, panes), called on the core's thread
    private val listeners = CopyOnWriteArrayList<(MobileEvent) -> Unit>()

    /** The core, for what this class doesn't wrap. */
    val core: MobileCoreInterface = coreFactory(this)

    override fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.MachineState -> machineState(event.machineId, event.state)
            is MobileEvent.HostKeyUnknown -> set(event.machineId, Link.HostKeyUnknown(event.fingerprint))
            is MobileEvent.HostKeyChanged -> set(event.machineId, Link.HostKeyChanged(event.pinned, event.offered))
            is MobileEvent.SignInRequested -> signInRequested(event.machineId, event.message, event.url)
            is MobileEvent.Tree -> listed(event.machineId, event.sessions)
            is MobileEvent.SessionName -> named(event.machineId, event.sessionId, event.name)
            else -> Unit
        }
        listeners.forEach { it(event) }
    }

    /**
     * Hands [listener] every event the core reports from now on, after this class has taken it, on the core's
     * thread: it must not block.
     */
    fun listen(listener: (MobileEvent) -> Unit) {
        listeners += listener
    }

    // a listing replaces the last one and its failure; an attached session it no longer has has ended there, so its
    // stream is let go
    private fun listed(
        machineId: String,
        sessions: List<RemoteSession>,
    ) {
        synchronized(listingLock) {
            bump(machineId)
            listings[machineId] = sessions
            treeState.update { it + (machineId to withStreamNames(machineId, sessions)) }
            treeFailureState.update { it - machineId }
        }
        val ids = sessions.mapTo(mutableSetOf()) { it.id }
        val gone = attachedState.value[machineId].orEmpty() - ids
        gone.forEach { detach(machineId, it) }
    }

    // a stream's word on its session's name (a blank one says nothing); it reaches the listing once the session is
    // attached, as one may land before its attach is recorded
    private fun named(
        machineId: String,
        sessionId: String,
        name: String,
    ) {
        if (name.isBlank()) return
        streamNames[SessionKey(machineId, sessionId)] = name
        if (sessionId in attachedState.value[machineId].orEmpty()) renamed(machineId)
    }

    // the machine's last listing, its attached sessions under the names their streams said
    private fun renamed(machineId: String) =
        synchronized(listingLock) {
            val listing = listings[machineId] ?: return@synchronized
            treeState.update { it + (machineId to withStreamNames(machineId, listing)) }
        }

    private fun withStreamNames(
        machineId: String,
        sessions: List<RemoteSession>,
    ): List<RemoteSession> {
        val attached = attachedState.value[machineId].orEmpty()
        return sessions.map { session ->
            val said = streamNames[SessionKey(machineId, session.id)]
            if (said != null && session.id in attached && said != session.name) session.copy(name = said) else session
        }
    }

    // callers hold listingLock
    private fun bump(machineId: String) {
        generations[machineId] = generations.getOrDefault(machineId, 0L) + 1
    }

    private fun machineState(
        machineId: String,
        state: MachineState,
    ) {
        when (state) {
            MachineState.Connected -> reconnected(machineId)
            is MachineState.Reconnecting -> set(machineId, Link.Reconnecting(clock() + state.nextInMs.toLong()))
            is MachineState.Unreachable -> failed(machineId, state.reason)
        }
    }

    // a reconnect that has to sign in again (a Tailscale check that ran out while the connection was down) stays a
    // reconnect, so its Connected lists the machine afresh
    private fun signInRequested(
        machineId: String,
        message: String,
        url: String,
    ) = linkState.update { links ->
        val before = links[machineId]
        val reconnecting = before is Link.Reconnecting || (before is Link.SignIn && before.reconnecting)
        links + (machineId to Link.SignIn(message, url, reconnecting))
    }

    // the core restores a lost connection's streams but lists nothing: a machine back from reconnecting is listed
    // afresh here, as a listing that failed or was dropped while it was down would otherwise never be redone
    private fun reconnected(machineId: String) {
        val before = linkState.getAndUpdate { it + (machineId to Link.Connected) }[machineId]
        val cancelled = unlisted.remove(machineId)
        val wasReconnecting = before is Link.Reconnecting || (before is Link.SignIn && before.reconnecting)
        if (wasReconnecting || cancelled) scope.launch { listSessions(machineId) }
    }

    // a failure doesn't hide a changed host key (the core reports one with both), and a declined key is no failure
    private fun failed(
        machineId: String,
        reason: String,
    ) {
        val wasDeclined = machineId in declined.value
        linkState.update { links ->
            when {
                links[machineId] is Link.HostKeyChanged -> links
                wasDeclined -> links - machineId
                else -> links + (machineId to Link.Unreachable(reason))
            }
        }
    }

    private fun set(
        machineId: String,
        link: Link,
    ) = linkState.update { it + (machineId to link) }

    /**
     * Connects [machine] (or retries it at once), then lists its sessions for [trees]. A machine already connected as
     * it is stays connected; one connected as it was before an edit connects anew, and lets go of its attached sessions
     * (the core ends their streams with the old connection). A machine waiting for the user to sign in keeps waiting
     * (the core joins its connect), unless it was edited: that connect is overtaken, and the new one asks again.
     */
    fun connect(machine: Machine) {
        val id = machine.id
        val before = records.put(id, machine)
        val edited = before != null && !before.reachedAs(machine)
        if (edited) replaced(id)
        declined.update { it - id }
        linkState.update { links ->
            when (links[id]) {
                Link.Connected, is Link.HostKeyUnknown -> links
                is Link.SignIn -> if (edited) links + (id to Link.Connecting) else links
                else -> links + (id to Link.Connecting)
            }
        }
        scope.launch {
            try {
                core.connect(machine)
                // connected already: the core reports no state for a machine that didn't change
                linkState.update { links ->
                    if (links[id] == Link.Connecting) links + (id to Link.Connected) else links
                }
            } catch (_: MobileException.Cancelled) {
                // overtaken by a disconnect or another connect, which reports its own state; or set aside by a
                // suspend, whose resume connects it with no caller: that connection is listed when it comes
                unlisted.add(id)
                // a resumed connection may have reported Connected before the marker went in: list it here, unless
                // that event took the marker first and lists it itself
                if (linkState.value[id] != Link.Connected || !unlisted.remove(id)) return@launch
            } catch (error: MobileException) {
                if (linkState.value[id] is Link.Connecting) failed(id, error.reason())
                return@launch
            }
            listSessions(id)
        }
    }

    // the machine's connection is replaced by one with another record: its attached sessions' streams went with the old
    // one, and a listing call on it no longer counts
    private fun replaced(machineId: String) {
        synchronized(listingLock) { bump(machineId) }
        attachedState.value[machineId].orEmpty().forEach { detach(machineId, it) }
    }

    /**
     * Lists [machineId]'s sessions again: the listing arrives in [trees], or why it failed in [treeFailures]. A listing
     * overtaken by a disconnect, or by a newer listing or connection, fails nothing.
     */
    suspend fun listSessions(machineId: String) {
        val generation = synchronized(listingLock) { generations.getOrDefault(machineId, 0L) }
        try {
            core.listSessions(machineId)
        } catch (_: MobileException.Cancelled) {
            // a later call reports its own outcome
        } catch (_: MobileException.Stopped) {
            linkState.update { emptyMap() }
        } catch (error: MobileException) {
            synchronized(listingLock) {
                val current = generations.getOrDefault(machineId, 0L) == generation
                if (current && linkState.value[machineId] == Link.Connected) {
                    treeFailureState.update { it + (machineId to error.reason()) }
                }
            }
        }
    }

    /**
     * Creates a session on [machineId], named [name] (blank for the machine's own name), and returns it once it is
     * listed; the listing that has it arrives in [trees].
     *
     * @throws MobileException with the core's words when it could not.
     */
    suspend fun createSession(
        machineId: String,
        name: String,
    ): RemoteSession = core.createSession(machineId, name.trim().ifEmpty { null })

    /**
     * Duplicates [sessionId] on [machineId] as desktop's Duplicate does, and returns the copy once it is listed; the
     * listing that has it arrives in [trees].
     *
     * @throws MobileException with the core's words when it could not.
     */
    suspend fun duplicateSession(
        machineId: String,
        sessionId: String,
    ): RemoteSession = core.duplicateSession(machineId, sessionId)

    /**
     * Ends [sessionId] on [machineId], with no undo, and lets go of it here when it was attached.
     *
     * @throws MobileException with the core's words when it could not.
     */
    suspend fun endSession(
        machineId: String,
        sessionId: String,
    ) {
        core.endSession(machineId, sessionId)
        if (sessionId in attachedState.value[machineId].orEmpty()) detach(machineId, sessionId)
    }

    /**
     * Attaches [sessionId] of [machineId], a session of its last listing; attaching an attached one changes nothing.
     *
     * @throws MobileException with the core's words when it could not.
     */
    suspend fun attach(
        machineId: String,
        sessionId: String,
    ) {
        // a name an earlier stream of the session said is not this one's; attaching an attached one keeps its stream
        if (sessionId !in attachedState.value[machineId].orEmpty()) streamNames.remove(SessionKey(machineId, sessionId))
        core.attachSession(machineId, sessionId)
        attachedState.update { it + (machineId to (it[machineId].orEmpty() + sessionId)) }
        renamed(machineId)
    }

    /** Detaches [sessionId] of [machineId]: it goes on running there. */
    fun detach(
        machineId: String,
        sessionId: String,
    ) {
        attachedState.update { attached ->
            val left = attached[machineId].orEmpty() - sessionId
            if (left.isEmpty()) attached - machineId else attached + (machineId to left)
        }
        streamNames.remove(SessionKey(machineId, sessionId))
        renamed(machineId)
        guarded { core.detachSession(machineId, sessionId) }
    }

    /** The user's answer about the unknown host key [fingerprint] of [machineId]. */
    fun answerHostKey(
        machineId: String,
        fingerprint: String,
        trust: Boolean,
    ) {
        if (trust) {
            set(machineId, Link.Connecting)
        } else {
            declined.update { it + machineId }
            linkState.update { it - machineId }
        }
        guarded { core.answerHostKey(machineId, fingerprint, trust) }
    }

    /**
     * Closes [machineId]'s connection, if any, and forgets its listing and its attached sessions (the core ends their
     * streams).
     */
    fun disconnect(machineId: String) {
        guarded { core.disconnect(machineId) }
        records -= machineId
        linkState.update { it - machineId }
        synchronized(listingLock) {
            bump(machineId)
            listings -= machineId
            treeState.update { it - machineId }
            treeFailureState.update { it - machineId }
        }
        attachedState.update { it - machineId }
        streamNames.keys.removeIf { it.machineId == machineId }
    }

    /**
     * [machineId] signs in with the key [keyId] from its next connection attempt, a reconnect included; its connection
     * and attached sessions stay.
     */
    fun useKey(
        machineId: String,
        keyId: String,
    ) {
        records[machineId]?.let { records[machineId] = it.copy(keyId = keyId) }
        guarded { core.useKey(machineId, keyId) }
    }

    /** The network came back or changed: a machine waiting to reconnect tries at once. */
    fun networkChanged() = guarded { core.networkChanged() }

    /**
     * The app has nothing to keep connected (in the background, nothing attached): the core closes every connection
     * quietly until [resume], which connects again the machines that were.
     */
    fun suspend() = guarded { core.suspend() }

    /** Brings back what [suspend] closed. */
    fun resume() = guarded { core.resume() }

    // a core that has stopped answers nothing more; there is nothing left to tell it
    private inline fun guarded(call: () -> Unit) {
        try {
            call()
        } catch (_: MobileException.Stopped) {
            linkState.update { emptyMap() }
        }
    }
}

/** The words for a failed core call, as the core gives them. */
fun MobileException.reason(): String =
    when (this) {
        is MobileException.Unreachable -> reason
        is MobileException.Remote -> reason
        else -> message ?: toString()
    }
