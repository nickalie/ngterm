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
 * [nested] is separate from [companionOf] because Sessions indents a companion only under its attached parent, so one
 * whose parent isn't attached never reads as another session's; a machine's list indents every companion.
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
    nested: Boolean = companionOf != null,
    menu: (@Composable () -> Unit)? = null,
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
                .padding(end = if (menu != null) spacing.xs else spacing.lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BusBar(active = selected)
        Box(Modifier.width(if (nested) spacing.xxl + spacing.sm else spacing.md))
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
        menu?.invoke()
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
