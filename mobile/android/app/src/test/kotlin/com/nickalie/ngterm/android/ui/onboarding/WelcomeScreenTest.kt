package com.nickalie.ngterm.android.ui.onboarding

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class WelcomeScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun makingAKeyHoldsBothActionsAndFailureIsAnnouncedBeforeRetrying() {
        val state = mutableStateOf(OnboardingStep.Welcome())
        var creates = 0
        var imports = 0
        compose.setContent {
            NgtermTheme {
                WelcomeScreen(state.value.busy, state.value.failure, { creates++ }, { imports++ })
            }
        }
        compose.onNodeWithText("Set up a key").assertIsDisplayed()
        compose.onNodeWithText("Create a key").performClick()
        compose.onNodeWithText("Import a key").performClick()
        assertEquals(1, creates)
        assertEquals(1, imports)
        compose.runOnIdle { state.value = OnboardingStep.Welcome(busy = true) }
        compose.onNodeWithText("Creating a key").assertIsDisplayed().performClick()
        compose.onNodeWithText("Import a key").assertIsNotEnabled()
        assertEquals(1, creates)
        assertEquals(1, imports)
        compose.runOnIdle { state.value = OnboardingStep.Welcome(failure = "keystore unavailable") }
        compose.onNodeWithText("The key couldn't be created: keystore unavailable").assertIsDisplayed().assert(
            hasAnyAncestor(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite)),
        )
        compose.onNodeWithText("Create a key").performClick()
        assertEquals(2, creates)
    }
}
