package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/** The chat's backlog on its way, with nothing to show yet: a spinner and what is under way, read from [machine]. */
@Composable
fun ChatLoading(
    machine: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.padding(NgtermTheme.spacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.lg),
    ) {
        CircularProgressIndicator()
        Text(
            text = stringResource(R.string.chat_loading, machine),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
    }
}

/**
 * The chat's link dropped and resumes on its own: a strip over the top of the rows, which stay readable under it, with
 * a spinner and [machine]'s name. Announced politely.
 */
@Composable
fun ReconnectingStrip(
    machine: String,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Surface(
        modifier = modifier.fillMaxWidth().semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.small,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = spacing.lg, vertical = spacing.sm),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.md),
        ) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            Column {
                Text(
                    stringResource(R.string.chat_reconnecting_title, machine),
                    style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    stringResource(R.string.chat_reconnecting_body),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * The top of the list, by [older]: earlier messages on their way (a page is asked for as soon as this shows), the
 * conversation's start with its working directory ([cwd]), or why the last page failed with Retry ([onRetry]).
 */
@Composable
fun OlderHeader(
    older: OlderPages,
    cwd: String?,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = spacing.touchTarget).padding(horizontal = spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        when (older) {
            OlderPages.More, OlderPages.Loading -> {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Text(
                    stringResource(R.string.chat_older_loading),
                    style = MaterialTheme.typography.bodySmall,
                    color = quiet,
                )
            }

            OlderPages.AtStart -> {
                Column(Modifier.weight(1f)) {
                    Text(
                        stringResource(R.string.chat_older_start),
                        style = MaterialTheme.typography.bodySmall,
                        color = quiet,
                    )
                    cwd?.let { Text(it, style = NgtermTheme.mono.small, color = quiet) }
                }
            }

            is OlderPages.Failed -> {
                Text(
                    stringResource(R.string.chat_older_failed, older.reason),
                    style = MaterialTheme.typography.bodySmall,
                    color = NgtermTheme.status.error,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
            }
        }
    }
}
