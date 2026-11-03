package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.OfflineState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.machines.countdown

// the link or the listing is in the way: say what and offer the one action that moves on
@Composable
internal fun Problem(
    name: String,
    body: SessionsBody,
    actions: SessionActions,
    now: () -> Long,
) {
    val retry = PanelAction(stringResource(R.string.action_retry), R.drawable.ic_retry, actions.onConnect)
    when (body) {
        is SessionsBody.NotConnected -> {
            NotConnected(name, body.deviceOffline, actions)
        }

        is SessionsBody.Reconnecting -> {
            Reconnecting(name, body.retryAt, now, actions)
        }

        is SessionsBody.Unreachable -> {
            ErrorState(stringResource(R.string.machine_sessions_unreachable_title, name), body.reason, retry)
        }

        SessionsBody.HostKeyChanged -> {
            ErrorState(
                title = stringResource(R.string.machine_sessions_key_changed_title, name),
                body = stringResource(R.string.machine_sessions_key_changed_body),
                action = PanelAction(stringResource(R.string.machine_review_key), onClick = actions.onReviewHostKey),
            )
        }

        is SessionsBody.MissingCli -> {
            ErrorState(
                title = stringResource(R.string.machine_sessions_missing_cli_title, name),
                body = stringResource(R.string.machine_sessions_missing_cli_body),
                action =
                    PanelAction(
                        stringResource(R.string.machine_sessions_open_readme),
                        onClick = actions.onOpenReadme,
                    ),
                secondary =
                    PanelAction(
                        stringResource(R.string.action_retry),
                        R.drawable.ic_retry,
                        actions.onRefresh,
                    ),
            )
        }

        is SessionsBody.ListFailed -> {
            ErrorState(
                title = stringResource(R.string.machine_sessions_failed_title, name),
                body = body.reason,
                action = PanelAction(stringResource(R.string.action_retry), R.drawable.ic_retry, actions.onRefresh),
            )
        }

        // the waiting states and the list are the body's own
        else -> {}
    }
}

@Composable
private fun NotConnected(
    name: String,
    deviceOffline: Boolean,
    actions: SessionActions,
) {
    if (deviceOffline) {
        OfflineState(
            title = stringResource(R.string.machine_sessions_device_offline_title),
            body = stringResource(R.string.machine_sessions_device_offline_body, name),
            action = PanelAction(stringResource(R.string.action_retry), R.drawable.ic_retry, actions.onConnect),
        )
    } else {
        OfflineState(
            title = stringResource(R.string.machine_sessions_not_connected_title, name),
            body = stringResource(R.string.machine_sessions_not_connected_body),
            action = PanelAction(stringResource(R.string.machine_connect), onClick = actions.onConnect),
        )
    }
}

@Composable
private fun Reconnecting(
    name: String,
    retryAt: Long,
    now: () -> Long,
    actions: SessionActions,
) {
    val seconds = countdown(retryAt, now)
    OfflineState(
        title = stringResource(R.string.machine_sessions_dropped_title, name),
        body =
            if (seconds > 0) {
                stringResource(R.string.machine_sessions_dropped_body, seconds)
            } else {
                stringResource(R.string.machine_sessions_dropped_now)
            },
        action = PanelAction(stringResource(R.string.machine_retry_now), R.drawable.ic_retry, actions.onConnect),
    )
}
