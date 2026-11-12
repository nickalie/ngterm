package com.nickalie.ngterm.android.ui.updates

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.update.AppUpdates
import com.nickalie.ngterm.android.update.UpdateState

/** The update sheet, up while [updates] asks for it (an update notification was tapped); closing it leaves it. */
@Composable
fun UpdatePrompt(updates: AppUpdates?) {
    updates ?: return
    val shown by updates.prompt.collectAsStateWithLifecycle()
    if (!shown) return
    val state by updates.state.collectAsStateWithLifecycle()
    val actions = rememberUpdateActions(updates)
    UpdateSheet(
        installed = updates.installed,
        state = state,
        actions = actions,
        onClose = updates::promptClosed,
    )
}

/**
 * [UpdatePanel] in a sheet under its title, the sheet as its plate; dismissing it is [onClose], and changes nothing of
 * the update.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UpdateSheet(
    installed: String,
    state: UpdateState,
    actions: UpdateActions,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    ModalBottomSheet(
        onDismissRequest = onClose,
        modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .navigationBarsPadding()
                    .padding(horizontal = spacing.xl)
                    .padding(bottom = spacing.lg),
            verticalArrangement = Arrangement.spacedBy(spacing.lg),
        ) {
            Text(
                text = stringResource(R.string.settings_updates),
                style = MaterialTheme.typography.headlineSmall,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.semantics { heading() },
            )
            UpdatePanel(installed, state, actions, plate = false)
        }
    }
}
