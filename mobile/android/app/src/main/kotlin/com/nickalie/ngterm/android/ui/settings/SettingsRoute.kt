package com.nickalie.ngterm.android.ui.settings

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.onboarding.ImportKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

private const val HOME = "home"
private const val IMPORT = "import"
private const val KEY = "key:"

/**
 * The Settings destination: Settings itself, and the screens it opens (a key's public half, importing a key),
 * stacked, with back returning to Settings. A key just made or brought opens its public half, which offers to point
 * every machine at it. Whether the system lets the app post notifications is read again each time the app resumes,
 * so coming back from the system's settings shows the change.
 */
@Composable
fun SettingsRoute(
    graph: AppGraph,
    coreVersion: String,
    modifier: Modifier = Modifier,
    model: SettingsViewModel = viewModel { SettingsViewModel(graph) },
) {
    var page by rememberSaveable { mutableStateOf(HOME) }
    val state by model.state.collectAsStateWithLifecycle()
    val work by model.keyWork.collectAsStateWithLifecycle()
    val fresh by model.newKey.collectAsStateWithLifecycle()
    LaunchedEffect(fresh) {
        fresh?.let {
            page = KEY + it
            model.newKeyShown()
        }
    }
    val back = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back)) { page = HOME }
    val shown = page.takeIf { it.startsWith(KEY) }?.removePrefix(KEY)
    val item = shown?.let { id -> (state as? SettingsUiState.Ready)?.keys?.firstOrNull { it.key.id == id } }
    BackHandler(enabled = page == IMPORT || item != null) { page = HOME }
    when {
        page == IMPORT -> {
            ImportKeyScreen(
                busy = work.importing,
                problem = work.importProblem,
                onImport = model::import,
                onUnreadable = model::unreadable,
                onBack = back.onClick,
                modifier = modifier,
            )
        }

        item != null -> {
            PublicKeyScreen(
                key = item.key,
                exit = back,
                modifier = modifier,
                footer = { SwitchMachines(item, onUse = { model.useEverywhere(item.key.id) }) },
            )
        }

        else -> {
            // a key page whose key isn't there: Settings (its progress while the stores are read, why with Retry when
            // they can't be), back on Settings itself once they are read without the key
            if (shown != null) LaunchedEffect(state) { if (state is SettingsUiState.Ready) page = HOME }
            val actions = rememberSettingsActions(model) { page = it }
            SettingsScreen(
                state = state,
                coreVersion = coreVersion,
                notificationsOn = rememberNotificationsAllowed(),
                keyWork = work,
                actions = actions,
                modifier = modifier,
            )
        }
    }
}

/**
 * Under a key's public half: once it is on the machines, point them all at it; or that every machine uses it
 * already. Nothing with no machine.
 */
@Composable
internal fun SwitchMachines(
    item: KeyItem,
    onUse: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    // with no machine there is nothing to switch
    if (item.usedBy.isEmpty() && item.everyMachine) return
    when {
        item.everyMachine -> {
            Text(
                stringResource(R.string.settings_key_everywhere),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = modifier,
            )
        }

        else -> {
            Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
                Text(
                    stringResource(R.string.settings_key_switch_title),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    stringResource(R.string.settings_key_switch_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Button(onClick = onUse, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.settings_key_use))
                }
            }
        }
    }
}

// the controls' actions over [model]; [onPage] opens a key's public half or the import
@Composable
private fun rememberSettingsActions(
    model: SettingsViewModel,
    onPage: (String) -> Unit,
): SettingsActions {
    val context = LocalContext.current
    val page by rememberUpdatedState(onPage)
    return remember(model, context) {
        SettingsActions(
            onChange = model::change,
            onAlert = model::setAlert,
            onRevokeClipboard = model::revokeClipboard,
            onShowKey = { page(KEY + it) },
            onRotate = model::rotate,
            onImport = {
                model.startImport()
                page(IMPORT)
            },
            onTurnOnNotifications = { context.openNotificationSettings() },
            onRetry = model::retry,
        )
    }
}

// whether the system lets the app post notifications, read again on every resume
@Composable
private fun rememberNotificationsAllowed(): Boolean {
    val context = LocalContext.current
    var allowed by remember { mutableStateOf(true) }
    LifecycleResumeEffect(context) {
        allowed = NotificationManagerCompat.from(context).areNotificationsEnabled()
        onPauseOrDispose {}
    }
    return allowed
}

// the system's notification settings for the app, where a permission refused (or Not now) is turned on
private fun Context.openNotificationSettings() {
    val intent =
        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    startActivity(intent)
}
