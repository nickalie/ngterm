package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A session in a machine's list: its [name], the note of what it is for ([context], one line), its agent's [status]
 * (an [AgentStatusBadge], or nothing for an idle one), and an Attached mark when this app has it attached. A page
 * companion ([companionOf], its parent's name) sits indented under its parent. [selected] puts the bus bar on the
 * session on screen, where a list is also the navigation (the expanded sidebar).
 */
@Composable
fun SessionRow(
    name: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    context: String? = null,
    attached: Boolean = false,
    selected: Boolean = false,
    companionOf: String? = null,
    status: @Composable () -> Unit = {},
) {
    val spacing = NgtermTheme.spacing
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = spacing.touchTarget + spacing.sm)
                .clip(MaterialTheme.shapes.medium)
                .selectable(selected = selected, role = Role.Button, onClick = onClick)
                .padding(end = spacing.lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BusBar(active = selected)
        Box(Modifier.width(if (companionOf != null) spacing.xxl + spacing.sm else spacing.md))
        Column(
            modifier = Modifier.weight(1f).padding(vertical = spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.xxs),
        ) {
            Text(
                text = name,
                style = MaterialTheme.typography.titleSmall,
                color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Detail(companionOf?.let { stringResource(R.string.session_companion, it) } ?: context)
            status()
        }
        if (attached) {
            Text(
                text = stringResource(R.string.session_attached),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.secondary,
                modifier = Modifier.padding(start = spacing.sm),
            )
        }
    }
}

@Composable
private fun Detail(
    text: String?,
    modifier: Modifier = Modifier,
) {
    if (text == null) return
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}
