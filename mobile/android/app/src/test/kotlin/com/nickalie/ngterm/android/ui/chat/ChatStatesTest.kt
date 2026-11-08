package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The chat keeps its machine and older-page status readable, with retry only after a failure. */
@RunWith(AndroidJUnit4::class)
class ChatStatesTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun aDroppedChatAnnouncesItsMachineAndKeepsTheConversationReadable() {
        compose.setContent {
            NgtermTheme { ReconnectingStrip("build-01", Modifier) }
        }
        compose.onNodeWithText("Reconnecting to build-01", substring = true).assertIsDisplayed().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite),
        )
        compose.onNodeWithText("Reconnecting to build-01", substring = true).assert(
            hasText("What's here stays; new messages follow once it's back."),
        )
    }

    @Test
    fun earlierPagesShowProgressThenTheReasonAndRetryThenTheConversationStart() {
        val older = mutableStateOf<OlderPages>(OlderPages.More)
        val cwd = mutableStateOf<String?>(null)
        var retries = 0
        compose.setContent {
            NgtermTheme { OlderHeader(older.value, cwd.value, { retries++ }, Modifier) }
        }
        compose.onNodeWithText("Loading earlier messages").assertIsDisplayed()
        older.value = OlderPages.Loading
        compose.onNodeWithText("Loading earlier messages").assertIsDisplayed()
        compose.onNodeWithText("Retry").assertDoesNotExist()
        older.value = OlderPages.Failed("SSH connection closed")
        compose.onNodeWithText("SSH connection closed", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Retry").performClick()
        assertEquals(1, retries)
        older.value = OlderPages.AtStart
        compose.onNodeWithText("Start of the conversation").assertIsDisplayed()
        compose.onNodeWithText("Retry").assertDoesNotExist()
        cwd.value = "/home/nick/project"
        compose.onNodeWithText("/home/nick/project").assertIsDisplayed()
    }

    @Test
    fun theFirstPageNamesTheMachineAndAnnouncesItsProgress() {
        compose.setContent { NgtermTheme { ChatLoading("studio", Modifier) } }
        compose.onNodeWithText("studio", substring = true).assertIsDisplayed().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite),
        )
        compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo)).assertExists()
    }
}
