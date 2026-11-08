package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** New session keeps the name through a failure or recreation and prevents duplicate creates while busy. */
@RunWith(AndroidJUnit4::class)
class NewSessionDialogTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun creatingHoldsTheNameAndCancelStillClosesTheDialog() {
        val state = mutableStateOf(NewSessionState())
        val names = mutableListOf<String>()
        var dismisses = 0
        compose.setContent {
            NgtermTheme {
                NewSessionDialog("build-01", state.value, { names += it }, { dismisses++ })
            }
        }
        compose.onNodeWithText("Create session").performClick()
        assertEquals(listOf(""), names)
        compose.onNode(hasSetTextAction()).performTextReplacement("install guide")
        compose.onNode(hasSetTextAction()).performImeAction()
        assertEquals(listOf("", "install guide"), names)
        state.value = NewSessionState(running = true)
        compose.onNodeWithText("Creating").assertIsDisplayed().assertIsNotEnabled()
        compose.onNodeWithText("install guide").assertIsNotEnabled()
        compose.onNodeWithText("Cancel").assertIsEnabled().performClick()
        assertEquals(1, dismisses)
        assertEquals(listOf("", "install guide"), names)
        state.value = NewSessionState(failed = "The machine refused the session")
        compose.onNodeWithText("The machine refused the session").assertIsDisplayed()
        compose.onNodeWithText("install guide").assertIsDisplayed()
        compose.onNodeWithText("Create session").assertIsEnabled().performClick()
        assertEquals(listOf("", "install guide", "install guide"), names)
    }

    @Test
    fun recreatingTheDialogKeepsTheNameForTheNextCreate() {
        val restoration = StateRestorationTester(compose)
        val names = mutableListOf<String>()
        restoration.setContent {
            NgtermTheme { NewSessionDialog("studio", NewSessionState(), { names += it }, {}) }
        }
        compose.onNode(hasSetTextAction()).performTextReplacement("review")
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("review").assertIsDisplayed()
        compose.onNode(hasSetTextAction()).performImeAction()
        assertEquals(listOf("review"), names)
    }
}
