package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The chat's text can be selected to copy: a long press on a row's text selects it, its handles up (the system's
 * menu with Copy goes with them). Each test starts with nothing selected, so no handle is up before the press.
 */
@RunWith(AndroidJUnit4::class)
// API 27: the platform magnifier a press shows from 28 on has no surface under Robolectric
@Config(qualifiers = "w411dp-h891dp-xhdpi", sdk = [27])
class ChatSelectionTest {
    @get:Rule
    val compose = createComposeRule()

    // [row] found, pressed long at [x] of its width (on its text: a short title ends before the row's middle)
    private fun selects(
        row: SemanticsMatcher,
        state: ChatUiState,
        x: Float = 0.5f,
    ) {
        compose.setContent { NgtermTheme { ChatScreen(state, "build-01", ChatActions()) } }
        compose.waitForIdle()
        assertTrue(compose.onAllNodes(isPopup()).fetchSemanticsNodes().isEmpty())
        compose.onNode(row).performTouchInput { longClick(percentOffset(x, 0.5f)) }
        compose.waitForIdle()
        assertTrue("nothing selected in $row", compose.onAllNodes(isPopup()).fetchSemanticsNodes().isNotEmpty())
    }

    @Test
    fun aLongPressSelectsAReply() =
        selects(
            hasText("installer's PATH check", substring = true),
            ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening)),
        )

    @Test
    fun aLongPressSelectsAPrompt() =
        selects(
            hasContentDescription("You: The install guide", substring = true),
            ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening)),
        )

    @Test
    fun aLongPressSelectsAToolsTitle() =
        selects(
            hasText("grep -n PATH", substring = true),
            ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.grep)),
            x = 0.15f,
        )

    @Test
    fun aLongPressSelectsAToolsOutput() {
        compose.setContent {
            NgtermTheme {
                ChatScreen(ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.grep)), "build-01", ChatActions())
            }
        }
        compose.onNode(hasText("grep -n PATH", substring = true)).performClick()
        compose.waitForIdle()
        compose.onNode(hasText("echo", substring = true), useUnmergedTree = true).performTouchInput {
            longClick(percentOffset(0.2f, 0.5f))
        }
        compose.waitForIdle()
        assertTrue("nothing selected", compose.onAllNodes(isPopup()).fetchSemanticsNodes().isNotEmpty())
    }
}
