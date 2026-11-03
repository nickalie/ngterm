package com.nickalie.ngterm.android.ui

import androidx.activity.compose.BackHandler
import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfoV2
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.window.core.layout.WindowSizeClass
import com.nickalie.ngterm.android.ui.navigation.NavBar
import com.nickalie.ngterm.android.ui.navigation.NavRail
import com.nickalie.ngterm.android.ui.navigation.NavSidebar

/** A top-level destination: its key, its label and its line icon. */
data class NavDestination(
    val key: String,
    @param:StringRes val label: Int,
    @param:DrawableRes val icon: Int,
)

/** Where the top-level navigation goes, by the window's width class. */
enum class NavLayout {
    /** Compact (a phone): a bar along the bottom, the content full width above it. */
    Bar,

    /** Medium (a small tablet, an unfolded foldable in portrait): a rail that shows its labels on demand. */
    Rail,

    /** Expanded (a tablet, landscape, a desktop window): a permanent sidebar with room for a list of its own. */
    Sidebar,
    ;

    companion object {
        /** The layout for a window of [sizeClass]. */
        fun of(sizeClass: WindowSizeClass): NavLayout =
            when {
                sizeClass.isWidthAtLeastBreakpoint(WindowSizeClass.WIDTH_DP_EXPANDED_LOWER_BOUND) -> Sidebar
                sizeClass.isWidthAtLeastBreakpoint(WindowSizeClass.WIDTH_DP_MEDIUM_LOWER_BOUND) -> Rail
                else -> Bar
            }
    }
}

/** The layout for the current window, which follows rotation, folding, multi-window and freeform resizing. */
@Composable
fun currentNavLayout(): NavLayout = NavLayout.of(currentWindowAdaptiveInfoV2().windowSizeClass)

/**
 * The system bars and the display cutout: what the navigation keeps clear of. The keyboard is not among them, so the
 * bottom bar stays put under it rather than riding up on it.
 */
internal val navigationInsets: WindowInsets
    @Composable get() = WindowInsets.systemBars.union(WindowInsets.displayCutout)

/**
 * The app's frame: the top-level [destinations] as a bottom bar, a rail or a sidebar ([layout], by default the
 * window's width class), and [content] beside or above them, edge to edge.
 *
 * The frame pads its navigation for the system bars and the cutout and consumes those sides (under the bottom bar,
 * the bar's whole height), so [content] pads only for what is left (its own `safeDrawingPadding` and the like), and
 * the keyboard meets its bottom edge. [content] is movable: a change of [layout] (a rotation, a fold, a resize)
 * moves it rather than recreating it, and a new [content] lambda recomposes it in place, so nothing in it loses its
 * state. [sidebarContent]
 * fills the sidebar under the destinations, where the expanded layout has room for a list (machines and their
 * sessions), and [railContent] the rail under its destinations (told whether the rail is open); the bottom bar shows
 * neither, and steps aside while [navigationVisible] is false (a phone's full-screen terminal).
 *
 * Back: the rail's labels close first (predictively, following the gesture), then a destination other than the first
 * goes back to the first, and only from the first does back leave the app.
 */
@Composable
fun AdaptiveScaffold(
    destinations: List<NavDestination>,
    selected: String,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    layout: NavLayout = currentNavLayout(),
    navigationVisible: Boolean = true,
    railContent: (@Composable ColumnScope.(expanded: Boolean) -> Unit)? = null,
    sidebarContent: @Composable ColumnScope.() -> Unit = {},
    content: @Composable () -> Unit,
) {
    val start = destinations.first().key
    BackHandler(enabled = selected != start) { onSelect(start) }
    var railExpanded by rememberSaveable { mutableStateOf(false) }
    // keyed on nothing: a new lambda (one capturing a changed value) recomposes the content rather than replacing it
    val current by rememberUpdatedState(content)
    val movable = remember { movableContentOf { current() } }
    Surface(modifier = modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        when (layout) {
            NavLayout.Bar -> {
                BarFrame(destinations, selected, onSelect, navigationVisible, movable)
            }

            NavLayout.Rail -> {
                Row {
                    NavRail(
                        destinations = destinations,
                        selected = selected,
                        onSelect = onSelect,
                        expanded = railExpanded,
                        onExpandedChange = { railExpanded = it },
                        content = railContent,
                    )
                    ContentBeside(movable)
                }
            }

            NavLayout.Sidebar -> {
                Row {
                    NavSidebar(destinations, selected, onSelect, content = sidebarContent)
                    ContentBeside(movable)
                }
            }
        }
    }
}

@Composable
private fun BarFrame(
    destinations: List<NavDestination>,
    selected: String,
    onSelect: (String) -> Unit,
    visible: Boolean,
    content: @Composable () -> Unit,
) {
    // the bar's whole height (its own and the navigation bar's) is below the content, so the content consumes it:
    // the keyboard's inset then counts from the content's bottom edge, not the screen's
    var barHeight by remember { mutableIntStateOf(0) }
    val barHeightDp = if (visible) with(LocalDensity.current) { barHeight.toDp() } else 0.dp
    Column {
        Box(
            modifier =
                Modifier
                    .weight(1f)
                    .consumeWindowInsets(PaddingValues(bottom = barHeightDp)),
        ) { content() }
        if (visible) NavBar(destinations, selected, onSelect, Modifier.onSizeChanged { barHeight = it.height })
    }
}

@Composable
private fun RowScope.ContentBeside(content: @Composable () -> Unit) {
    Box(
        modifier =
            Modifier
                .weight(1f)
                .fillMaxHeight()
                .consumeWindowInsets(navigationInsets.only(WindowInsetsSides.Start)),
    ) { content() }
}
