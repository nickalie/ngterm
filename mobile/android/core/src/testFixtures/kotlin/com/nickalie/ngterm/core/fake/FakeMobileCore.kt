package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.NoHandle
import com.nickalie.ngterm.core.ffi.Pane
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.CompletableDeferred

/** One call the app made on a [FakeMobileCore]: the method's name and its arguments, in order. */
data class FakeCall(
    val method: String,
    val args: List<Any?> = emptyList(),
)

/**
 * A [MobileCoreInterface] with no native library behind it, for the app's JVM and UI tests. It records every call in
 * [calls], keeps each machine's sessions in [sessions], hands [sink] the events a test [emit]s, and throws the
 * failure a test queued with [failNext] from that method's next call. A suspending method's next call can be held open
 * with [holds]: it is recorded and takes its queued failure at once, then waits for the gate before going on.
 *
 * It follows the real core's rules (crates/ngterm-mobile, api.rs and actor.rs) where the app sees them:
 * - [connect] reports [MachineState.Connected] as a [MobileEvent.MachineState] when the machine was not connected
 *   (or was connected as another record: an edited machine connects anew, one only renamed keeps its connection);
 *   a connect that fails reports [MachineState.Unreachable] with the failure's reason before it throws
 *   [MobileException.Unreachable];
 *   [disconnect] reports nothing and forgets the machine's listing; [useKey] puts the key in the record the machine
 *   is connected with and keeps the connection, so a connect with that key is no edit;
 * - host keys, when the fake has [hosts] and [hostKeys] names the key a machine's host offers: a pinned key that
 *   matches connects; another pinned key reports [MobileEvent.HostKeyChanged] with both and fails ([HOST_KEY_CHANGED]);
 *   no pin reports [MobileEvent.HostKeyUnknown] and the connect waits for [answerHostKey], which pins the key through
 *   [hosts] and connects when it trusts the key on offer, and fails ([HOST_KEY_REFUSED]) otherwise; a [disconnect]
 *   while it waits cancels it ([MobileException.Cancelled]);
 * - sign-ins, when [signIns] names what a machine's host asks the user to sign in with: after its host key, the
 *   connect reports [MobileEvent.SignInRequested] and waits for [finishSignIn], which connects it once signed in and
 *   fails it ([SIGN_IN_UNFINISHED]) otherwise; a [disconnect] while it waits cancels it ([MobileException.Cancelled]);
 * - [listSessions] keeps what it answers as the machine's listing and also delivers it as a [MobileEvent.Tree];
 *   [createSession] does the same with the listing that has the new session;
 * - [endSession] ends a session of [sessions] whether listed or not ([MobileException.Remote] for one the machine
 *   doesn't have) and delivers a [MobileEvent.Tree] only when a listing is kept;
 * - [attachSession] and [showPane] take only a session of the machine's last listing ([MobileException.NotListed]
 *   otherwise), not of [sessions]: seeding [sessions] without listing them is refused, as the real core refuses a
 *   session it has not listed;
 * - a session given [panes] has those panes: [showPane] takes only one the session had at the machine's last listing
 *   ([MobileException.Remote] with [UNKNOWN_PANE] otherwise), as the real core resolves a pane through the kept
 *   listing, so a pane added since (a split) needs a new listing first. A session not in [panes] takes any pane.
 *
 * - [showPane] hands back the same handle for a pane already shown; [hidePane], [detachSession] (its session's
 *   panes) and [disconnect] (its machine's) hide panes, and [shownPanes] lists those still shown.
 *
 * [showPane] returns what [paneFactory] makes; the default is a `Pane(NoHandle)`, a placeholder whose methods must not
 * be called (they would reach the native library): a test that drives a pane subclasses it.
 */
@Suppress("TooManyFunctions") // the core's whole interface, and the hooks a test drives its waits with
class FakeMobileCore(
    private val sink: EventSink? = null,
    private val hosts: HostKeyStore? = null,
) : MobileCoreInterface {
    private val recorded = mutableListOf<FakeCall>()
    private val records = mutableMapOf<String, Machine>()
    private val questions = mutableMapOf<String, Question>()

    private class Question(
        val offered: String,
        // trusted or not; null when a disconnect overtook the question
        val answer: CompletableDeferred<Boolean?> = CompletableDeferred(),
    )

    /** The host key each host offers (OpenSSH's `SHA256:` fingerprint), by host name; a host not here offers none. */
    val hostKeys: MutableMap<String, String> = mutableMapOf()

    /** The machines whose connect waits for [answerHostKey], by id, with the key on offer. */
    val waitingForHostKey: Map<String, String> get() = questions.mapValues { it.value.offered }

    /** What each host asks the user to sign in with, by host name: its message and the link in it. */
    val signIns: MutableMap<String, FakeSignIn> = mutableMapOf()

    // each machine's connect waiting for the user to sign in: signed in or not; null when a disconnect overtook it
    private val signInWaits = mutableMapOf<String, CompletableDeferred<Boolean?>>()

    /** The machines whose connect waits for [finishSignIn]. */
    val waitingForSignIn: Set<String> get() = signInWaits.keys.toSet()

    /** Ends the wait of [machineId]'s connect for the user to sign in: [signedIn] connects it, else it fails. */
    fun finishSignIn(
        machineId: String,
        signedIn: Boolean = true,
    ) {
        signInWaits.remove(machineId)?.complete(signedIn)
    }

    private val failures = mutableMapOf<String, MobileException>()

    /**
     * Gates that hold the next call of a suspending method (its Kotlin name) open until they complete: the call is
     * recorded and takes the failure [failNext] queued for it before it waits, so a later call goes on unheld.
     */
    val holds: MutableMap<String, CompletableDeferred<Unit>> = mutableMapOf()
    private var created = 0

    /** The calls made so far, oldest first. */
    val calls: List<FakeCall> get() = synchronized(recorded) { recorded.toList() }

    /** The machines connected (and not disconnected since), by id. */
    val connected: MutableSet<String> = mutableSetOf()

    /** Each machine's sessions, by machine id, as [listSessions] answers them. */
    val sessions: MutableMap<String, MutableList<RemoteSession>> = mutableMapOf()

    /** Each machine's last listing, by machine id: what [listSessions] or [createSession] last answered. */
    private val listings = mutableMapOf<String, List<RemoteSession>>()

    /** Each session's panes, by session id, for the sessions a test gives them: see the class's rules. */
    val panes: MutableMap<String, Set<String>> = mutableMapOf()

    // each machine's sessions' panes at its last listing, by machine id then session id
    private val listedPanes = mutableMapOf<String, Map<String, Set<String>>>()

    private val shown = mutableMapOf<PaneRef, Pane>()

    /** The panes shown and not hidden since, as (machine, session, pane). */
    val shownPanes: Set<PaneRef> get() = synchronized(shown) { shown.keys.toSet() }

    /** What [showPane] returns. */
    var paneFactory: (machineId: String, sessionId: String, paneId: String, size: PaneSize) -> Pane =
        { _, _, _, _ -> Pane(NoHandle) }

    /** Makes the next call of [method] (its Kotlin name, `connect` say) throw [error]. */
    fun failNext(
        method: String,
        error: MobileException,
    ) {
        failures[method] = error
    }

    /** Hands [event] to the sink, as the core's actor would. */
    fun emit(event: MobileEvent) {
        sink?.onEvent(event)
    }

    private fun record(
        method: String,
        vararg args: Any?,
    ) {
        // an app test may read the calls while its coroutines make them on other threads
        synchronized(recorded) { recorded += FakeCall(method, args.toList()) }
        failures.remove(method)?.let { throw it }
    }

    // [record] for a suspending method, held open while a test's gate in [holds] is
    private suspend fun recordHeld(
        method: String,
        vararg args: Any?,
    ) {
        synchronized(recorded) { recorded += FakeCall(method, args.toList()) }
        val failure = failures.remove(method)
        holds.remove(method)?.await()
        failure?.let { throw it }
    }

    private fun sessionsOf(machineId: String): MutableList<RemoteSession> {
        if (machineId !in connected) throw MobileException.NotConnected(machineId)
        return sessions.getOrPut(machineId) { mutableListOf() }
    }

    private fun listed(
        machineId: String,
        sessionId: String,
    ): RemoteSession {
        sessionsOf(machineId)
        return listings[machineId]?.firstOrNull { it.id == sessionId } ?: throw MobileException.NotListed(sessionId)
    }

    /** Keeps the machine's sessions as its listing and delivers them, as the real core does with a fresh listing. */
    private fun keepListing(machineId: String): List<RemoteSession> {
        val listing = sessionsOf(machineId).toList()
        listings[machineId] = listing
        listedPanes[machineId] = listing.mapNotNull { session -> panes[session.id]?.let { session.id to it } }.toMap()
        emit(MobileEvent.Tree(machineId, listing))
        return listing
    }

    override suspend fun connect(machine: Machine) {
        try {
            recordHeld("connect", machine)
        } catch (error: MobileException.Unreachable) {
            unreachable(machine.id, error.reason)
        }
        val id = machine.id
        // renamed only: the core keeps the connection and takes the label
        if (id in connected && records[id]?.copy(label = machine.label) == machine) {
            records[id] = machine
            return
        }
        drop(id)
        checkHostKey(machine)
        checkSignIn(machine)
        connected += id
        records[id] = machine
        emit(MobileEvent.MachineState(id, MachineState.Connected))
    }

    private fun unreachable(
        machineId: String,
        reason: String,
    ): Nothing {
        emit(MobileEvent.MachineState(machineId, MachineState.Unreachable(reason)))
        throw MobileException.Unreachable(reason)
    }

    private suspend fun checkHostKey(machine: Machine) {
        val store = hosts ?: return
        val offered = hostKeys[machine.host] ?: return
        val pinned = store.pinned(machine.id)
        when {
            // pinned as offered: the handshake goes on
            pinned == offered -> {}

            pinned != null -> {
                emit(MobileEvent.HostKeyChanged(machine.id, pinned, offered))
                unreachable(machine.id, HOST_KEY_CHANGED)
            }

            else -> {
                val question = Question(offered)
                questions[machine.id] = question
                emit(MobileEvent.HostKeyUnknown(machine.id, offered))
                val trusted =
                    try {
                        question.answer.await()
                    } finally {
                        if (questions[machine.id] === question) questions -= machine.id
                    }
                if (trusted == null) throw MobileException.Cancelled()
                if (!trusted) unreachable(machine.id, HOST_KEY_REFUSED)
                store.pin(machine.id, offered)
            }
        }
    }

    private suspend fun checkSignIn(machine: Machine) {
        val signIn = signIns[machine.host] ?: return
        val wait = CompletableDeferred<Boolean?>()
        signInWaits[machine.id] = wait
        emit(MobileEvent.SignInRequested(machine.id, signIn.message, signIn.url))
        val signedIn =
            try {
                wait.await()
            } finally {
                if (signInWaits[machine.id] === wait) signInWaits -= machine.id
            }
        if (signedIn == null) throw MobileException.Cancelled()
        if (!signedIn) unreachable(machine.id, SIGN_IN_UNFINISHED)
    }

    private fun drop(machineId: String) {
        synchronized(shown) { shown.keys.removeAll { it.machineId == machineId } }
        connected -= machineId
        records -= machineId
        listings -= machineId
        listedPanes -= machineId
    }

    override fun disconnect(machineId: String) {
        record("disconnect", machineId)
        drop(machineId)
        questions.remove(machineId)?.answer?.complete(null)
        signInWaits.remove(machineId)?.complete(null)
    }

    override fun useKey(
        machineId: String,
        keyId: String,
    ) {
        record("useKey", machineId, keyId)
        records[machineId]?.let { records[machineId] = it.copy(keyId = keyId) }
    }

    override suspend fun listSessions(machineId: String): List<RemoteSession> {
        recordHeld("listSessions", machineId)
        return keepListing(machineId)
    }

    override suspend fun createSession(
        machineId: String,
        name: String?,
    ): RemoteSession {
        recordHeld("createSession", machineId, name)
        created += 1
        val session = fakeSession(id = "fake-$created", name = name?.takeIf { it.isNotBlank() } ?: "session $created")
        sessionsOf(machineId) += session
        keepListing(machineId)
        return session
    }

    override suspend fun endSession(
        machineId: String,
        sessionId: String,
    ) {
        recordHeld("endSession", machineId, sessionId)
        val list = sessionsOf(machineId)
        if (!list.removeAll { it.id == sessionId }) throw MobileException.Remote("no session $sessionId")
        val listing = listings[machineId] ?: return
        listings[machineId] = listing.filter { it.id != sessionId }.also { emit(MobileEvent.Tree(machineId, it)) }
    }

    override suspend fun attachSession(
        machineId: String,
        sessionId: String,
    ) {
        recordHeld("attachSession", machineId, sessionId)
        listed(machineId, sessionId)
    }

    override fun detachSession(
        machineId: String,
        sessionId: String,
    ) {
        record("detachSession", machineId, sessionId)
        synchronized(shown) { shown.keys.removeAll { it.machineId == machineId && it.sessionId == sessionId } }
    }

    override suspend fun showPane(
        machineId: String,
        sessionId: String,
        paneId: String,
        size: PaneSize,
    ): Pane {
        recordHeld("showPane", machineId, sessionId, paneId, size)
        listed(machineId, sessionId)
        if (sessionId in panes && paneId !in listedPanes[machineId]?.get(sessionId).orEmpty()) {
            throw MobileException.Remote(UNKNOWN_PANE)
        }
        val ref = PaneRef(machineId, sessionId, paneId)
        synchronized(shown) { shown[ref] }?.let { return it }
        val pane = paneFactory(machineId, sessionId, paneId, size)
        synchronized(shown) { shown[ref] = pane }
        return pane
    }

    override fun hidePane(
        machineId: String,
        sessionId: String,
        paneId: String,
    ) {
        record("hidePane", machineId, sessionId, paneId)
        synchronized(shown) { shown.keys.remove(PaneRef(machineId, sessionId, paneId)) }
    }

    override fun takeLead(
        machineId: String,
        sessionId: String,
        paneId: String,
    ) = record("takeLead", machineId, sessionId, paneId)

    override fun answerAsk(
        machineId: String,
        sessionId: String,
        askId: String,
        reply: AskReply,
    ) = record("answerAsk", machineId, sessionId, askId, reply)

    override fun answerClipboardRead(
        machineId: String,
        sessionId: String,
        paneId: String,
        readId: ULong,
        reply: ClipboardReply,
    ) = record("answerClipboardRead", machineId, sessionId, paneId, readId, reply)

    override fun answerHostKey(
        machineId: String,
        fingerprint: String,
        trust: Boolean,
    ) {
        record("answerHostKey", machineId, fingerprint, trust)
        // an answer about another key than the one on offer refuses: the user never saw that key
        val question = questions.remove(machineId) ?: return
        question.answer.complete(trust && fingerprint == question.offered)
    }

    override fun networkChanged() = record("networkChanged")

    override fun suspend() = record("suspend")

    override fun resume() = record("resume")
}

/** The core's reason for a connect refused over a changed host key (transport/russh.rs's `HOST_KEY_CHANGED`). */
const val HOST_KEY_CHANGED = "the host key changed"

/** The core's reason for a pane its session's listing doesn't have (pane.rs's `UNKNOWN_PANE`). */
const val UNKNOWN_PANE = "the session has no such pane"

/** The core's reason for a connect whose unknown host key was not trusted (`HOST_KEY_REFUSED`). */
const val HOST_KEY_REFUSED = "the host key was not trusted"

/** The core's reason for a connect whose sign-in was not completed in time (`SIGN_IN_UNFINISHED`). */
const val SIGN_IN_UNFINISHED = "the sign-in was not completed in time"

/** What a host asks the user to sign in with: its [message] and the [url] in it. */
data class FakeSignIn(
    val message: String,
    val url: String,
)

/** A sign-in as Tailscale SSH asks for one, for the tests: its banner and link (made up). */
val TailscaleSignIn =
    FakeSignIn(
        message =
            "# Tailscale SSH requires an additional check.\n" +
                "# To authenticate, visit: https://login.tailscale.com/a/l1nk",
        url = "https://login.tailscale.com/a/l1nk",
    )

/** A session as a machine's tree lists it, with what a test doesn't care about left plain. */
fun fakeSession(
    id: String,
    name: String = id,
    parent: String? = null,
    paneCount: UInt = 1u,
): RemoteSession =
    RemoteSession(
        id = id,
        name = name,
        windowName = "main",
        workspaceName = "default",
        context = null,
        cwd = "/home/fake",
        status = null,
        statusColor = null,
        statusShape = null,
        parent = parent,
        paneCount = paneCount,
        splitAxis = null,
    )
