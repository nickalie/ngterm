package com.nickalie.ngterm.android.ui.updates

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.espresso.Espresso
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.ui.settings.SettingsRoute
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.update.UpdateOffer
import com.nickalie.ngterm.android.update.UpdateState
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Settings' Updates section and the update sheet over the app's updates. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp-mdpi")
class UpdateScreensTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    private val updates = FakeUpdates()
    private var test: TestGraph? = null

    @After fun tearDown() {
        test?.close()
    }

    private fun settings(updates: FakeUpdates?) {
        val graph = TestGraph(dir.root, updates = updates).also { test = it }
        compose.setContent { NgtermTheme { SettingsRoute(graph.graph, "updates-test") } }
        compose.awaitText("core updates-test")
    }

    @Test
    fun settingsShowsWhereUpdatingStandsAndItsSwitch() {
        settings(updates)
        compose.onNodeWithText("Not checked since ngterm started.").performScrollTo()
        compose.onNodeWithText("Check now").performScrollTo().performClick()
        val switch = compose.onNode(isToggleable() and hasText("Check automatically", substring = true))
        switch.performScrollTo().assertIsOn().performClick()
        compose.waitForIdle()
        switch.assertIsOff()
        updates.state.value = UpdateState.Available(UpdateOffer("0.3.0", 1_000, "https://github.com/x"))
        compose.onNodeWithText("ngterm 0.3.0 is available").performScrollTo()
        assertEquals(listOf("check", "automatic:false"), updates.calls)
    }

    @Test
    fun googlePlaysBuildHasNoUpdatesSection() {
        settings(null)
        compose.onNodeWithText("Updates").assertDoesNotExist()
        compose.onNodeWithText("Check automatically").assertDoesNotExist()
    }

    @Test
    fun theSheetIsUpWhileAskedForAndClosingItSaysSo() {
        compose.setContent {
            NgtermTheme {
                UpdatePrompt(updates)
                UpdatePrompt(null)
            }
        }
        compose.onNodeWithText("Version 0.2.3").assertDoesNotExist()
        updates.prompt.value = true
        compose.awaitText("Version 0.2.3")
        compose.onNodeWithText("Check now").performClick()
        Espresso.pressBack()
        compose.waitForIdle()
        assertEquals(listOf("check", "promptClosed"), updates.calls)
    }
}
