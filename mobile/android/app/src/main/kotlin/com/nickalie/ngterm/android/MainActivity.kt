package com.nickalie.ngterm.android

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.platform.openTarget
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.settings.SettingsTheme
import com.nickalie.ngterm.android.ui.settings.isDark
import com.nickalie.ngterm.core.ffi.coreVersion
import kotlinx.coroutines.flow.catch

/**
 * The app's one activity, edge to edge: the app's frame over the mobile core, whose version it loads first, in the
 * theme Settings sets (the system bars' icons follow its light or dark side). A notification's tap reaches it as an
 * intent naming the session and pane to open, which it hands to the frame; an update notification's, to the updates.
 * Each launch lets the updates check.
 */
class MainActivity : ComponentActivity() {
    private val graph: AppGraph get() = (application as GraphOwner).graph

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val version = coreVersion()
        // a recreated activity's intent was handled when it first arrived
        if (savedInstanceState == null) {
            graph.updates?.appStarted()
            opened(intent)
        }
        setContent {
            val flow = remember { graph.settings.settings.catch { emit(AppSettings()) } }
            // nothing is drawn until the settings are read, so a pinned theme never flashes the system's first
            val settings by flow.collectAsStateWithLifecycle(initialValue = null)
            settings?.let { now ->
                val dark = isDark(now)
                DisposableEffect(dark) {
                    val style = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT) { dark }
                    enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
                    onDispose {}
                }
                SettingsTheme(now) {
                    NgtermApp(graph = graph, coreVersion = version)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    private fun opened(intent: Intent?) {
        openTarget(intent)?.let(graph.opens::request)
        graph.updates?.opened(intent)
    }
}
