package com.nickalie.ngterm.android

import android.app.Application
import android.os.Build
import android.os.SystemClock
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.ClipboardReads
import com.nickalie.ngterm.android.data.ClipboardTrust
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.LiveSessions
import com.nickalie.ngterm.android.data.MachineStore
import com.nickalie.ngterm.android.data.NativeKeyMaker
import com.nickalie.ngterm.android.platform.AndroidClipboard
import com.nickalie.ngterm.android.platform.AndroidKeystoreWrapper
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.platform.NetworkMonitor
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.android.ui.session.SessionPanes
import com.nickalie.ngterm.core.ffi.MobileCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import java.io.File

/**
 * The app's process: it makes the [AppGraph] on first use (not in `onCreate`, so the JVM tests, which run this class
 * without the native library, never load the core).
 */
class NgtermApplication : Application() {
    val graph: AppGraph by lazy { makeGraph() }

    private fun makeGraph(): AppGraph {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val vault = KeystoreKeyVault(File(noBackupFilesDir, "keys"), AndroidKeystoreWrapper())
        val pins = PinnedHostKeys(PreferenceDataStoreFactory.create { file("host_keys") })
        val connections =
            Connections(scope, SystemClock::elapsedRealtime) { sink -> MobileCore(sink, vault, pins) }
        val network = NetworkMonitor(this, connections::networkChanged).also { it.start() }
        val clipboard = AndroidClipboard(this)
        val live = LiveSessions(scope, connections, clipboard)
        return AppGraph(
            machines = MachineStore(PreferenceDataStoreFactory.create { file("machines") }),
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
            panes = SessionPanes(scope, connections, live),
            online = network.online,
        )
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
