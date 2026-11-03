package com.nickalie.ngterm.android.ui.navigation

import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.VectorConverter
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.lerp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.NavDestination
import com.nickalie.ngterm.android.ui.navigationInsets
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** The collapsed rail's width (an icon and a short label), beside a start-side system bar or cutout. */
val RailWidth: Dp = 88.dp

/** The open rail's width (icons beside their labels), beside a start-side system bar or cutout. */
val RailExpandedWidth: Dp = 248.dp

/** The permanent sidebar's width, beside a start-side system bar or cutout. */
val SidebarWidth: Dp = 296.dp

/** The compact layout's bottom bar, clear of the navigation bar. */
@Composable
internal fun NavBar(
    destinations: List<NavDestination>,
    selected: String,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .windowInsetsPadding(navigationInsets.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom))
                .selectableGroup(),
    ) {
        destinations.forEach { destination ->
            BarItem(destination, destination.key == selected, { onSelect(destination.key) }, Modifier.weight(1f))
        }
    }
}

/**
 * The medium layout's rail, which opens to show its labels beside the icons ([expanded]). Back closes it, and the
 * predictive back gesture narrows it as the finger moves, so letting go finishes what the user already sees.
 */
@Composable
internal fun NavRail(
    destinations: List<NavDestination>,
    selected: String,
    onSelect: (String) -> Unit,
    expanded: Boolean,
    onExpandedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val motion = NgtermTheme.motion
    val width = remember { Animatable(if (expanded) RailExpandedWidth else RailWidth, Dp.VectorConverter) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(expanded) { width.animateTo(if (expanded) RailExpandedWidth else RailWidth, motion.standard()) }
    PredictiveBackHandler(enabled = expanded) { gesture ->
        try {
            gesture.collect { width.snapTo(lerp(RailExpandedWidth, RailWidth, it.progress)) }
            onExpandedChange(false)
        } catch (cancelled: CancellationException) {
            scope.launch { width.animateTo(RailExpandedWidth, motion.quick()) }
            throw cancelled
        }
    }
    Column(
        modifier =
            modifier
                .fillMaxHeight()
                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                .windowInsetsPadding(navigationInsets.only(WindowInsetsSides.Start + WindowInsetsSides.Vertical))
                .width(width.value)
                .clipToBounds(),
    ) {
        IconButton(
            onClick = { onExpandedChange(!expanded) },
            modifier = Modifier.padding(start = NgtermTheme.spacing.lg, top = NgtermTheme.spacing.sm),
        ) {
            Icon(
                painterResource(if (expanded) R.drawable.ic_menu_open else R.drawable.ic_menu),
                contentDescription =
                    stringResource(
                        if (expanded) R.string.nav_hide_labels else R.string.nav_show_labels,
                    ),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Column(
            modifier = Modifier.padding(top = NgtermTheme.spacing.lg).selectableGroup(),
            verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
        ) {
            destinations.forEach { destination ->
                val pick = { onSelect(destination.key) }
                if (expanded) {
                    RowItem(destination, destination.key == selected, pick)
                } else {
                    RailItem(destination, destination.key == selected, pick)
                }
            }
        }
    }
}

/**
 * The expanded layout's permanent sidebar: ngterm's mark and name, the destinations, and under a rule [content], the
 * sidebar's own list (machines and their sessions), which takes the rest of the height.
 */
@Composable
internal fun NavSidebar(
    destinations: List<NavDestination>,
    selected: String,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit = {},
) {
    val spacing = NgtermTheme.spacing
    Column(
        modifier =
            modifier
                .fillMaxHeight()
                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                .windowInsetsPadding(navigationInsets.only(WindowInsetsSides.Start + WindowInsetsSides.Vertical))
                .width(SidebarWidth),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = spacing.xl, vertical = spacing.lg),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Image(
                painterResource(R.drawable.ic_ngterm_mark),
                contentDescription = null,
                modifier = Modifier.size(28.dp),
            )
            Text(
                text = stringResource(R.string.app_name),
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.padding(start = spacing.md),
            )
        }
        Column(Modifier.selectableGroup(), verticalArrangement = Arrangement.spacedBy(spacing.xxs)) {
            destinations.forEach { destination ->
                RowItem(destination, destination.key == selected, { onSelect(destination.key) })
            }
        }
        HorizontalDivider(
            modifier = Modifier.padding(horizontal = spacing.lg, vertical = spacing.md),
            color = MaterialTheme.colorScheme.outlineVariant,
        )
        Column(Modifier.weight(1f), content = content)
    }
}
