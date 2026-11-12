package com.nickalie.ngterm.android.ui.updates

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalUriHandler
import com.nickalie.ngterm.android.update.AppUpdates

/** What the update panel's buttons do. */
data class UpdateActions(
    val onCheck: () -> Unit,
    val onInstall: () -> Unit,
    val onCancel: () -> Unit,
    val onSkip: () -> Unit,
    val onNotes: (url: String) -> Unit,
)

/** [updates]' panel actions, release notes opening in the browser. */
@Composable
fun rememberUpdateActions(updates: AppUpdates): UpdateActions {
    val uris = LocalUriHandler.current
    return remember(updates, uris) {
        UpdateActions(
            onCheck = updates::check,
            onInstall = updates::install,
            onCancel = updates::cancel,
            onSkip = updates::skip,
            onNotes = uris::openUri,
        )
    }
}
