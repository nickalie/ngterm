package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The composer as a finger, a hardware keyboard and TalkBack reach it: the `/` menu opening as `/` is typed, its rows
 * read whole and inserting with the field keeping the focus, its keys, and Send and Stop with what holds them.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
@OptIn(ExperimentalTestApi::class)
class ComposerScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val state = mutableStateOf(ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening)))
    private var sent = 0
    private var stopped = 0

    private fun show(composer: ChatComposer = ChatSamples.composer("")): ChatComposer {
        val actions = ChatActions(onSend = { sent++ }, onStop = { stopped++ })
        compose.setContent { NgtermTheme { ChatScreen(state.value, "build-01", actions, composer = composer) } }
        compose.waitForIdle()
        return composer
    }

    private fun field() = compose.onNode(hasSetTextAction())

    private fun clickLabel(label: String) =
        SemanticsMatcher("click label $label") {
            it.config.getOrElseNullable(SemanticsActions.OnClick) { null }?.label == label
        }

    @Test
    fun typingASlashOpensTheMenuAndARowGoesInWithTheFieldKeepingTheFocus() {
        val composer = show()
        field().performClick()
        field().performTextInput("/")
        compose.onNodeWithText("8 commands").assertIsDisplayed()
        field().performTextInput("co")
        compose.onNodeWithText("4 commands").assertIsDisplayed()
        // a row reads whole (the command, what it does, where it comes from) and names its tap
        compose
            .onNode(hasText("/context") and hasText("Show how the context window is used") and hasText("Built-in"))
            .assert(clickLabel("Insert"))
            .performClick()
        assertEquals("/context ", composer.draft.text)
        field().assertIsFocused()
        compose.onNodeWithText("4 commands").assertDoesNotExist()
    }

    @Test
    fun aHardwareKeyboardMovesPicksAndClosesTheMenu() {
        val composer = show()
        field().performClick()
        field().performTextInput("/co")
        // the first row is highlighted; Down moves it, Enter picks it
        compose.onNode(hasText("/compact")).assertIsSelected()
        field().performKeyInput { pressKey(Key.DirectionDown) }
        compose.onNode(hasText("/context")).assertIsSelected()
        field().performKeyInput { pressKey(Key.Enter) }
        assertEquals("/context ", composer.draft.text)
        // Escape closes the menu without touching the text, until the text changes; Tab picks too
        field().performTextReplacement("/clear")
        compose.onNodeWithText("1 command").assertIsDisplayed()
        field().performKeyInput { pressKey(Key.Escape) }
        compose.onNodeWithText("1 command").assertDoesNotExist()
        assertEquals("/clear", composer.draft.text)
        field().performTextReplacement("/clea")
        compose.onNodeWithText("2 commands").assertIsDisplayed()
        field().performKeyInput { pressKey(Key.Tab) }
        assertEquals("/clear ", composer.draft.text)
        // Ctrl+Enter sends
        field().performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.Enter) } }
        assertEquals(1, sent)
    }

    @Test
    fun theMenuWaitsQuietlyForTheCatalog() {
        show(ChatSamples.composer("/", commands = null))
        compose.onNodeWithText("Loading Claude Code's commands").assertIsDisplayed()
        compose
            .onNodeWithText("Loading Claude Code's commands")
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.LiveRegion))
    }

    @Test
    fun sendAndStopSayWhatHoldsThem() {
        show(ChatSamples.composer(""))
        compose.onNodeWithContentDescription("Send").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Stop").assertDoesNotExist()
        field().performTextInput("Fix the guide")
        compose.onNodeWithContentDescription("Send").assertIsEnabled().performClick()
        assertEquals(1, sent)

        // the agent works on a turn: Stop shows
        state.value = state.value.copy(working = true)
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Stop").assertIsEnabled().performClick()
        assertEquals(1, stopped)

        // an action on its way: Send says so, and nothing takes a second tap
        state.value = state.value.copy(busy = true)
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Sending").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Stop").assertIsNotEnabled()

        // a card waits: Send and Stop hold, and the line over the field says why
        state.value = state.value.copy(busy = false, approval = ChatSamples.approval)
        compose.waitForIdle()
        compose.onNodeWithText("Answer the card above first", substring = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Send").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Stop").assertDoesNotExist()
        assertEquals("Fix the guide", field().fetchSemanticsNode().config[SemanticsProperties.EditableText].text)
    }

    @Test
    fun anEndedChatHasNoComposer() {
        state.value = state.value.copy(phase = ChatPhase.Ended)
        show()
        field().assertDoesNotExist()
    }
}
