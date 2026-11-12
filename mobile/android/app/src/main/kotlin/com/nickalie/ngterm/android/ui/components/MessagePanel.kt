package com.nickalie.ngterm.android.ui.components

import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * What a screen shows instead of its content: an [icon] in [tint], a [title] saying what is the case, a [body] saying
 * what to do about it, and the one [action] that does it ([busy]: under way, taking no taps; not [enabled]: held by
 * work elsewhere), with at most one quieter [secondary] one under it (a text button: Retry after the fix the main
 * action points to). Every empty, error and offline state is one of these, so no screen is ever blank and none leaves
 * the user without a next step. Use [EmptyState], [ErrorState] or [OfflineState]. The panel scrolls itself when its
 * window is too short for it, so don't put it in a vertical scroller.
 */
@Composable
fun MessagePanel(
    @DrawableRes icon: Int,
    tint: Color,
    title: String,
    body: String,
    action: PanelAction,
    modifier: Modifier = Modifier,
    secondary: PanelAction? = null,
    busy: Boolean = false,
    enabled: Boolean = !busy,
) {
    val spacing = NgtermTheme.spacing
    Column(
        // scrolls when it is taller than its window (a landscape phone at a large font scale), so the action stays
        // reachable; centered by its host when it fits
        modifier = modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(spacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        Icon(painterResource(icon), contentDescription = null, tint = tint, modifier = Modifier.size(40.dp))
        Text(
            text = title,
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = body,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.widthIn(max = PANEL_TEXT_WIDTH.dp),
        )
        Button(
            onClick = action.onClick,
            enabled = enabled,
            modifier = Modifier.padding(top = spacing.sm),
            contentPadding = ButtonDefaults.ButtonWithIconContentPadding,
        ) {
            ActionLabel(action, busy)
        }
        if (secondary != null) {
            TextButton(onClick = secondary.onClick) { ActionLabel(secondary) }
        }
    }
}

@Composable
private fun RowScope.ActionLabel(
    action: PanelAction,
    busy: Boolean = false,
) {
    if (busy) {
        CircularProgressIndicator(
            modifier = Modifier.size(16.dp),
            strokeWidth = 2.dp,
            color = LocalContentColor.current,
        )
        Text(action.label, modifier = Modifier.padding(start = NgtermTheme.spacing.sm))
    } else if (action.icon != null) {
        Icon(painterResource(action.icon), contentDescription = null, modifier = Modifier.size(18.dp))
        Text(action.label, modifier = Modifier.padding(start = NgtermTheme.spacing.sm))
    } else {
        Text(action.label)
    }
}

/** A panel's one action: its button's words, its optional leading icon, and what it does. */
data class PanelAction(
    val label: String,
    @param:DrawableRes val icon: Int? = null,
    val onClick: () -> Unit,
)

/**
 * Nothing here yet: [action] makes the first one (Add machine, New session), [busy] while it does, and takes no taps
 * unless [enabled].
 */
@Composable
fun EmptyState(
    @DrawableRes icon: Int,
    title: String,
    body: String,
    action: PanelAction,
    modifier: Modifier = Modifier,
    busy: Boolean = false,
    enabled: Boolean = !busy,
) {
    MessagePanel(icon, MaterialTheme.colorScheme.primary, title, body, action, modifier, busy = busy, enabled = enabled)
}

/**
 * Something failed: [body] says what and how to fix it, [action] is the fix or a retry, and [secondary] a retry after
 * the fix.
 */
@Composable
fun ErrorState(
    title: String,
    body: String,
    action: PanelAction,
    modifier: Modifier = Modifier,
    secondary: PanelAction? = null,
) {
    MessagePanel(R.drawable.ic_alert, NgtermTheme.status.error, title, body, action, modifier, secondary)
}

/** The machine or the network is out of reach: [action] tries again now rather than at the next retry. */
@Composable
fun OfflineState(
    title: String,
    body: String,
    action: PanelAction,
    modifier: Modifier = Modifier,
) {
    MessagePanel(R.drawable.ic_offline, NgtermTheme.status.offline, title, body, action, modifier)
}

private const val PANEL_TEXT_WIDTH = 420
