package com.nickalie.ngterm.android.ui.sessions

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
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.ConfirmDialog
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.OfflineState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.components.SectionHeader
import com.nickalie.ngterm.android.ui.currentNavLayout
import com.nickalie.ngterm.android.ui.machines.countdown
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone

/**
 * A machine's sessions: a top row with Back, the machine's name and address, and New session; then the sessions this
 * app has attached and the others the machine runs, companions under their parents, with pull to refresh. Instead of
 * the list it shows what the machine's link or listing calls for ([SessionsBody]), always with the one action that
 * moves on: Connect, Retry, Review, New session, or the README when the machine has no `ngtermctl`. [now] is the clock
 * a reconnect's countdown counts against.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionsScreen(
    state: MachineSessions,
    actions: SessionActions,
    now: () -> Long,
    modifier: Modifier = Modifier,
) {
    val connected = state.link == Link.Connected
    // the session whose End waits for its confirmation
    var confirming by rememberSaveable { mutableStateOf<String?>(null) }
    val asking = { id: String -> confirming = id }
    Column(modifier = modifier.fillMaxSize().safeDrawingPadding()) {
        TopRow(state, actions, newSession = connected)
        PullToRefreshBox(
            isRefreshing = state.refreshing,
            onRefresh = if (connected) actions.onRefresh else actions.onConnect,
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Body(state, actions, asking, now)
        }
        state.sessions?.firstOrNull { it.id == confirming }?.let { session ->
            ConfirmDialog(
                title = stringResource(R.string.machine_sessions_end_title, session.name),
                body = stringResource(R.string.machine_sessions_end_body, state.machine.title),
                confirmLabel = stringResource(R.string.machine_sessions_end),
                onConfirm = {
                    confirming = null
                    actions.onEnd(session.id)
                },
                onDismiss = { confirming = null },
            )
        }
    }
}

@Composable
private fun TopRow(
    state: MachineSessions,
    actions: SessionActions,
    newSession: Boolean,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = spacing.touchTarget + spacing.sm)
                .padding(horizontal = spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = actions.onBack) {
            Icon(painterResource(R.drawable.ic_back), contentDescription = stringResource(R.string.action_back))
        }
        Column(Modifier.weight(1f).padding(horizontal = spacing.xs)) {
            Text(
                text = state.machine.title,
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                text = state.machine.address,
                style = NgtermTheme.mono.small,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                // compared character by character: it wraps at a large font scale rather than losing its end
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (newSession) {
            TextButton(onClick = actions.onNewSession) {
                Icon(painterResource(R.drawable.ic_add), contentDescription = null, modifier = Modifier.size(18.dp))
                Text(
                    stringResource(R.string.machine_sessions_new),
                    modifier = Modifier.padding(start = spacing.sm),
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
private fun Body(
    state: MachineSessions,
    actions: SessionActions,
    onAskEnd: (String) -> Unit,
    now: () -> Long,
) {
    val name = state.machine.title
    when (val body = state.body()) {
        SessionsBody.Connecting -> Waiting(stringResource(R.string.machine_sessions_connecting, name))
        SessionsBody.Reading -> Waiting(stringResource(R.string.machine_sessions_reading, name))
        is SessionsBody.Sessions -> Listing(state, body, actions, onAskEnd)
        is SessionsBody.Empty -> Empty(state, body.failure, actions)
        else -> Problem(name, body, actions, now)
    }
}

@Composable
private fun Waiting(
    label: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(NgtermTheme.spacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.lg),
    ) {
        CircularProgressIndicator()
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier =
                Modifier.semantics {
                    contentDescription = label
                    liveRegion = LiveRegionMode.Polite
                },
        )
    }
}

// what is said over the list or the empty state: the device is offline, the last refresh failed
@Composable
private fun Banners(
    offline: Boolean,
    failure: String?,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm)) {
        if (offline) Banner(StatusTone.Offline, stringResource(R.string.machines_offline))
        if (failure != null) {
            Banner(
                tone = StatusTone.Error,
                message = stringResource(R.string.machine_sessions_failed_banner, failure),
                action = SectionAction(stringResource(R.string.action_retry), onRetry),
            )
        }
    }
}

// no sessions: the invitation to start one, under what the banners say (the panel scrolls, so pull to refresh works)
@Composable
private fun Empty(
    state: MachineSessions,
    failure: String?,
    actions: SessionActions,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val name = state.machine.title
    Column(
        modifier = modifier.fillMaxSize().padding(horizontal = gutter(), vertical = spacing.sm),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Banners(state.offline, failure, actions.onRefresh, Modifier.widthIn(max = spacing.readableWidth))
        Box(Modifier.weight(1f), contentAlignment = Alignment.Center) {
            EmptyState(
                icon = R.drawable.ic_sessions,
                title = stringResource(R.string.machine_sessions_empty_title, name),
                body = stringResource(R.string.machine_sessions_empty_body, name),
                action =
                    PanelAction(
                        stringResource(R.string.machine_sessions_new),
                        R.drawable.ic_add,
                        actions.onNewSession,
                    ),
            )
        }
    }
}

@Composable
private fun gutter() = if (currentNavLayout() == NavLayout.Bar) NgtermTheme.spacing.lg else NgtermTheme.spacing.xl

@Composable
private fun Listing(
    state: MachineSessions,
    body: SessionsBody.Sessions,
    actions: SessionActions,
    onAskEnd: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val name = state.machine.title
    val wide = Modifier.widthIn(max = spacing.readableWidth)
    val attached = stringResource(R.string.machine_sessions_attached)
    val others = stringResource(R.string.machine_sessions_others, name)
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        contentPadding = PaddingValues(horizontal = gutter(), vertical = spacing.sm),
        verticalArrangement = Arrangement.spacedBy(spacing.xs),
    ) {
        if (state.offline || body.failure != null) {
            item(key = "banners") {
                Banners(state.offline, body.failure, actions.onRefresh, wide.padding(bottom = spacing.sm))
            }
        }
        group(attached, body.attached, actions, onAskEnd, wide)
        group(others, body.others, actions, onAskEnd, wide)
        item(key = "end") { Box(Modifier.heightIn(min = spacing.xxxl)) }
    }
}

private fun LazyListScope.group(
    title: String,
    lines: List<SessionLine>,
    actions: SessionActions,
    onAskEnd: (String) -> Unit,
    modifier: Modifier,
) {
    if (lines.isEmpty()) return
    item(key = "header:$title") {
        SectionHeader(title = title, count = lines.size, modifier = modifier.animateItem())
    }
    items(lines, key = { it.session.id }) { line ->
        SessionEntry(line, actions, { onAskEnd(line.session.id) }, modifier.animateItem())
    }
}
