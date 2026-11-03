package com.nickalie.ngterm.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.nickalie.ngterm.android.ui.CoreStatus
import com.nickalie.ngterm.android.ui.NgtermTheme
import com.nickalie.ngterm.core.ffi.coreVersion

/** The app's one activity: for now it shows that the mobile core's native library loaded, by its version. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val version = coreVersion()
        setContent {
            NgtermTheme {
                CoreStatus(coreVersion = version)
            }
        }
    }
}
