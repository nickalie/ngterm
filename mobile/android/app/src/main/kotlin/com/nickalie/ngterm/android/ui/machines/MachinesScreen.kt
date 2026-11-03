package com.nickalie.ngterm.android.ui.machines

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.currentNavLayout
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone

/**
 * The Machines home: a card per machine with its connection and its sessions' status, and the action its state calls
 * for; Add machine at the top. While the list is read it shows a spinner, when it can't be read an error with Retry,
 * with no machines the invitation to add one, and over the list a banner while the device is offline. [now] is the
 * clock the reconnect countdowns count against.
 */
@Composable
fun MachinesScreen(
    state: MachinesUiState,
    actions: MachineActions,
    now: () -> Long,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
        when (state) {
            MachinesUiState.Loading -> {
                Loading()
            }

            is MachinesUiState.Failed -> {
                ErrorState(
                    title = stringResource(R.string.machines_failed_title),
                    body = stringResource(R.string.machines_failed_body, state.reason),
                    action =
                        PanelAction(
                            stringResource(R.string.action_retry),
                            R.drawable.ic_retry,
                            actions.onRetryRead,
                        ),
                )
            }

            is MachinesUiState.Ready -> {
                if (state.items.isEmpty()) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        if (state.offline) OfflineBanner(Modifier.padding(NgtermTheme.spacing.lg))
                        EmptyState(
                            icon = R.drawable.ic_machines,
                            title = stringResource(R.string.machines_empty_title),
                            body = stringResource(R.string.machines_empty_body),
                            action =
                                PanelAction(
                                    stringResource(R.string.machines_add),
                                    R.drawable.ic_add,
                                    actions.onAdd,
                                ),
                        )
                    }
                } else {
                    MachineList(state, actions, now)
                }
            }
        }
    }
}

@Composable
private fun Loading(modifier: Modifier = Modifier) {
    val label = stringResource(R.string.machines_loading)
    CircularProgressIndicator(
        modifier =
            modifier.semantics {
                contentDescription = label
                liveRegion = LiveRegionMode.Polite
            },
    )
}

@Composable
private fun OfflineBanner(modifier: Modifier = Modifier) {
    Banner(
        tone = StatusTone.Offline,
        message = stringResource(R.string.machines_offline),
        modifier = modifier.widthIn(max = NgtermTheme.spacing.readableWidth),
    )
}

@Composable
private fun MachineList(
    state: MachinesUiState.Ready,
    actions: MachineActions,
    now: () -> Long,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val gutter = if (currentNavLayout() == NavLayout.Bar) spacing.lg else spacing.xl
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        contentPadding = PaddingValues(horizontal = gutter, vertical = spacing.sm),
        verticalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        item(key = "header") {
            Header(onAdd = actions.onAdd, modifier = Modifier.widthIn(max = spacing.readableWidth))
        }
        if (state.offline) {
            item(key = "offline") { OfflineBanner() }
        }
        items(state.items, key = { it.machine.id }) { item ->
            MachineEntry(
                item = item,
                actions = actions,
                now = now,
                modifier = Modifier.widthIn(max = spacing.readableWidth).animateItem(),
            )
        }
        item(key = "end") { Box(Modifier.heightIn(min = spacing.xxxl)) }
    }
}

@Composable
private fun Header(
    onAdd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = NgtermTheme.spacing.touchTarget),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = stringResource(R.string.machines_title),
            style = MaterialTheme.typography.titleLarge,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f).semantics { heading() },
        )
        TextButton(onClick = onAdd) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = null, modifier = Modifier.size(18.dp))
            Text(stringResource(R.string.machines_add), modifier = Modifier.padding(start = NgtermTheme.spacing.sm))
        }
    }
}

/** What the Machines home does with a machine, as [MachinesViewModel] and the route do it. */
data class MachineActions(
    val onAdd: () -> Unit,
    val onEdit: (String) -> Unit,
    val onConnect: (String) -> Unit,
    val onDisconnect: (String) -> Unit,
    val onRemove: (String) -> Unit,
    val onReviewHostKey: (String) -> Unit,
    val onRetryRead: () -> Unit,
)
