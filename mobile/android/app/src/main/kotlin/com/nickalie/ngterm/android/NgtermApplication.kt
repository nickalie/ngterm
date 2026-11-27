package com.nickalie.ngterm.android

import android.app.Application
import android.os.Build
import android.os.SystemClock
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import com.nickalie.ngterm.android.data.Alerts
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.Asks
import com.nickalie.ngterm.android.data.ClipboardReads
import com.nickalie.ngterm.android.data.ClipboardTrust
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.CrashLogger
import com.nickalie.ngterm.android.data.Diagnostics
import com.nickalie.ngterm.android.data.LiveSessions
import com.nickalie.ngterm.android.data.LogTag
import com.nickalie.ngterm.android.data.MachineStore
import com.nickalie.ngterm.android.data.NativeKeyMaker
import com.nickalie.ngterm.android.data.NotificationPrefs
import com.nickalie.ngterm.android.data.OpenRequests
import com.nickalie.ngterm.android.data.Picks
import com.nickalie.ngterm.android.data.SessionChatViews
import com.nickalie.ngterm.android.data.SettingsStore
import com.nickalie.ngterm.android.platform.AndroidClipboard
import com.nickalie.ngterm.android.platform.AndroidKeystoreWrapper
import com.nickalie.ngterm.android.platform.Foreground
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.platform.NetworkMonitor
import com.nickalie.ngterm.android.platform.Notifications
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.android.service.IdleKeeper
import com.nickalie.ngterm.android.service.LinkKeeper
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.android.ui.chat.PaneChats
import com.nickalie.ngterm.android.ui.session.SessionPanes
import com.nickalie.ngterm.android.update.AppUpdates
import com.nickalie.ngterm.android.update.UpdatesOwner
import com.nickalie.ngterm.android.update.appUpdates
import com.nickalie.ngterm.core.ffi.AppLogInterface
import com.nickalie.ngterm.core.ffi.MobileCore
import com.nickalie.ngterm.core.ffi.appLog
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import java.io.File

/**
 * The app's process: it makes the [AppGraph] on first use (not in `onCreate`, so the JVM tests, which run this class
 * without the native library, never load the core), with what works beside the screens: the network's nudges, the
 * notifications ([Alerts]), the foreground service while sessions are attached ([LinkKeeper]), and the core's
 * connections closed while the app is in the background with nothing attached and no sign-in under way ([IdleKeeper]).
 * Its [updates] (the GitHub build's) stand apart from the graph, so the background check and the installer's answers
 * never start the core. The graph's first step opens the diagnostic log, so the core's own lines reach it from its
 * first connect, and from then on the app's start, its coming to the screen and leaving it, the network and an
 * uncaught exception are logged ([Diagnostics]).
 */
class NgtermApplication :
    Application(),
    GraphOwner,
    UpdatesOwner {
    override val graph: AppGraph by lazy { makeGraph() }

    override val updates: AppUpdates? by lazy { appUpdates(this) }

    // counted from the start, before the graph is made by the first screen
    private lateinit var foreground: Foreground

    override fun onCreate() {
        super.onCreate()
        foreground = Foreground(this)
        CrashLogger.install()
    }

    private fun makeGraph(): AppGraph {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val logs = openLog()
        val vault = KeystoreKeyVault(File(noBackupFilesDir, "keys"), AndroidKeystoreWrapper())
        val pins = PinnedHostKeys(PreferenceDataStoreFactory.create { file("host_keys") })
        val connections =
            Connections(scope, SystemClock::elapsedRealtime) { sink -> MobileCore(sink, vault, pins) }
        val network = NetworkMonitor(this, connections::networkChanged).also { it.start() }
        scope.logStates(network)
        val clipboard = AndroidClipboard(this)
        val live = LiveSessions(scope, connections, clipboard)
        val settings = SettingsStore(PreferenceDataStoreFactory.create { file("settings") })
        connections.followConnectTimeout(settings.settings.map { it.connectTimeout })
        val machines = MachineStore(PreferenceDataStoreFactory.create { file("machines") })
        val asks = Asks(scope, connections)
        val chatViews =
            SessionChatViews(PreferenceDataStoreFactory.create { file("session_chat_views") }, settings.settings)
        val panes = SessionPanes(scope, connections, live, settings = settings.settings)
        val prefs = NotificationPrefs(PreferenceDataStoreFactory.create { file("notifications") })
        Alerts(scope, connections, asks, machines.machines, prefs.enabled, Notifications(this), live.sessions)
        LinkKeeper(scope, connections.attached) { LinkService.start(this) }
        IdleKeeper(
            scope,
            foreground.visible,
            connections.attached,
            connections.signingIn,
            connections::suspend,
            connections::resume,
        )
        return AppGraph(
            machines = machines,
            keys =
                AppKeys(
                    store = PreferenceDataStoreFactory.create { file("keys") },
                    vault = vault,
                    maker = NativeKeyMaker,
                    comment = keyComment(),
                ),
            pins = pins,
            connections = connections,
            clipboardReads =
                ClipboardReads(
                    scope = scope,
                    connections = connections,
                    trust = ClipboardTrust(PreferenceDataStoreFactory.create { file("clipboard") }),
                    clipboard = clipboard,
                ),
            panes = panes,
            chats = PaneChats(scope, connections, live, panes.shown, chatViews),
            asks = asks,
            picks = Picks(scope, connections),
            notificationPrefs = prefs,
            settings = settings,
            opens = OpenRequests(),
            online = network.online,
            logs = logs,
            updates = updates,
        )
    }

    // the process's log, in storage no backup takes, with the line every later one is read against
    private fun openLog(): AppLogInterface {
        val log = appLog(File(noBackupFilesDir, "logs").path)
        Diagnostics.log = log
        val version = runCatching { packageManager.getPackageInfo(packageName, 0).versionName }.getOrNull()
        Diagnostics.info(
            LogTag.APP,
            "started version=${version ?: "unknown"} android=${Build.VERSION.SDK_INT} device=${Build.MODEL}",
        )
        return log
    }

    // the screen's and the network's states in the log, from now on
    private fun CoroutineScope.logStates(network: NetworkMonitor) {
        logChanges(foreground.visible, "in the foreground", "in the background")
        logChanges(network.online, "network online", "network offline")
    }

    // a line for the state as it is now and for each change after
    private fun CoroutineScope.logChanges(
        state: StateFlow<Boolean>,
        on: String,
        off: String,
    ) {
        Diagnostics.info(LogTag.APP, if (state.value) on else off)
        launch { state.drop(1).collect { Diagnostics.info(LogTag.APP, if (it) on else off) } }
    }

    private fun file(name: String) = preferencesDataStoreFile(name)

    // the public key's comment, which shows in a machine's authorized_keys: whose key it is
    private fun keyComment(): String {
        val device =
            Build.MODEL
                .orEmpty()
                .replace(Regex("[^A-Za-z0-9._-]+"), "-")
                .trim('-')
                .ifEmpty { "android" }
        return "ngterm@$device"
    }
}
