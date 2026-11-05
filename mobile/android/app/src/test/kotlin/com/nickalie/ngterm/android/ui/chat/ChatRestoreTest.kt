package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The chat's saved state brought back after the app was put away, into what the chat shows by then: a question's
 * draft only into that same question, open rows only into their own conversation.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class ChatRestoreTest {
    @get:Rule
    val compose = createComposeRule()

    private val answers = mutableListOf<List<QuestionAnswer>>()

    // what the screen draws; a plain field, so changing it redraws nothing until the restore composes afresh
    private var shown = ChatSamples.state(emptyList())

    private fun show(first: ChatUiState): StateRestorationTester {
        shown = first
        val actions =
            ChatActions(
                onLoadOlder = {},
                onAnswerApproval = { _, _ -> },
                onAnswerQuestion = { _, answer -> answers += answer },
                onProblemShown = {},
                onShowTerminal = {},
            )
        val tester = StateRestorationTester(compose)
        tester.setContent { NgtermTheme { ChatScreen(shown, "build-01", actions) } }
        compose.waitForIdle()
        return tester
    }

    private fun asking(card: OpenQuestion) =
        ChatSamples
            .state(listOf(ChatSamples.prompt, event(card.id, ChatBody.Question(card.question))))
            .copy(question = card)

    // the shells question answered with fish and a typed tcsh
    private fun draftShells(): StateRestorationTester {
        val tester = show(asking(OpenQuestion("q1", ChatSamples.shells)))
        compose.onNode(hasText("fish") and toggleable).performClick()
        compose.onNodeWithText("Type an answer").performTextInput("tcsh")
        compose.onNodeWithText("Send answers").assertIsEnabled()
        return tester
    }

    private val toggleable = SemanticsMatcher.keyIsDefined(SemanticsProperties.ToggleableState)

    // a choice of the card: a checkbox row (several picks) or a radio row (one)
    private val choice = toggleable or SemanticsMatcher.keyIsDefined(SemanticsProperties.Selected)

    private fun assertNothingPicked() {
        val picks =
            compose
                .onAllNodes(choice)
                .fetchSemanticsNodes()
                .map { node ->
                    node.config.getOrElseNullable(SemanticsProperties.ToggleableState) { null } == ToggleableState.On ||
                        node.config.getOrElseNullable(SemanticsProperties.Selected) { null } == true
                }
        assertTrue("the card's choices picked: $picks", picks.isNotEmpty() && picks.none { it })
        compose.onNodeWithText("tcsh").assertDoesNotExist()
        compose.onNodeWithText("Send answers").assertIsNotEnabled()
    }

    @Test
    fun aDraftComesBackIntoItsOwnQuestion() {
        val tester = draftShells()
        tester.emulateSavedInstanceStateRestore()
        compose.onNode(hasText("fish") and toggleable).assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.ToggleableState,
                ToggleableState.On,
            ),
        )
        compose.onNodeWithText("tcsh").assertExists()
        compose.onNodeWithText("Send answers").assertIsEnabled().performClick()
        assertEquals(listOf(listOf(QuestionAnswer(listOf("fish"), "tcsh"))), answers)
    }

    @Test
    fun aDraftSavedForOneQuestionStartsNoOtherQuestion() {
        val tester = draftShells()
        // answered elsewhere while the app was away: another question, with two parts, waits now
        shown = asking(OpenQuestion("q2", twoParts))
        tester.emulateSavedInstanceStateRestore()
        compose.onNode(hasText("emacs") and choice).assertExists()
        assertNothingPicked()
        compose.onNodeWithText("vim").performClick()
        compose.onNodeWithText("tabs").performClick()
        compose.onNodeWithText("Send answers").assertIsEnabled().performClick()
        assertEquals(listOf(listOf(QuestionAnswer(listOf("vim"), null), QuestionAnswer(listOf("tabs"), null))), answers)
    }

    @Test
    fun aSavedDraftThatDoesNotFitItsQuestionIsLetGo() {
        val tester = draftShells()
        // the same id with another shape: the draft's own key, but not its question
        shown = asking(OpenQuestion("q1", twoParts))
        tester.emulateSavedInstanceStateRestore()
        compose.onNode(hasText("emacs") and choice).assertExists()
        assertNothingPicked()
    }

    @Test
    fun openRowsComeBackOnlyIntoTheirConversation() {
        val rows = listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.grep)
        val tester = show(ChatSamples.state(rows))
        compose.onNodeWithText("grep -n PATH scripts/install.sh").performClick()
        val expanded = SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Expanded")
        val collapsed = SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Collapsed")
        val grep =
            hasText("grep -n PATH scripts/install.sh") and
                SemanticsMatcher.keyIsDefined(
                    SemanticsProperties.StateDescription,
                )
        compose.onNode(grep).assert(expanded)
        tester.emulateSavedInstanceStateRestore()
        compose.onNode(grep).assert(expanded)

        // another conversation in the pane now, whose rows' keys are the same transcript offsets
        shown = shown.copy(conversation = ChatSamples.conversation.copy(conversationId = "c2"))
        tester.emulateSavedInstanceStateRestore()
        compose.onNode(grep).assert(collapsed)
    }

    private companion object {
        val twoParts =
            ChatQuestion(
                listOf(
                    ChatQuestionItem(
                        "Which editor?",
                        "Editor",
                        false,
                        listOf(ChatQuestionOption("vim", null), ChatQuestionOption("emacs", null)),
                    ),
                    ChatQuestionItem(
                        "Tabs or spaces?",
                        "Indent",
                        false,
                        listOf(ChatQuestionOption("tabs", null), ChatQuestionOption("spaces", null)),
                    ),
                ),
                ChatToolStatus.RUNNING,
                null,
            )
    }
}
