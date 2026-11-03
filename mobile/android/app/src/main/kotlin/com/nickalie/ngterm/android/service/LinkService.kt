package com.nickalie.ngterm.android.service

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.nickalie.ngterm.android.GraphOwner
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.platform.LINK_NOTIFICATION_ID
import com.nickalie.ngterm.android.platform.Notifications
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps the process, and with it the mobile core and its ssh connections, alive while any session is attached: a
 * `specialUse` foreground service (its subtype in the manifest says why), started by [LinkKeeper] as the first
 * session is attached and stopping itself once none is. Its notification counts the attached sessions, names them,
 * and offers Detach all. A process the system ends anyway is not restarted: its connections went with it.
 */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var connections: Connections
    private lateinit var notifications: Notifications
    private var names: List<String> = emptyList()

    // whether it has been in the foreground: a service started for the foreground must get there before it stops
    private var foreground = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val graph = (application as GraphOwner).graph
        connections = graph.connections
        notifications = Notifications(this)
        names = attachedNames(connections.attached.value, connections)
        scope.launch {
            combine(connections.attached, connections.trees) { attached, _ -> attachedNames(attached, connections) }
                .distinctUntilChanged()
                .collect { now ->
                    names = now
                    // the first emission runs inside onCreate, before onStartCommand has put it in the foreground:
                    // stopping then would crash the app, so an empty one waits for onStartCommand to stop it
                    when {
                        now.isNotEmpty() -> show()
                        foreground -> stopSelf()
                    }
                }
        }
    }

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        // a started foreground service must be in the foreground within seconds, whatever it was started for
        show()
        if (intent?.action == ACTION_DETACH_ALL) detachAll()
        if (names.isEmpty()) stopSelf()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun show() {
        val type =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            } else {
                0
            }
        ServiceCompat.startForeground(this, LINK_NOTIFICATION_ID, notifications.link(names), type)
        foreground = true
    }

    private fun detachAll() {
        connections.attached.value.forEach { (machine, sessions) ->
            sessions.forEach { connections.detach(machine, it) }
        }
    }

    companion object {
        /** The intent action of the notification's Detach all. */
        const val ACTION_DETACH_ALL = "com.nickalie.ngterm.android.DETACH_ALL"

        /** Starts the service in the foreground (the app is, as a session is attached from it). */
        fun start(context: Context) {
            try {
                ContextCompat.startForegroundService(context, Intent(context, LinkService::class.java))
            } catch (_: IllegalStateException) {
                // the system refuses a foreground start from the background (Android 12+); it is tried again only when
                // the attached sessions next go from none to some
            }
        }
    }
}

/** The attached sessions' names, by machine and then in each machine's listing order. */
private fun attachedNames(
    attached: Map<String, Set<String>>,
    connections: Connections,
): List<String> =
    attached.flatMap { (machine, ids) ->
        val tree = connections.trees.value[machine].orEmpty()
        tree.filter { it.id in ids }.map { it.name } + ids.filter { id -> tree.none { it.id == id } }
    }

/**
 * Starts the foreground service ([start]) each time the attached sessions go from none to some; the service stops
 * itself when they go back to none.
 */
class LinkKeeper(
    scope: CoroutineScope,
    attached: StateFlow<Map<String, Set<String>>>,
    private val start: () -> Unit,
) {
    init {
        scope.launch {
            attached
                .map { sessions -> sessions.values.any { it.isNotEmpty() } }
                .distinctUntilChanged()
                .collect { any -> if (any) start() }
        }
    }
}

/**
 * Lets the core close its connections while the app has nothing to keep connected: in the background ([visible]
 * false) with no session attached and no machine waiting for the user to sign in ([signingIn]: the user left the app
 * for the browser to do it, and the connect must still be there when they have), [suspend] runs (the core's
 * `suspend`); once any of them changes, [resume] brings the connections back. Nothing runs until the app first goes
 * idle.
 */
class IdleKeeper(
    scope: CoroutineScope,
    visible: StateFlow<Boolean>,
    attached: StateFlow<Map<String, Set<String>>>,
    signingIn: Flow<Boolean>,
    private val suspend: () -> Unit,
    private val resume: () -> Unit,
) {
    init {
        scope.launch {
            var suspended = false
            combine(visible, attached, signingIn) { shown, sessions, waiting ->
                !shown && !waiting && sessions.values.all { it.isEmpty() }
            }.distinctUntilChanged()
                .collect { idle ->
                    if (idle) {
                        suspend()
                        suspended = true
                    } else if (suspended) {
                        resume()
                        suspended = false
                    }
                }
        }
    }
}
