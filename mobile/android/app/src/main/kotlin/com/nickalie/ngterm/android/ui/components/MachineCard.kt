package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone

/**
 * A machine on the Machines home: its [label], its ssh [address] (`user@host:port`, in mono: it is compared
 * character by character) with the app it runs beside it when that isn't ngterm ([peer], a [PeerBadge]: the sessions
 * screen's says what that app lacks), its connection as a [StatusBadge], and [summary] under them, the sessions' aggregate
 * status (an [AgentStatusBadge] or a line of text, and the action the state calls for) or nothing. [menu], when
 * given, ends the top row (an icon button opening the machine's menu). A flat tonal card, no shadow: depth is for
 * what floats. When the name and a long connection label don't fit side by side, the label moves under the name
 * rather than cutting it short.
 */
@Composable
fun MachineCard(
    label: String,
    address: String,
    connection: StatusTone,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    connectionLabel: String? = null,
    peer: String? = null,
    menu: (@Composable () -> Unit)? = null,
    summary: @Composable () -> Unit = {},
) {
    val spacing = NgtermTheme.spacing
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().heightIn(min = spacing.touchTarget),
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = spacing.lg, vertical = spacing.md),
            verticalArrangement = Arrangement.spacedBy(spacing.xs),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                FlowRow(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    itemVerticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(end = spacing.sm),
                    )
                    StatusBadge(connection, label = connectionLabel ?: stringResource(connection.label))
                }
                menu?.invoke()
            }
            Row(
                horizontalArrangement = Arrangement.spacedBy(spacing.sm),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = address,
                    style = NgtermTheme.mono.small,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                peer?.let { PeerBadge(it) }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(spacing.md),
                verticalArrangement = Arrangement.spacedBy(spacing.xs),
            ) {
                summary()
            }
        }
    }
}
