package com.nickalie.ngterm.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.nickalie.ngterm.android.platform.openTarget
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.coreVersion

/**
 * The app's one activity, edge to edge: the app's frame over the mobile core, whose version it loads first. A
 * notification's tap reaches it as an intent naming the session and pane to open, which it hands to the frame.
 */
class MainActivity : ComponentActivity() {
    private val graph: AppGraph get() = (application as GraphOwner).graph

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val version = coreVersion()
        // a recreated activity's intent was handled when it first arrived
        if (savedInstanceState == null) opened(intent)
        setContent {
            NgtermTheme {
                NgtermApp(graph = graph, coreVersion = version)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    private fun opened(intent: Intent?) {
        openTarget(intent)?.let(graph.opens::request)
    }
}
