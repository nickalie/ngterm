package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A group's heading in a list ("Attached", "On this machine"), in sentence case, with its [count] after it when that
 * says something, and one [action] at the end (a text button) when the group has one.
 */
@Composable
fun SectionHeader(
    title: String,
    modifier: Modifier = Modifier,
    count: Int? = null,
    action: SectionAction? = null,
) {
    val spacing = NgtermTheme.spacing
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = spacing.touchTarget).padding(start = spacing.lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // the heading gives way first: the action keeps its words whole and the title ellipsizes
        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false).semantics { heading() },
            )
            if (count != null) {
                Text(
                    text = count.toString(),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = spacing.sm),
                )
            }
        }
        if (action != null) {
            TextButton(onClick = action.onClick) { Text(action.label, maxLines = 1, softWrap = false) }
        }
    }
}

/** A [SectionHeader]'s action: what its button says and does. */
data class SectionAction(
    val label: String,
    val onClick: () -> Unit,
)
