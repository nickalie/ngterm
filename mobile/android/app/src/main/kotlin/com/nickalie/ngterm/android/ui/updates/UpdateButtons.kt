package com.nickalie.ngterm.android.ui.updates

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.FlowRowScope
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.update.UpdateState

/** What can be done next in [state]: one filled button at most, the rest quieter; none while it works on its own. */
@Composable
internal fun UpdateButtons(
    state: UpdateState,
    actions: UpdateActions,
    modifier: Modifier = Modifier,
) {
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm)) {
        StateButtons(state, actions)
    }
}

@Composable
private fun FlowRowScope.StateButtons(
    state: UpdateState,
    actions: UpdateActions,
) {
    when (state) {
        UpdateState.Idle, UpdateState.Current -> {
            OutlinedButton(onClick = actions.onCheck) { Text(stringResource(R.string.update_check)) }
        }

        is UpdateState.Available -> {
            Button(onClick = actions.onInstall) { Text(stringResource(R.string.update_install)) }
            TextButton(
                onClick = { actions.onNotes(state.offer.notesUrl) },
            ) { Text(stringResource(R.string.update_notes)) }
            if (!state.skipped) TextButton(onClick = actions.onSkip) { Text(stringResource(R.string.update_skip)) }
        }

        is UpdateState.Downloading -> {
            TextButton(onClick = actions.onCancel) { Text(stringResource(R.string.update_cancel)) }
        }

        is UpdateState.Installing -> {
            if (state.waitingForYou) {
                Button(onClick = actions.onInstall) { Text(stringResource(R.string.update_open_installer)) }
            }
        }

        is UpdateState.Failed -> {
            val retry = if (state.offer == null) actions.onCheck else actions.onInstall
            OutlinedButton(onClick = retry) { Text(stringResource(R.string.update_try_again)) }
            state.offer?.let { offer ->
                TextButton(
                    onClick = { actions.onNotes(offer.notesUrl) },
                ) { Text(stringResource(R.string.update_notes)) }
            }
        }

        UpdateState.Checking, is UpdateState.Verifying -> {}
    }
}
