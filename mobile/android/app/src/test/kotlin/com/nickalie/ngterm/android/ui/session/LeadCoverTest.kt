package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.components.underCover
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Each pane cover explains its state and offers the action that can recover from it. */
@RunWith(AndroidJUnit4::class)
class LeadCoverTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun followerAndFailedCoversOfferTheirOwnRecoveryActions() {
        val cover = mutableStateOf<PaneCover>(PaneCover.Follower)
        var leads = 0
        var retries = 0
        var detaches = 0
        compose.setContent {
            NgtermTheme { LeadCover(cover.value, "studio", { leads++ }, { detaches++ }, { retries++ }, Modifier) }
        }
        compose.onNodeWithText("Take control").performClick()
        assertEquals(1, leads)
        cover.value = PaneCover.Failed("The pane no longer exists")
        compose.onNodeWithText("The pane no longer exists").assertIsDisplayed()
        compose.onNodeWithText("Take control").assertDoesNotExist()
        compose.onNodeWithText("Retry").performClick()
        assertEquals(1, retries)
        cover.value = PaneCover.Attaching
        compose.onNodeWithText("studio", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Retry").assertDoesNotExist()
        assertEquals(0, detaches)
    }

    @Test
    fun anExitedProgramShowsItsSignalCodeOrUnknownReasonAndCanBeDetached() {
        val cover = mutableStateOf<PaneCover>(PaneCover.Exited(7u, "TERM"))
        var detaches = 0
        compose.setContent {
            NgtermTheme { LeadCover(cover.value, "build-01", {}, { detaches++ }, {}) }
        }
        compose.onNodeWithText("TERM", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Detach").performClick()
        cover.value = PaneCover.Exited(7u, null)
        compose.onNodeWithText("7", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Detach").performClick()
        cover.value = PaneCover.Exited(null, null)
        compose.onNodeWithText("The last screen stays as it was.").assertIsDisplayed()
        compose.onNodeWithText("Detach").performClick()
        assertEquals(3, detaches)
    }

    @Test
    fun reconnectingKeepsTheLastScreenReadableButStopsTouchesFromTypingIntoIt() {
        var taps = 0
        val cover = mutableStateOf<PaneCover>(PaneCover.Reconnecting)
        compose.setContent {
            NgtermTheme {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.fillMaxSize().underCover(cover.value.veils).clickable { taps++ }) {
                        Text("last frame")
                    }
                    LeadCover(cover.value, "studio", {}, {}, {}, Modifier.testTag("cover"))
                }
            }
        }
        compose.onNodeWithText("Reconnecting to studio").assertIsDisplayed()
        compose.onNodeWithText("last frame").assertIsDisplayed()
        compose.onNodeWithTag("cover").performTouchInput { click(bottomRight) }
        assertEquals(0, taps)
        cover.value = PaneCover.Follower
        compose.onNodeWithText("last frame").assertDoesNotExist()
        compose.onNodeWithText("Take control").assertIsDisplayed()
    }
}
