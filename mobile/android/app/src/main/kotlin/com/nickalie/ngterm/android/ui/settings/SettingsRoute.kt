package com.nickalie.ngterm.android.ui.settings

import android.content.Context
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import com.nickalie.ngterm.android.platform.AndroidKeyExports
import com.nickalie.ngterm.android.platform.DeviceAuth
import com.nickalie.ngterm.android.platform.rememberDeviceAuth
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.logs.LogsRoute
import com.nickalie.ngterm.android.ui.onboarding.ImportKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.updates.rememberUpdateActions
import com.nickalie.ngterm.android.update.AppUpdates
import kotlinx.coroutines.flow.catch

private const val HOME = "home"
private const val IMPORT = "import"
private const val LOGS = "logs"
private const val KEY = "key:"

/**
 * The Settings destination: Settings itself, and the screens it opens (a key's public half, importing a key,
 * exporting one once [auth] has confirmed the device's owner, the diagnostic log), stacked, with back returning to
 * Settings. A key just made or brought opens its public half, which offers to point every machine at it. Whether the
 * system lets the app post notifications is read again each time the app resumes, so coming back from the system's
 * settings shows the change.
 */
@Composable
fun SettingsRoute(
    graph: AppGraph,
    coreVersion: String,
    modifier: Modifier = Modifier,
    model: SettingsViewModel = viewModel { SettingsViewModel(graph) },
    auth: DeviceAuth = rememberDeviceAuth(),
    exports: ExportKeyViewModel = rememberExportKeyViewModel(graph),
) {
    var page by rememberSaveable { mutableStateOf(HOME) }
    val state by model.state.collectAsStateWithLifecycle()
    val work by model.keyWork.collectAsStateWithLifecycle()
    val export by exports.work.collectAsStateWithLifecycle()
    val fresh by model.newKey.collectAsStateWithLifecycle()
    OpenFreshKey(fresh) {
        page = KEY + it
        model.newKeyShown()
    }
    val back = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back)) { page = HOME }
    val shown = page.takeIf { it.startsWith(KEY) }?.removePrefix(KEY)
    val item = state.keyItem(shown)
    val exported = state.keyItem(export.keyId)
    PageEffects(page, keyShown = item != null, exporting = exported != null) {
        exports.close()
        page = HOME
    }
    when {
        exported != null -> {
            EndUnlockOnLeave(exports::close)
            ExportKeyPage(exported.key, export, exports.rule, exports.handlers(), modifier)
        }

        page == LOGS -> {
            LogsRoute(log = graph.logs, exit = back, modifier = modifier)
        }

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
            val actions = rememberSettingsActions(model, rememberExportStart(auth, exports::unlock)) { page = it }
            SettingsScreen(
                state = state,
                coreVersion = coreVersion,
                notificationsOn = rememberNotificationsAllowed(),
                keyWork = work,
                actions = actions,
                modifier = modifier,
                updates = graph.updates?.let { rememberUpdatesUi(it) },
            )
        }
    }
}

// a key just made or brought opens its public half
@Composable
private fun OpenFreshKey(
    fresh: String?,
    onOpen: (String) -> Unit,
) {
    val open by rememberUpdatedState(onOpen)
    LaunchedEffect(fresh) { fresh?.let(open) }
}

// back from a page Settings opened ([onBack]) returns to Settings; the page shown is logged here as it composes,
// before the page itself composes (the Logs page then reads its own line): when a tap sets it, when Settings opens
// or comes back after a rotation or a restore, and for the export's page
@Composable
private fun PageEffects(
    page: String,
    keyShown: Boolean,
    exporting: Boolean,
    onBack: () -> Unit,
) {
    BackHandler(enabled = page == IMPORT || page == LOGS || keyShown || exporting, onBack = onBack)
    // the page that shows, by the conditions that select it in [SettingsRoute]: a key page whose key is not there
    // (yet) shows Settings
    val shown =
        when {
            exporting -> "export"
            page == LOGS || page == IMPORT -> page
            keyShown -> KEY.removeSuffix(":")
            else -> HOME
        }
    // only here, in the composition: an effect runs after the page reads the log, and an effect that a newer page
    // replaced before it ran writes a stale page; a second writer (where the page is set) logs a page that the export
    // hides, which this one then logs over
    rememberSettingsPageLog().note(shown)
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

// however the export's page is left, by the navigation bar's other destinations too, the unlock ends; a rotation
// keeps it, and so does the system's file screen, which only stops the activity
@Composable
private fun EndUnlockOnLeave(onLeave: () -> Unit) {
    val activity = LocalActivity.current
    val leave by rememberUpdatedState(onLeave)
    DisposableEffect(Unit) {
        onDispose { if (activity?.isChangingConfigurations != true) leave() }
    }
}

// the export's model over the app's keys, the system's documents and its clipboard
@Composable
private fun rememberExportKeyViewModel(graph: AppGraph): ExportKeyViewModel {
    val context = LocalContext.current.applicationContext
    return viewModel { ExportKeyViewModel(graph.keys, AndroidKeyExports(context)) }
}

// the controls' actions over [model]; [onPage] opens a key's public half or the import, [onExport] a key's export
@Composable
private fun rememberSettingsActions(
    model: SettingsViewModel,
    onExport: (String) -> Unit,
    onPage: (String) -> Unit,
): SettingsActions {
    val context = LocalContext.current
    val page by rememberUpdatedState(onPage)
    val export by rememberUpdatedState(onExport)
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
            onExport = { export(it) },
            onShowLogs = { page(LOGS) },
        )
    }
}

// the updates' controls over [updates]: until its store answers, both switches show as on, their default
@Composable
private fun rememberUpdatesUi(updates: AppUpdates): UpdatesUi {
    val state by updates.state.collectAsStateWithLifecycle()
    val automaticFlow = remember(updates) { updates.automatic.catch { emit(true) } }
    val automatic by automaticFlow.collectAsStateWithLifecycle(initialValue = true)
    val notifyFlow = remember(updates) { updates.notify.catch { emit(true) } }
    val notify by notifyFlow.collectAsStateWithLifecycle(initialValue = true)
    val actions = rememberUpdateActions(updates)
    return UpdatesUi(updates.installed, state, automatic, actions, updates::setAutomatic, notify, updates::setNotify)
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
