package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.ConnectTimeoutRange
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.CopyButton
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.onboarding.Problem
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.android.ui.updates.UpdatePanel

/** A notification kind's switch: its words. */
private class AlertRow(
    val kind: AlertKind,
    val label: Int,
    val note: Int,
)

private val AlertRows =
    listOf(
        AlertRow(AlertKind.Questions, R.string.settings_alert_questions, R.string.settings_alert_questions_note),
        AlertRow(AlertKind.Waiting, R.string.settings_alert_waiting, R.string.settings_alert_waiting_note),
        AlertRow(AlertKind.Done, R.string.settings_alert_done, R.string.settings_alert_done_note),
        AlertRow(AlertKind.Notices, R.string.settings_alert_notices, R.string.settings_alert_notices_note),
        AlertRow(AlertKind.Errors, R.string.settings_alert_errors, R.string.settings_alert_errors_note),
    )

/**
 * The notification kinds' switches ([on] are on), in the GitHub build ([updates]) the update notification's switch
 * too, the note that the attached sessions keep one quiet notification whatever they say, and while the system lets
 * the app post none ([allowed] false) a banner whose Turn on opens the system's settings for the app.
 */
@Composable
internal fun NotificationsSection(
    on: Set<AlertKind>,
    allowed: Boolean,
    actions: SettingsActions,
    updates: UpdatesUi? = null,
) {
    SettingsSection(stringResource(R.string.settings_notifications)) {
        if (!allowed) {
            Banner(
                tone = StatusTone.Attention,
                message = stringResource(R.string.settings_notifications_off),
                action =
                    SectionAction(
                        stringResource(R.string.settings_notifications_turn_on),
                        actions.onTurnOnNotifications,
                    ),
                modifier = Modifier.padding(vertical = NgtermTheme.spacing.sm),
            )
        }
        for (row in AlertRows) {
            SwitchRow(
                label = stringResource(row.label),
                note = stringResource(row.note),
                checked = row.kind in on,
                onCheckedChange = { actions.onAlert(row.kind, it) },
            )
        }
        updates?.let {
            SwitchRow(
                label = stringResource(R.string.settings_alert_updates),
                note = stringResource(R.string.settings_alert_updates_note),
                checked = it.notify,
                onCheckedChange = it.onNotify,
            )
        }
        SettingsNote(stringResource(R.string.settings_alert_attached))
    }
}

/** How long a connect may take, for each machine that sets no time of its own in its editor. */
@Composable
internal fun ConnectionsSection(
    timeout: Int,
    onTimeout: (Int) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_connections)) {
        SliderRow(
            label = stringResource(R.string.settings_connect_timeout),
            value = timeout,
            range = ConnectTimeoutRange,
            valueText = R.string.settings_connect_timeout_value,
            note = stringResource(R.string.settings_connect_timeout_note),
            onValue = onTimeout,
        )
    }
}

/**
 * How a pane running Claude Code or Codex shows: on ([chatOn]), as a chat with its terminal one switch away in the
 * session's bar; off, always as its terminal, with no switch.
 */
@Composable
internal fun AgentsSection(
    chatOn: Boolean,
    onChatOn: (Boolean) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_agents)) {
        SwitchRow(
            label = stringResource(R.string.settings_agent_chat),
            note = stringResource(R.string.settings_agent_chat_note),
            checked = chatOn,
            onCheckedChange = onChatOn,
        )
    }
}

/** The machines whose programs read the clipboard unasked, each with Ask again; or that every machine asks. */
@Composable
internal fun ClipboardSection(
    trusted: List<TrustedMachine>,
    onRevoke: (String) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_clipboard)) {
        if (trusted.isEmpty()) {
            SettingsNote(stringResource(R.string.settings_clipboard_empty))
            return@SettingsSection
        }
        SettingsNote(stringResource(R.string.settings_clipboard_note))
        for (machine in trusted) {
            val revoke = stringResource(R.string.settings_clipboard_revoke_description, machine.title)
            Row(
                modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        machine.title,
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Text(
                        machine.address,
                        style = NgtermTheme.mono.small,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                TextButton(
                    onClick = { onRevoke(machine.id) },
                    modifier = Modifier.semantics { contentDescription = revoke },
                ) {
                    Text(stringResource(R.string.settings_clipboard_revoke))
                }
            }
        }
    }
}

/**
 * The app's keys, each with its fingerprint, the machines that use it, Show public key, Copy and Export private
 * key; then Rotate key
 * (spinning while [work] makes one, and why it failed) and Import a key.
 */
@Composable
internal fun KeysSection(
    keys: List<KeyItem>,
    work: KeyWork,
    actions: SettingsActions,
) {
    val spacing = NgtermTheme.spacing
    SettingsSection(stringResource(R.string.settings_keys)) {
        Column(
            Modifier.padding(vertical = spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            for (item in keys) {
                KeyCard(
                    item,
                    onShow = { actions.onShowKey(item.key.id) },
                    onExport = { actions.onExport(item.key.id) },
                )
            }
        }
        SettingsNote(stringResource(R.string.settings_rotate_note))
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            OutlinedButton(onClick = { if (!work.rotating) actions.onRotate() }) {
                if (work.rotating) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    Text(stringResource(R.string.settings_rotating), Modifier.padding(start = spacing.sm))
                } else {
                    Text(stringResource(R.string.settings_rotate))
                }
            }
            TextButton(onClick = actions.onImport) { Text(stringResource(R.string.settings_keys_import)) }
        }
        work.rotateFailure?.let {
            Problem(stringResource(R.string.settings_rotate_failed, it), Modifier.padding(top = spacing.sm))
        }
    }
}

// a key on its plate: where it came from, its fingerprint, who uses it, its public half's actions and its export
@Composable
private fun KeyCard(
    item: KeyItem,
    onShow: () -> Unit,
    onExport: () -> Unit,
) {
    val spacing = NgtermTheme.spacing
    Column(
        Modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.medium)
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(start = spacing.lg, end = spacing.sm, top = spacing.md, bottom = spacing.xs),
        verticalArrangement = Arrangement.spacedBy(spacing.xs),
    ) {
        Text(
            stringResource(if (item.key.imported) R.string.settings_key_imported else R.string.settings_key_made),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Text(item.key.fingerprint, style = NgtermTheme.mono.small, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            if (item.usedBy.isEmpty()) {
                stringResource(R.string.settings_key_unused)
            } else {
                stringResource(R.string.settings_key_used_by, item.usedBy.joinToString(", "))
            },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        FlowRow(Modifier.padding(top = spacing.xs)) {
            TextButton(onClick = onShow) { Text(stringResource(R.string.settings_key_show)) }
            CopyButton(item.key.authorizedKey, label = stringResource(R.string.settings_key_copy))
            TextButton(onClick = onExport) { Text(stringResource(R.string.settings_key_export)) }
        }
    }
}

/**
 * The app's own updates (the GitHub build's): where updating stands ([UpdatePanel]), and whether the app checks on
 * its own.
 */
@Composable
internal fun UpdatesSection(updates: UpdatesUi) {
    SettingsSection(stringResource(R.string.settings_updates)) {
        UpdatePanel(
            installed = updates.installed,
            state = updates.state,
            actions = updates.actions,
            modifier = Modifier.padding(vertical = NgtermTheme.spacing.sm),
        )
        SwitchRow(
            label = stringResource(R.string.settings_update_automatic),
            note = stringResource(R.string.settings_update_automatic_note),
            checked = updates.automatic,
            onCheckedChange = updates.onAutomatic,
        )
    }
}

/** The way to the diagnostic log: a row that opens it, saying what it is for. */
@Composable
internal fun DiagnosticsSection(onShowLogs: () -> Unit) {
    SettingsSection(stringResource(R.string.settings_diagnostics)) {
        LinkRow(
            label = stringResource(R.string.settings_logs),
            note = stringResource(R.string.settings_logs_note),
            onClick = onShowLogs,
        )
    }
}

/** The app's name and the loaded core's [coreVersion]. */
@Composable
internal fun AboutSection(coreVersion: String) {
    SettingsSection(stringResource(R.string.settings_about)) {
        Column(Modifier.padding(vertical = NgtermTheme.spacing.sm)) {
            Text(
                stringResource(R.string.settings_about_name),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                stringResource(R.string.core_version, coreVersion),
                style = NgtermTheme.mono.medium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
