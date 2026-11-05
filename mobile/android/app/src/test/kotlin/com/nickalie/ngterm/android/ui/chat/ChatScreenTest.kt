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
import androidx.compose.ui.test.hasScrollToIndexAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isHeading
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.ChatUsage
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

    // the cards the taps answered, by id
    private val answered = mutableListOf<String>()

    // the asks for an earlier page the view model acts on: with rows shown (an empty log has nothing before it)
    private var olderAsked = 0
    private var terminalShown = 0
    private var problemsShown = 0
    private var endsShown = 0

    private fun show() {
        val actions =
            ChatActions(
                onLoadOlder = { if (!state.value.log.isEmpty) olderAsked++ },
                onAnswerApproval = { id, label ->
                    answered += id
                    approvals += label
                },
                onAnswerQuestion = { id, answer ->
                    answered += id
                    answers += answer
                },
                onProblemShown = { problemsShown++ },
                onShowTerminal = { terminalShown++ },
                onEndShown = {
                    endsShown++
                    state.value = state.value.copy(toEnd = false)
                },
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
    fun theAccountLineReadsAsOneSentence() {
        state.value = state.value.copy(account = ChatBody.Account("dev@example.com", ChatUsage(97u, null)))
        show()
        compose
            .onNodeWithContentDescription("Signed in as dev@example.com. 97% of the weekly limit used.")
            .assertIsDisplayed()
        // the email alone when the origin read no limit
        state.value = state.value.copy(account = ChatBody.Account("dev@example.com", null))
        compose.onNodeWithContentDescription("Signed in as dev@example.com.").assertIsDisplayed()
    }

    @Test
    fun theFootSaysWhatStillRunsAfterTheTurnAndATapListsIt() {
        val agent = event("task", tool("Agent", "Review the parser", kind = ChatToolKind.TASK))
        val build = event("t9", tool("Bash", "npm run build"))
        state.value = ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening, agent, build))
        show()
        val strip = hasText("Still running in the background")
        compose
            .onNode(strip)
            .assert(hasText("1 subagent, 1 command"))
            .assert(expectation(SemanticsProperties.StateDescription, "Collapsed"))
            .assert(clickLabel("Show what's running"))
            .performClick()
        compose.onNode(strip).assert(clickLabel("Hide what's running"))
        assertEquals(2, compose.onAllNodesWithText("npm run build").fetchSemanticsNodes().size)
        // while the agent works on a turn, it says so, and a card waiting on the user takes the foot alone
        state.value = state.value.copy(working = true)
        compose.onNodeWithText("Claude Code is working").assertIsDisplayed()
        state.value = state.value.copy(approval = ChatSamples.approval)
        compose.onNodeWithText("Claude Code is working").assertDoesNotExist()
    }

    @Test
    fun aFinishedTurnWithNothingRunningLeavesTheFootEmpty() {
        state.value = ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.grep))
        show()
        compose.onNodeWithText("Still running", substring = true).assertDoesNotExist()
        compose.onNodeWithText("is working", substring = true).assertDoesNotExist()
    }

    @Test
    fun anApprovalCardIsAHeadingWithTheDialogsOptions() {
        state.value = state.value.copy(approval = ChatSamples.approval)
        show()
        compose.onNode(hasText("Allow Bash?") and isHeading()).assertIsDisplayed()
        compose.onNodeWithText("Yes, and don't ask again", substring = true).performClick()
        assertEquals(listOf("Yes, and don't ask again for rm commands in this project"), approvals)
        // the tap names the card it was drawn on
        assertEquals(listOf(ChatSamples.approval.id), answered)
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

    // whether a node with [text] is on screen now (a row out of sight isn't composed, or isn't placed)
    // a prompt bubble is read as one line ("You: …"), its text no node of its own
    private fun prompted(text: String) =
        try {
            compose.onNodeWithContentDescription("You: $text").assertIsDisplayed()
            true
        } catch (_: AssertionError) {
            false
        }

    private fun onScreen(text: String) =
        try {
            compose.onNodeWithText(text).assertIsDisplayed()
            true
        } catch (_: AssertionError) {
            false
        }

    private val table = "| Shell | Check |\n|---|---|\n| bash | which |\n| fish | command -v |"

    @Test
    fun aMarkdownTableDrawsInAReplyAndInASubagentsOpenedSteps() {
        val task =
            ChatBody.Tool(
                ChatTool("Task", ChatToolKind.TASK, "Compare the shells", ChatToolStatus.OK, null, null, false),
            )
        state.value =
            ChatSamples.state(
                listOf(
                    ChatSamples.prompt,
                    event("a1", reply("Where each check works:\n\n$table")),
                    event("task", task),
                    event("s1", reply("The subagent found:\n\n$table"), parent = "task"),
                ),
            )
        show()
        compose.onNodeWithText("Where each check works:").assertIsDisplayed()
        // opening the steps measures the subagent's reply, table and all, under the rule
        compose.onNode(hasText("Compare the shells") and hasContentDescription("Subagent")).performClick()
        compose.waitForIdle()
        compose.onNodeWithText("The subagent found:").assertIsDisplayed()
        val cells = compose.onAllNodesWithText("command -v", substring = true, useUnmergedTree = true)
        assertEquals(2, cells.fetchSemanticsNodes().size)
    }

    @Test
    fun aPromptsImagesAreReadWithIt() {
        state.value = ChatSamples.state(listOf(event("u1", ChatBody.User("What is wrong here?", 2u))))
        show()
        compose.onNodeWithContentDescription("You: What is wrong here?. With 2 images").assertExists()
    }

    @Test
    fun aFingerScrollingBackUpAsksForTheEarlierPageOnceTheTopIsInSight() {
        // a tall reply under the top: scrolling back up, the list composes the top ahead while the reply fills the
        // screen, and only shows it later
        val long = (0 until 80).joinToString("\n\n") { "Paragraph $it of a long reply about the installer." }
        val rows = listOf(event("a0", reply(long))) + (1 until 20).map { event("p$it", user("prompt $it")) }
        state.value = ChatSamples.state(rows, older = OlderPages.More)
        show()
        assertEquals(0, olderAsked)
        var swipes = 0
        while (!onScreen("Loading earlier messages") && swipes < MAX_SWIPES) {
            compose.onNode(hasScrollAction()).performTouchInput { swipeDown(durationMillis = 600) }
            compose.waitForIdle()
            swipes++
        }
        assertTrue("the top never came into sight", onScreen("Loading earlier messages"))
        compose.waitForIdle()
        assertEquals(1, olderAsked)
    }

    @Test
    fun aNewConversationAsksForItsEarlierPageOnceItsFirstRowIsIn() {
        state.value = ChatSamples.state(listOf(ChatSamples.prompt), older = OlderPages.More)
        show()
        assertEquals(1, olderAsked)
        // the page came back empty: the start is shown, and nothing more is asked for
        state.value = state.value.copy(older = OlderPages.AtStart)
        compose.waitForIdle()
        compose.onNodeWithText("Start of the conversation").assertIsDisplayed()
        assertEquals(1, olderAsked)

        // /clear: the reset drops the rows, and the new conversation has none yet
        state.value = ChatUiState(log = state.value.log.reset(), phase = ChatPhase.Live)
        compose.waitForIdle()
        compose.onNodeWithText("New conversation").assertIsDisplayed()
        assertEquals(1, olderAsked)
        // its first prompt is in: the header, still on screen, asks for what came before it
        state.value = state.value.copy(log = state.value.log.append(listOf(event("n1", user("Start over")))))
        compose.waitForIdle()
        assertEquals(2, olderAsked)
    }

    @Test
    fun aFailedPageOffersRetryAndAnEmptyOneEndsAtTheStart() {
        state.value = ChatSamples.state(listOf(ChatSamples.prompt), older = OlderPages.More)
        show()
        assertEquals(1, olderAsked)
        state.value = state.value.copy(older = OlderPages.Loading)
        compose.waitForIdle()
        state.value = state.value.copy(older = OlderPages.Failed("the link went down"))
        compose.waitForIdle()
        compose.onNodeWithText("the link went down", substring = true).assertIsDisplayed()
        // a failure isn't asked again by itself
        assertEquals(1, olderAsked)
        compose.onNodeWithText("Retry").performClick()
        assertEquals(2, olderAsked)
        state.value = state.value.copy(older = OlderPages.Loading)
        compose.waitForIdle()
        state.value = state.value.copy(older = OlderPages.AtStart)
        compose.waitForIdle()
        compose.onNodeWithText("Start of the conversation").assertIsDisplayed()
        assertEquals(2, olderAsked)
    }

    @Test
    fun aFlingAwayFromTheEndIsNotPulledBackByANewRow() {
        val rows = (0 until 30).map { event("p$it", user("prompt $it")) }
        state.value = ChatSamples.state(rows)
        show()
        compose.mainClock.autoAdvance = false
        compose.onNode(hasScrollAction()).performTouchInput { swipeDown(durationMillis = 120) }
        compose.mainClock.advanceTimeByFrame()
        // a row comes while the list still flings toward the top
        state.value = state.value.copy(log = state.value.log.append(listOf(event("new", reply("The newest reply")))))
        compose.mainClock.advanceTimeBy(FLING_MS)
        compose.mainClock.autoAdvance = true
        compose.waitForIdle()
        assertTrue("the fling was pulled back to the end", !onScreen("The newest reply"))
    }

    @Test
    fun aNotificationsTapBringsAListScrolledAwayBackToItsEndToFollowAgain() {
        val rows = (0 until 30).map { event("p$it", user("prompt $it")) }
        state.value = ChatSamples.state(rows).copy(approval = ChatSamples.approval)
        show()
        compose.onNode(hasScrollToIndexAction()).performTouchInput { swipeDown(durationMillis = 600) }
        compose.waitForIdle()
        assertTrue("the swipe left the end in sight", !prompted("prompt 29"))

        state.value = state.value.copy(toEnd = true)
        compose.waitForIdle()
        assertEquals(1, endsShown)
        assertTrue("the list did not go to its end", prompted("prompt 29"))
        compose.onNode(hasText("Allow Bash?") and isHeading()).assertIsDisplayed()
        // it follows again: a new row keeps it at the end
        state.value = state.value.copy(log = state.value.log.append(listOf(event("new", reply("The newest reply")))))
        compose.waitForIdle()
        assertTrue("the list stopped following", onScreen("The newest reply"))
        assertEquals(1, endsShown)
    }

    private companion object {
        const val MAX_SWIPES = 60
        const val FLING_MS = 2_000L
    }
}
