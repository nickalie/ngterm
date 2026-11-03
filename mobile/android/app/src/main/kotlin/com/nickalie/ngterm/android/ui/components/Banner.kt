package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone

/**
 * A strip across the top of a screen about something that holds for the whole screen while the content stays usable
 * ("Reconnecting in 4 s", "Showing the last list"): the [tone]'s glyph and color, the [message], an optional [action]
 * and an optional dismiss. TalkBack announces it when it appears or changes.
 */
@Composable
fun Banner(
    tone: StatusTone,
    message: String,
    modifier: Modifier = Modifier,
    action: SectionAction? = null,
    onDismiss: (() -> Unit)? = null,
) {
    val spacing = NgtermTheme.spacing
    Surface(
        modifier = modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.small,
    ) {
        Row(
            modifier = Modifier.heightIn(min = spacing.touchTarget).padding(start = spacing.lg),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            StatusGlyph(tone.shape, tone.color(NgtermTheme.status))
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f).padding(horizontal = spacing.md, vertical = spacing.sm),
            )
            if (action != null) {
                TextButton(onClick = action.onClick) { Text(action.label) }
            }
            if (onDismiss != null) {
                IconButton(onClick = onDismiss) {
                    Icon(
                        painterResource(R.drawable.ic_close),
                        contentDescription = stringResource(R.string.action_dismiss),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
