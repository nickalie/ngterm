package com.nickalie.ngterm.android.ui

import androidx.activity.ComponentActivity
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The frame's behavior: picking destinations, back, the rail's labels, and state kept across layouts. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp")
class AdaptiveScaffoldTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        test.seedKey()
    }

    @After
    fun tearDown() = test.close()

    // the app with its key made, past the first run
    private fun app(layout: NavLayout) {
        compose.setContent { NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout) } }
        compose.awaitText("Machines")
    }

    private fun back() = compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

    @Test
    fun aDestinationIsPickedByTapAndBackReturnsToTheFirst() {
        app(NavLayout.Bar)
        compose.awaitText("No machines yet")
        compose.onNodeWithText("Settings").performClick()
        compose.onNodeWithText("Settings").assertIsSelected()
        compose.onNodeWithText("core 1.2.3").assertExists()
        back()
        compose.onNodeWithText("Machines").assertIsSelected()
        compose.awaitText("No machines yet")
    }

    @Test
    fun anEmptyStatesActionLeadsOn() {
        app(NavLayout.Bar)
        compose.onNodeWithText("Sessions").performClick()
        compose.onNodeWithText("Open machines").performClick()
        compose.onNodeWithText("Machines").assertIsSelected()
    }

    @Test
    fun backClosesTheRailsLabelsBeforeLeavingTheDestination() {
        app(NavLayout.Rail)
        compose.onNodeWithText("Settings").performClick()
        compose.onNodeWithContentDescription("Show navigation labels").performClick()
        compose.onNodeWithContentDescription("Hide navigation labels").assertExists()
        back()
        compose.onNodeWithContentDescription("Show navigation labels").assertExists()
        compose.onNodeWithText("Settings").assertIsSelected()
        back()
        compose.onNodeWithText("Machines").assertIsSelected()
    }

    @Test
    fun theContentKeepsItsStateWhenTheLayoutChanges() {
        var layout by mutableStateOf(NavLayout.Bar)
        compose.setContent {
            NgtermTheme {
                AdaptiveScaffold(AppDestinations, AppDestinations.first().key, onSelect = {}, layout = layout) {
                    var taps by remember { mutableIntStateOf(0) }
                    TextButton(onClick = { taps++ }) { Text("tapped $taps") }
                }
            }
        }
        compose.onNodeWithText("tapped 0").performClick()
        compose.onNodeWithText("tapped 1").assertExists()
        for (next in listOf(NavLayout.Rail, NavLayout.Sidebar, NavLayout.Bar)) {
            layout = next
            compose.waitForIdle()
            compose.onNodeWithText("tapped 1").assertExists()
        }
        assertEquals(NavLayout.Bar, layout)
    }

    @Test
    fun theContentKeepsItsStateWhenItsLambdaChanges() {
        var label by mutableStateOf("first")
        compose.setContent {
            NgtermTheme {
                AdaptiveScaffold(
                    AppDestinations,
                    AppDestinations.first().key,
                    onSelect = {},
                    layout = NavLayout.Bar,
                    content = counter(label),
                )
            }
        }
        compose.onNodeWithText("first tapped 0").performClick()
        label = "second"
        compose.onNodeWithText("second tapped 1").assertExists()
    }
}

// a new lambda on every call, as a caller's content is when it is built outside the composition
private fun counter(label: String): @Composable () -> Unit =
    {
        var taps by remember { mutableIntStateOf(0) }
        TextButton(onClick = { taps++ }) { Text("$label tapped $taps") }
    }
