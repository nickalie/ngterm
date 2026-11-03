package com.nickalie.ngterm.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.ConfirmDialog
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.components.SectionHeader
import com.nickalie.ngterm.android.ui.components.SessionRow
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

// The windows the sheets and the scaffold are rendered in: a phone (compact), an unfolded foldable (medium) and a
// landscape tablet (expanded). The component sheets get a tall window so nothing is cut off.
private const val PHONE = "w411dp-h891dp-xhdpi"
private const val PHONE_NIGHT = "w411dp-h891dp-night-xhdpi"
private const val PHONE_TALL = "w411dp-h1400dp-xhdpi"
private const val PHONE_TALL_NIGHT = "w411dp-h1400dp-night-xhdpi"
private const val FOLDABLE = "w700dp-h1000dp-hdpi"
private const val FOLDABLE_NIGHT = "w700dp-h1000dp-night-hdpi"
private const val TABLET = "w1280dp-h800dp-land-mdpi"
private const val TABLET_NIGHT = "w1280dp-h800dp-land-night-mdpi"
private const val LARGE_TYPE = 2f

/**
 * The design system rendered by Robolectric and compared with its goldens (src/test/screenshots/design): the
 * component sheets and the adaptive scaffold at the three widths, each light, dark, and light at font scale 2.0.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class DesignScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private fun capture(
        name: String,
        content: @Composable () -> Unit,
    ) {
        compose.setContent { NgtermTheme { content() } }
        compose.onRoot().captureRoboImage("src/test/screenshots/design/$name.png")
    }

    // the component sheets

    @Test
    @Config(qualifiers = PHONE_TALL)
    fun rowsCompactLight() = capture("rows_compact_light") { RowsSheet() }

    @Test
    @Config(qualifiers = PHONE_TALL_NIGHT)
    fun rowsCompactDark() = capture("rows_compact_dark") { RowsSheet() }

    @Test
    @Config(qualifiers = PHONE_TALL, fontScale = LARGE_TYPE)
    fun rowsCompactLargeType() = capture("rows_compact_font2") { RowsSheet() }

    @Test
    @Config(qualifiers = FOLDABLE)
    fun rowsMediumLight() = capture("rows_medium_light") { RowsSheet() }

    @Test
    @Config(qualifiers = FOLDABLE_NIGHT)
    fun rowsMediumDark() = capture("rows_medium_dark") { RowsSheet() }

    @Test
    @Config(qualifiers = FOLDABLE, fontScale = LARGE_TYPE)
    fun rowsMediumLargeType() = capture("rows_medium_font2") { RowsSheet() }

    @Test
    @Config(qualifiers = TABLET)
    fun rowsExpandedLight() = capture("rows_expanded_light") { RowsSheet() }

    @Test
    @Config(qualifiers = TABLET_NIGHT)
    fun rowsExpandedDark() = capture("rows_expanded_dark") { RowsSheet() }

    @Test
    @Config(qualifiers = TABLET, fontScale = LARGE_TYPE)
    fun rowsExpandedLargeType() = capture("rows_expanded_font2") { RowsSheet() }

    @Test
    @Config(qualifiers = PHONE_TALL)
    fun statesCompactLight() = capture("states_compact_light") { StatesSheet() }

    @Test
    @Config(qualifiers = PHONE_TALL_NIGHT)
    fun statesCompactDark() = capture("states_compact_dark") { StatesSheet() }

    @Test
    @Config(qualifiers = PHONE_TALL, fontScale = LARGE_TYPE)
    fun statesCompactLargeType() = capture("states_compact_font2") { StatesSheet() }

    @Test
    @Config(qualifiers = FOLDABLE)
    fun statesMediumLight() = capture("states_medium_light") { StatesSheet() }

    @Test
    @Config(qualifiers = FOLDABLE_NIGHT)
    fun statesMediumDark() = capture("states_medium_dark") { StatesSheet() }

    @Test
    @Config(qualifiers = FOLDABLE, fontScale = LARGE_TYPE)
    fun statesMediumLargeType() = capture("states_medium_font2") { StatesSheet() }

    @Test
    @Config(qualifiers = TABLET)
    fun statesExpandedLight() = capture("states_expanded_light") { StatesSheet() }

    @Test
    @Config(qualifiers = TABLET_NIGHT)
    fun statesExpandedDark() = capture("states_expanded_dark") { StatesSheet() }

    @Test
    @Config(qualifiers = TABLET, fontScale = LARGE_TYPE)
    fun statesExpandedLargeType() = capture("states_expanded_font2") { StatesSheet() }

    // the scaffold, its layout picked from the window as the app does

    @Test
    @Config(qualifiers = PHONE)
    fun scaffoldCompactLight() = capture("scaffold_compact_light") { ScaffoldSample() }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun scaffoldCompactDark() = capture("scaffold_compact_dark") { ScaffoldSample() }

    @Test
    @Config(qualifiers = PHONE, fontScale = LARGE_TYPE)
    fun scaffoldCompactLargeType() = capture("scaffold_compact_font2") { ScaffoldSample() }

    @Test
    @Config(qualifiers = FOLDABLE)
    fun scaffoldMediumLight() = capture("scaffold_medium_light") { ScaffoldSample() }

    @Test
    @Config(qualifiers = FOLDABLE_NIGHT)
    fun scaffoldMediumDark() = capture("scaffold_medium_dark") { ScaffoldSample() }

    @Test
    @Config(qualifiers = FOLDABLE, fontScale = LARGE_TYPE)
    fun scaffoldMediumLargeType() = capture("scaffold_medium_font2") { ScaffoldSample() }

    @Test
    @Config(qualifiers = FOLDABLE)
    fun scaffoldMediumRailOpen() {
        compose.setContent { NgtermTheme { ScaffoldSample() } }
        compose.onNodeWithContentDescription("Show navigation labels").performClick()
        compose.onRoot().captureRoboImage("src/test/screenshots/design/scaffold_medium_rail_open.png")
    }

    @Test
    @Config(qualifiers = TABLET)
    fun scaffoldExpandedLight() = capture("scaffold_expanded_light") { ScaffoldSample() }

    @Test
    @Config(qualifiers = TABLET_NIGHT)
    fun scaffoldExpandedDark() = capture("scaffold_expanded_dark") { ScaffoldSample() }

    @Test
    @Config(qualifiers = TABLET, fontScale = LARGE_TYPE)
    fun scaffoldExpandedLargeType() = capture("scaffold_expanded_font2") { ScaffoldSample() }

    // the confirm dialog, in its own window

    @OptIn(ExperimentalRoborazziApi::class)
    private fun captureDialog(name: String) {
        compose.setContent {
            NgtermTheme {
                ConfirmDialog(
                    title = "End api-refactor?",
                    body = "Its programs on build-01 are stopped and its scrollback is gone. This can't be undone.",
                    confirmLabel = "End session",
                    onConfirm = {},
                    onDismiss = {},
                )
            }
        }
        captureScreenRoboImage("src/test/screenshots/design/$name.png")
    }

    @Test
    @Config(qualifiers = PHONE)
    fun confirmDialogLight() = captureDialog("confirm_dialog_light")

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun confirmDialogDark() = captureDialog("confirm_dialog_dark")
}

/** The scaffold as the app frames it, with a machine's sessions in the expanded sidebar. */
@Composable
private fun ScaffoldSample(modifier: Modifier = Modifier) {
    var selected by remember { mutableStateOf(AppDestinations.first().key) }
    AdaptiveScaffold(
        destinations = AppDestinations,
        selected = selected,
        onSelect = { selected = it },
        modifier = modifier,
        sidebarContent = {
            SectionHeader("build-01", count = 2)
            SessionRow("api-refactor", onClick = {}, attached = true, selected = true) {
                AgentStatusBadge("Working", "#e3b65a", "star")
            }
            SessionRow("scratch", onClick = {})
        },
    ) {
        Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
            EmptyState(
                icon = R.drawable.ic_machines,
                title = "No machines yet",
                body = "Add a server or desktop that runs ngterm, and drive its sessions from here over ssh.",
                action = PanelAction("Add machine", R.drawable.ic_add) {},
            )
        }
    }
}
