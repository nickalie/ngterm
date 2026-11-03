package com.nickalie.ngterm.android.ui.navigation

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.NavDestination
import com.nickalie.ngterm.android.ui.components.BusBar
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

// The three shapes of a destination: a bottom-bar cell, a narrow rail cell (icon over a short label), and a wide row
// (icon beside its label: the open rail and the sidebar). All three are tabs to TalkBack, all mark the selected one
// with the bus bar and the primary color, and all are at least 48dp tall.

@Composable
private fun tint(selected: Boolean): Color {
    val color by animateColorAsState(
        targetValue = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        animationSpec = NgtermTheme.motion.quick(),
        label = "destination tint",
    )
    return color
}

private fun Modifier.destination(
    selected: Boolean,
    onClick: () -> Unit,
) = selectable(selected = selected, role = Role.Tab, onClick = onClick)

/** A bottom bar's cell: the bus bar on its top edge, the icon, the label under it. */
@Composable
internal fun BarItem(
    destination: NavDestination,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val color = tint(selected)
    Column(
        modifier = modifier.heightIn(min = BAR_HEIGHT.dp).destination(selected, onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        BusBar(active = selected, vertical = false, length = 32.dp)
        Icon(
            painterResource(destination.icon),
            contentDescription = null,
            tint = color,
            modifier = Modifier.padding(top = NgtermTheme.spacing.sm).size(24.dp),
        )
        DestinationLabel(
            destination,
            color,
            MaterialTheme.typography.labelMedium,
            TextAlign.Center,
            Modifier.padding(bottom = NgtermTheme.spacing.sm),
        )
    }
}

/** A collapsed rail's cell: the bus bar at its leading edge, the icon over a short label. */
@Composable
internal fun RailItem(
    destination: NavDestination,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val color = tint(selected)
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = RAIL_ITEM_HEIGHT.dp).destination(selected, onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BusBar(active = selected)
        Column(
            modifier = Modifier.weight(1f).padding(end = NgtermTheme.spacing.busBar),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
        ) {
            Icon(
                painterResource(destination.icon),
                contentDescription = null,
                tint = color,
                modifier = Modifier.size(24.dp),
            )
            DestinationLabel(destination, color, MaterialTheme.typography.labelSmall, TextAlign.Center)
        }
    }
}

/** A wide row: the bus bar at its leading edge, the icon, the label beside it. */
@Composable
internal fun RowItem(
    destination: NavDestination,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val color = tint(selected)
    val spacing = NgtermTheme.spacing
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = spacing.touchTarget + spacing.xs)
                .padding(end = spacing.md)
                .clip(MaterialTheme.shapes.small)
                .destination(selected, onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BusBar(active = selected)
        Box(Modifier.padding(start = spacing.lg, end = spacing.md)) {
            Icon(
                painterResource(destination.icon),
                contentDescription = null,
                tint = color,
                modifier = Modifier.size(24.dp),
            )
        }
        DestinationLabel(destination, color, MaterialTheme.typography.titleSmall, TextAlign.Start)
    }
}

@Composable
private fun DestinationLabel(
    destination: NavDestination,
    color: Color,
    style: TextStyle,
    align: TextAlign,
    modifier: Modifier = Modifier,
) {
    Text(
        text = stringResource(destination.label),
        color = color,
        style = style,
        textAlign = align,
        maxLines = 1,
        softWrap = false,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}

internal const val BAR_HEIGHT = 64
private const val RAIL_ITEM_HEIGHT = 64
