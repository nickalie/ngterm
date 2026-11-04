package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isHeading
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The chat screen as TalkBack and a finger reach it: what each row reads and offers, opening a tool's output, the
 * cards' answers, a problem's way to the terminal, and older pages asked for at the top and kept in place.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class ChatScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val state = mutableStateOf(ChatSamples.state(ChatSamples.long))
    private val approvals = mutableListOf<String>()
    private val answers = mutableListOf<List<QuestionAnswer>>()
    private var olderAsked = 0
    private var terminalShown = 0
    private var problemsShown = 0

    private fun show() {
        val actions =
            ChatActions(
                onLoadOlder = { olderAsked++ },
                onAnswerApproval = { approvals += it },
                onAnswerQuestion = { answers += it },
                onProblemShown = { problemsShown++ },
                onShowTerminal = { terminalShown++ },
            )
        compose.setContent { NgtermTheme { ChatScreen(state.value, "build-01", actions) } }
        compose.waitForIdle()
    }

    private fun expectation(
        property: androidx.compose.ui.semantics.SemanticsPropertyKey<String>,
        value: String,
    ) = SemanticsMatcher.expectValue(property, value)

    private fun clickLabel(label: String) =
        SemanticsMatcher("click label $label") {
            it.config.getOrElseNullable(SemanticsActions.OnClick) { null }?.label ==
                label
        }

    @Test
    fun theChatAndItsRowsSayWhatTheyAreToTalkBack() {
        state.value = ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.grep))
        show()
        compose.onNodeWithContentDescription("Chat with Claude Code").assertExists()
        compose
            .onNodeWithContentDescription("You: The install guide", substring = true)
            .assertIsDisplayed()
        // the tool's glyph has its kind's word, read with its title and status; a tap opens its output
        val grep = hasText("grep -n PATH scripts/install.sh") and hasContentDescription("Command")
        compose
            .onNode(grep)
            .assert(hasText("Done"))
            .assert(expectation(SemanticsProperties.StateDescription, "Collapsed"))
            .assert(clickLabel("Show output"))
            .performClick()
        compose
            .onNode(grep)
            .assert(expectation(SemanticsProperties.StateDescription, "Expanded"))
            .assert(clickLabel("Hide output"))
        compose.onNodeWithText("echo", substring = true).assertIsDisplayed()
    }

    @Test
    fun anApprovalCardIsAHeadingWithTheDialogsOptions() {
        state.value = state.value.copy(approval = ChatSamples.approval)
        show()
        compose.onNode(hasText("Allow Bash?") and isHeading()).assertIsDisplayed()
        compose.onNodeWithText("Yes, and don't ask again", substring = true).performClick()
        assertEquals(listOf("Yes, and don't ask again for rm commands in this project"), approvals)
    }

    @Test
    fun anApprovalCardStandsInTheAgentsAnswersAndHoldsStillWhileOneIsSent() {
        state.value =
            state.value.copy(
                conversation = ChatSamples.conversation.copy(agent = "codex"),
                approval = ChatSamples.approval.copy(labels = emptyList()),
            )
        show()
        compose.onNodeWithText("Yes, proceed").performClick()
        assertEquals(listOf("Yes, proceed"), approvals)
        state.value = state.value.copy(busy = true)
        compose.waitForIdle()
        compose.onNodeWithText("Yes, proceed").assertIsNotEnabled()
        compose.onNodeWithText("Sending").assertIsDisplayed()
    }

    @Test
    fun aQuestionCardSendsOnceEveryQuestionIsAnswered() {
        val question = event("q1", ChatBody.Question(ChatSamples.shells))
        state.value =
            ChatSamples
                .state(
                    listOf(ChatSamples.prompt, question),
                ).copy(question = OpenQuestion("q1", ChatSamples.shells))
        show()
        compose.onNode(hasText("Claude Code asks") and isHeading()).assertIsDisplayed()
        compose.onNodeWithText("Send answers").assertIsNotEnabled()
        compose
            .onNode(hasText("fish") and SemanticsMatcher.keyIsDefined(SemanticsProperties.ToggleableState))
            .performClick()
        compose.onNodeWithText("bash").performClick()
        compose.onNodeWithText("Send answers").assertIsEnabled().performClick()
        assertEquals(listOf(listOf(QuestionAnswer(listOf("fish", "bash"), null))), answers)
    }

    @Test
    fun aProblemOffersTheTerminalWhenThatIsTheWayOn() {
        state.value = state.value.copy(problem = ChatProblem.NotOnScreen)
        show()
        compose.onNodeWithText("Answer in terminal").performClick()
        assertEquals(1, terminalShown)
        compose.onNodeWithContentDescription("Dismiss").performClick()
        assertEquals(1, problemsShown)

        state.value = state.value.copy(problem = ChatProblem.LeadRefused)
        compose.waitForIdle()
        compose.onNodeWithText("Another device kept control", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Answer in terminal").assertDoesNotExist()
    }

    @Test
    fun theTopAsksForTheEarlierPageOnceAndThePageKeepsTheRowThatWasOnTop() {
        val later = (10 until 40).map { event("p$it", user("prompt $it")) }
        state.value = ChatSamples.state(later, older = OlderPages.More)
        show()
        // the list opens at the newest row: the top is out of sight, and nothing is asked for
        compose.onNodeWithContentDescription("You: prompt 39").assertIsDisplayed()
        assertEquals(0, olderAsked)

        compose.onNode(hasScrollAction()).performScrollToIndex(0)
        compose.waitForIdle()
        assertEquals(1, olderAsked)

        state.value = state.value.copy(older = OlderPages.Loading)
        compose.waitForIdle()
        val earlier = (0 until 10).map { event("p$it", user("prompt $it")) }
        state.value =
            state.value.copy(log = state.value.log.prepend(earlier), older = OlderPages.More, anchor = "e:p10")
        compose.waitForIdle()
        // the row that was on top is on top again, the page above it, out of sight: nothing more is asked for
        val top =
            compose
                .onNodeWithContentDescription("You: prompt 10")
                .assertIsDisplayed()
                .getBoundsInRoot()
                .top
        assertTrue("prompt 10 is at $top", top < 64.dp)
        assertEquals(1, olderAsked)
    }

    @Test
    fun aNewRowAtTheEndIsFollowedWhileTheListIsThere() {
        val rows = (0 until 30).map { event("p$it", user("prompt $it")) }
        state.value = ChatSamples.state(rows)
        show()
        state.value = state.value.copy(log = state.value.log.append(listOf(event("new", reply("The newest reply")))))
        compose.waitForIdle()
        compose.onNodeWithText("The newest reply").assertIsDisplayed()
    }

    @Test
    fun theEmptyAndEndedStatesOfferTheTerminal() {
        state.value = ChatUiState(phase = ChatPhase.Live, conversation = ChatSamples.conversation)
        show()
        compose.onNodeWithText("Nothing said yet").assertIsDisplayed()
        compose.onNodeWithText("Show terminal").performClick()
        state.value = state.value.copy(phase = ChatPhase.Ended)
        compose.waitForIdle()
        compose.onNodeWithText("This chat ended").assertIsDisplayed()
        compose.onNodeWithText("Show terminal").performClick()
        assertEquals(2, terminalShown)
    }
}
