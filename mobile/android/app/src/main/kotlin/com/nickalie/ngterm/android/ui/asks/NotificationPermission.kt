package com.nickalie.ngterm.android.ui.asks

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.launch

/**
 * Android 13's notification permission, asked once and with its reason, at the moment it starts to matter: the first
 * time a session is attached (so its questions can reach the user in the background). Turn on notifications raises
 * the system's question; Not now, or dismissing the sheet, leaves it for Settings. Either way it isn't asked again.
 */
@Composable
fun NotificationPermissionPrompt(graph: AppGraph) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
    val context = LocalContext.current
    val prefs = graph.notificationPrefs
    // until the store answers, it counts as asked: the sheet never flashes up
    val askedFlow = remember(prefs) { prefs.permissionAsked.catch { emit(true) } }
    val asked by askedFlow.collectAsStateWithLifecycle(initialValue = true)
    val attached by graph.connections.attached.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    // the attached sessions' service went to the foreground before the answer, its notification dropped for want of
    // the permission: started again, it posts it now
    val launcher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            if (allowed &&
                graph.connections.attached.value.values
                    .any { it.isNotEmpty() }
            ) {
                LinkService.start(context)
            }
        }
    val granted =
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
    if (asked || granted || attached.values.all { it.isEmpty() }) return
    val done = { scope.launch { prefs.markPermissionAsked() } }
    NotificationPermissionSheet(
        onAllow = {
            done()
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        },
        onLater = { done() },
    )
}

/** [NotificationPermissionQuestion] in a sheet; dismissing it is [onLater]. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotificationPermissionSheet(
    onAllow: () -> Unit,
    onLater: () -> Unit,
    modifier: Modifier = Modifier,
) {
    ModalBottomSheet(
        onDismissRequest = onLater,
        modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        NotificationPermissionQuestion(onAllow, onLater)
    }
}

/**
 * Why the app wants to notify: what reaches the user in the background (questions, waiting, finished), that a
 * question can be answered from the notification, and the one quiet notification the attached sessions keep; then
 * Turn on notifications (filled) and Not now.
 */
@Composable
fun NotificationPermissionQuestion(
    onAllow: () -> Unit,
    onLater: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .navigationBarsPadding()
                .padding(horizontal = spacing.xl)
                .padding(bottom = spacing.lg),
        verticalArrangement = Arrangement.spacedBy(spacing.lg),
    ) {
        Text(
            text = stringResource(R.string.notifications_ask_title),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = stringResource(R.string.notifications_ask_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = stringResource(R.string.notifications_ask_quiet),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Button(onClick = onAllow, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.notifications_ask_allow))
            }
            TextButton(onClick = onLater, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.notifications_ask_later))
            }
        }
    }
}
