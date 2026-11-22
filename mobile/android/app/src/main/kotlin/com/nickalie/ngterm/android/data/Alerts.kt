package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.RemoteAsk
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/** The most actions a notification shows; an ask with more buttons is answered in the app. */
const val MAX_NOTIFICATION_ACTIONS = 3

/** Where tapping a notification leads: a session (and its pane), or with [sessionId] `null` only the app. */
data class OpenTarget(
    val machineId: String,
    val sessionId: String? = null,
    val paneId: String? = null,
)

/** How an ask is answered from its notification (the ask rule). */
enum class AskDelivery {
    /** A plain button ask: an action per button. */
    Actions,

    /** An input ask: an inline reply per answering button, and the cancel button as an action. */
    Reply,

    /** A choice ask, or one with more buttons than a notification holds: the notification opens the app. */
    OpenApp,
}

/** Why a machine with attached sessions was lost for good. */
sealed interface LostReason {
    /** The core gave up for [reason], its words. */
    data class Unreachable(
        val reason: String,
    ) : LostReason

    /** The machine offered a key other than the pinned one. */
    data object HostKeyChanged : LostReason
}

/**
 * A notification the app posts, on its machine's channel. [tag] names it: a later alert with the same tag replaces
 * it, and cancelling the tag takes it down.
 */
sealed interface Alert {
    val tag: String
    val machineId: String

    /** The machine's name, for its channel. */
    val machine: String

    /** Where its tap leads. */
    val target: OpenTarget

    /** A notification [session]'s agent sent. */
    data class Notice(
        override val tag: String,
        val key: SessionKey,
        override val machine: String,
        val session: String,
        val title: String,
        val body: String,
        val pane: String?,
    ) : Alert {
        override val machineId: String get() = key.machineId
        override val target: OpenTarget get() = OpenTarget(key.machineId, key.sessionId, pane)
    }

    /** [session]'s agent is waiting ([AlertKind.Waiting]) or done ([AlertKind.Done]); [context] says what it is for. */
    data class Status(
        val key: SessionKey,
        override val machine: String,
        val session: String,
        val kind: AlertKind,
        val context: String?,
        val pane: String?,
    ) : Alert {
        override val tag: String get() = statusTag(key)
        override val machineId: String get() = key.machineId
        override val target: OpenTarget get() = OpenTarget(key.machineId, key.sessionId, pane)
    }

    /** A machine with attached sessions was lost for good. */
    data class Lost(
        override val machineId: String,
        override val machine: String,
        val reason: LostReason,
    ) : Alert {
        override val tag: String get() = lostTag(machineId)
        override val target: OpenTarget get() = OpenTarget(machineId)
    }

    /** An open ask of [session], out of sight, answered as [delivery] says. */
    data class Ask(
        val open: OpenAsk,
        override val machine: String,
        val session: String,
        val delivery: AskDelivery,
    ) : Alert {
        override val tag: String get() = askTag(open)
        override val machineId: String get() = open.key.machineId
        override val target: OpenTarget get() = OpenTarget(open.key.machineId, open.key.sessionId, open.ask.pane)
    }
}

/** Where [Alerts] puts its notifications: the platform's, or a test's record. */
interface AlertSink {
    /** Posts [alert], replacing one with its tag. */
    fun post(alert: Alert)

    /** Takes down the notification [tag], if it is up. */
    fun cancel(tag: String)
}

/** How [ask] is answered from a notification. */
fun deliveryOf(ask: RemoteAsk): AskDelivery =
    when {
        ask.choices != null || ask.buttons.isEmpty() || ask.buttons.size > MAX_NOTIFICATION_ACTIONS -> {
            AskDelivery.OpenApp
        }

        ask.input != null -> {
            AskDelivery.Reply
        }

        else -> {
            AskDelivery.Actions
        }
    }

/** The tag of [key]'s agent status notification: one per session, the latest status replacing the one before. */
fun statusTag(key: SessionKey): String = "status:${key.machineId}:${key.sessionId}"

/** The tag of [machineId]'s lost notification. */
fun lostTag(machineId: String): String = "lost:$machineId"

/** The tag of [open]'s notification. */
fun askTag(open: OpenAsk): String = askTag(open.key, open.ask.id)

/** The tag of the notification of [key]'s ask [askId]. */
fun askTag(
    key: SessionKey,
    askId: String,
): String = "ask:${key.machineId}:${key.sessionId}:$askId"

/**
 * What reaches the user as notifications ([sink]), from the core's events and the open [asks]:
 * - a session's agent's notifications ([AlertKind.Notices]), taken down when the session is detached;
 * - an agent status change to waiting (`blocked`) or done (`completed`) while its session isn't on screen, replaced
 *   by the next change and taken down by one to anything else, by the session coming on screen or being detached;
 * - a machine with attached sessions lost for good ([AlertKind.Errors]: unreachable, or its host key changed), taken
 *   down when it connects again or its last attached session goes;
 * - every open ask out of sight ([AlertKind.Questions]; the ask rule: a dialog while its session is on screen, a
 *   notification otherwise), taken down when it is answered, dismissed, or comes on screen.
 *
 * The kinds the user turned off ([enabled]) are not posted, and turning one off takes its notifications down. Turned
 * on again, a kind brings back what is still current: the open asks, each session's last waiting or done status and
 * each machine still lost; an agent's notice is a message of its time and does not come back. Until [enabled] has said
 * which kinds are on, nothing posts on a guess. Names come from [machines] and the machines' listings, and a session's
 * context from its stream ([live]) once it has said one, its listing's until then. Events arrive on the core's thread;
 * [sink] is called from it and from [scope].
 */
@Suppress("LongParameterList") // a parameter per source the alerts read, and the sink they post to
class Alerts(
    scope: CoroutineScope,
    private val connections: Connections,
    private val asks: Asks,
    machines: Flow<List<StoredMachine>>,
    enabled: Flow<Set<AlertKind>>,
    private val sink: AlertSink,
    private val live: StateFlow<Map<SessionKey, LiveSession>>,
) {
    @Volatile private var names: Map<String, String> = emptyMap()

    private val choices = enabled.catch { emit(AlertKind.entries.toSet()) }

    // the core's events and a change of the kinds on hold it, so a post and a kind turned off do not cross
    private val lock = Any()

    // the kinds the notifications up follow; null until choices has said
    @Volatile private var kinds: Set<AlertKind>? = null
    private val notices = AtomicLong()

    // the machines whose host key changed since they were last connected
    private val keyChanged: MutableSet<String> = ConcurrentHashMap.newKeySet()

    // the asks posted, by tag (scope's thread)
    private val posted = mutableSetOf<String>()

    // under lock: the agents' notices up, by session, taken down with its detach since their tap leads to an attached
    // session; each session's current waiting or done status; each machine's current lost alert. The last two are up
    // while their kind is on.
    private val noticeTags = HashMap<SessionKey, MutableSet<String>>()
    private val statuses = HashMap<SessionKey, Alert.Status>()
    private val lostAlerts = HashMap<String, Alert.Lost>()

    // counts the lost alerts recorded, so one recorded and then detached before the conflated attached flow is seen
    // is still dropped
    private val lostRecorded = MutableStateFlow(0L)

    init {
        connections.listen(::onEvent)
        scope.launch {
            machines.catch { emit(emptyList()) }.collect { list ->
                names =
                    list.associate { it.id to it.title }
            }
        }
        scope.launch { choices.collect(::onKinds) }
        scope.launch {
            combine(asks.open, asks.onScreen, choices, ::Triple).collect { (open, shown, on) ->
                syncAsks(open.takeIf { AlertKind.Questions in on }.orEmpty(), shown)
            }
        }
        scope.launch {
            var before = emptySet<SessionKey>()
            connections.attached.collect { attached ->
                val now = attached.flatMap { (machine, ids) -> ids.map { SessionKey(machine, it) } }.toSet()
                (before - now).forEach { gone ->
                    cancelStatus(gone)
                    synchronized(lock) { noticeTags.remove(gone)?.forEach(sink::cancel) }
                }
                before = now
            }
        }
        scope.launch {
            combine(connections.attached, lostRecorded) { _, _ -> }.collect {
                synchronized(lock) {
                    lostAlerts.keys.filter { connections.attached.value[it].isNullOrEmpty() }.forEach(::dropLost)
                }
            }
        }
    }

    // runs [block] under lock with the kinds on; on the core's thread before choices has said, it waits for its first
    // word, outside the lock
    private inline fun withKinds(block: (Set<AlertKind>) -> Unit) {
        val read = kinds ?: runBlocking { choices.first() }
        synchronized(lock) { block(kinds ?: read.also { kinds = it }) }
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.Notify -> {
                notice(
                    SessionKey(event.machineId, event.sessionId),
                    event.notice.title,
                    event.notice.body,
                    event.notice.pane,
                )
            }

            is MobileEvent.SessionStatus -> {
                if (event.changed) {
                    status(
                        SessionKey(event.machineId, event.sessionId),
                        event.status?.status,
                        event.status?.pane,
                    )
                }
            }

            is MobileEvent.MachineState -> {
                when (val state = event.state) {
                    is MachineState.Unreachable -> {
                        // a changed host key is the reason the core gives up: its own alert says more
                        if (event.machineId !in keyChanged) lost(event.machineId, LostReason.Unreachable(state.reason))
                    }

                    MachineState.Connected -> {
                        keyChanged -= event.machineId
                        dropLost(event.machineId)
                    }

                    // it retries on its own: nothing is lost yet
                    is MachineState.Reconnecting -> {}
                }
            }

            is MobileEvent.HostKeyChanged -> {
                keyChanged += event.machineId
                lost(event.machineId, LostReason.HostKeyChanged)
            }

            else -> {}
        }
    }

    private fun notice(
        key: SessionKey,
        title: String,
        body: String,
        pane: String?,
    ) = withKinds { on ->
        if (AlertKind.Notices !in on) return@withKinds
        val session = connections.sessionName(key)
        val tag = "notice:${key.machineId}:${key.sessionId}:${notices.incrementAndGet()}"
        noticeTags.getOrPut(key) { mutableSetOf() } += tag
        sink.post(Alert.Notice(tag, key, machineName(key.machineId), session, title.ifBlank { session }, body, pane))
    }

    private fun status(
        key: SessionKey,
        status: String?,
        pane: String?,
    ) {
        val kind =
            when (status) {
                "blocked" -> AlertKind.Waiting
                "completed" -> AlertKind.Done
                else -> null
            }
        if (kind == null || asks.onScreen.value == key) {
            cancelStatus(key)
            return
        }
        val context = connections.contextOf(key, live.value)
        val alert = Alert.Status(key, machineName(key.machineId), connections.sessionName(key), kind, context, pane)
        withKinds { on ->
            statuses[key] = alert
            if (kind in on) sink.post(alert) else sink.cancel(alert.tag)
        }
    }

    // only a machine the user has sessions attached on: one without is lost quietly, as its own screen shows
    private fun lost(
        machineId: String,
        reason: LostReason,
    ) {
        val alert = Alert.Lost(machineId, machineName(machineId), reason)
        withKinds { on ->
            if (connections.attached.value[machineId].isNullOrEmpty()) return@withKinds
            lostAlerts[machineId] = alert
            lostRecorded.update { it + 1 }
            if (AlertKind.Errors in on) sink.post(alert)
        }
    }

    private fun syncAsks(
        open: List<OpenAsk>,
        shown: SessionKey?,
    ) {
        shown?.let(::cancelStatus)
        val wanted = open.filter { it.key != shown }.associateBy(::askTag)
        (posted - wanted.keys).forEach(sink::cancel)
        posted.retainAll(wanted.keys)
        for ((tag, ask) in wanted) {
            if (!posted.add(tag)) continue
            val session = connections.sessionName(ask.key)
            sink.post(Alert.Ask(ask, machineName(ask.key.machineId), session, deliveryOf(ask.ask)))
        }
    }

    // a kind turned off takes its notifications down, and one turned on brings back what is still current
    private fun onKinds(on: Set<AlertKind>) =
        synchronized(lock) {
            val before = kinds.orEmpty()
            kinds = on
            if (AlertKind.Notices !in on) {
                noticeTags.values.forEach { tags -> tags.forEach(sink::cancel) }
                noticeTags.clear()
            }

            fun turn(
                kind: AlertKind,
                alert: Alert,
            ) {
                when {
                    kind in on && kind !in before -> sink.post(alert)
                    kind !in on && kind in before -> sink.cancel(alert.tag)
                }
            }
            // the names and context of now, as they may have changed while the kind was off
            for (status in statuses.values) {
                val key = status.key
                val now =
                    status.copy(
                        machine = machineName(key.machineId),
                        session = connections.sessionName(key),
                        context = connections.contextOf(key, live.value),
                    )
                turn(status.kind, now)
            }
            lostAlerts.values.forEach { turn(AlertKind.Errors, it.copy(machine = machineName(it.machineId))) }
        }

    // the machine is back, or nothing is attached on it now: its lost alert is no longer current
    private fun dropLost(machineId: String) =
        synchronized(lock) {
            lostAlerts -= machineId
            sink.cancel(lostTag(machineId))
        }

    private fun cancelStatus(key: SessionKey) =
        synchronized(lock) {
            statuses.remove(key)
            sink.cancel(statusTag(key))
        }

    private fun machineName(machineId: String): String = names[machineId] ?: machineId
}

private fun Connections.listing(key: SessionKey) = trees.value[key.machineId]?.firstOrNull { it.id == key.sessionId }

private fun Connections.sessionName(key: SessionKey): String = listing(key)?.name ?: key.sessionId

// a session's context: its stream's once it has said one, its listing's until then
private fun Connections.contextOf(
    key: SessionKey,
    live: Map<SessionKey, LiveSession>,
): String? {
    val stream = live[key]
    return if (stream?.contextSaid == true) stream.context else listing(key)?.context?.takeIf { it.isNotBlank() }
}
