package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.core.fake.FakeAction
import com.nickalie.ngterm.core.ffi.AgentInputOutcome
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatState
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.ChatUsage
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

/** A pane's chat: its rows, its pages, its cards and its actions ([ChatViewModel], [ChatLog]). */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class ChatViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var bed: ChatBed
    private lateinit var chat: ChatViewModel

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        bed = ChatBed(dir.root)
        bed.attach("p1")
        bed.show("p1")
        bed.agent("p1")
        eventually { bed.chat("p1") != null }
        chat = bed.chat("p1")!!
    }

    @After
    fun tearDown() {
        bed.close()
        Dispatchers.resetMain()
    }

    private val pane get() = bed.panes.getValue("p1")

    private fun emit(vararg events: ChatEvent) = bed.emit("p1", *events)

    private fun state() = chat.state.value

    // the composer's text, sent
    private fun send(text: String) {
        chat.composer.edit(TextFieldValue(text))
        chat.send()
    }

    // the rows as (id or placeholder key, children's ids)
    private fun rows(items: List<ChatItem> = state().items): List<String> =
        items.map { item ->
            when (item) {
                is ChatItem.Entry -> if (item.children.isEmpty()) item.id else "${item.id}${rows(item.children)}"
                is ChatItem.Subagent -> "?${item.parent}${rows(item.children)}"
                ChatItem.Divider -> "--"
            }
        }

    private fun entry(id: String): ChatItem.Entry =
        state().items.filterIsInstance<ChatItem.Entry>().single { it.id == id }

    @Test
    fun theBacklogLoadsThenGoesLiveAndReconnects() {
        assertEquals(ChatPhase.Loading, state().phase)
        val conversation = ChatBody.Conversation("claude", "c1", "/home/user/project", "sonnet")
        emit(event("conv", conversation), event("u1", user("hi")), live)
        assertEquals(ChatPhase.Live, state().phase)
        assertEquals(conversation, state().conversation)
        assertEquals(listOf("u1"), rows())
        bed.test.core.emit(MobileEvent.ChatState(bed.ref("p1"), ChatState.RECONNECTING))
        assertEquals(ChatPhase.Reconnecting, state().phase)
        emit(event("a1", reply("hello")), live)
        assertEquals(ChatPhase.Live, state().phase)
        assertEquals(listOf("u1", "a1"), rows())
    }

    @Test
    fun theAccountIsNoRowAndOutlivesAResetUntilOneThatKnowsNothingClearsIt() {
        val account = ChatBody.Account("dev@example.com", ChatUsage(72u, 9uL))
        emit(event("u1", user("hi")), event("account", account), live)
        assertEquals(listOf("u1"), rows())
        assertEquals(account, state().account)
        emit(event("reset", ChatBody.Reset))
        assertEquals(account, state().account)
        emit(event("account", ChatBody.Account(null, null)))
        assertEquals(null, state().account)
    }

    @Test
    fun eventsUpsertByIdAndToolResultsFoldIntoTheirCalls() {
        emit(
            event("u1", user("list the files")),
            event("a1", reply("Let me")),
            event("t1", tool("Bash", "ls")),
            event("unknown", ChatBody.Unknown("attachment")),
        )
        // a later word on a1 and the result of t1, which the origin read without its call
        emit(
            event("a1", reply("Let me look.")),
            event("t1", tool("", "", ChatToolStatus.OK, ChatToolKind.OTHER, output = "README.md")),
        )
        assertEquals(listOf("u1", "a1", "t1"), rows())
        assertEquals(reply("Let me look."), entry("a1").body)
        val result = (entry("t1").body as ChatBody.Tool).tool
        assertEquals("Bash", result.name)
        assertEquals("ls", result.title)
        assertEquals(ChatToolKind.SHELL, result.kind)
        assertEquals(ChatToolStatus.OK, result.status)
        assertEquals("README.md", result.output)
    }

    @Test
    fun subagentEventsFoldUnderTheirTask() {
        emit(
            event("task", tool("Agent", "Explore the repo", kind = ChatToolKind.TASK)),
            event("agent-1:line-0", user("Explore"), parent = "task"),
            event("agent-1:line-9", tool("Read", "README.md", kind = ChatToolKind.READ), parent = "task"),
            event("a1", reply("Waiting on it")),
            // a subagent whose task is older than what was read
            event("agent-2:line-0", reply("Found it"), parent = "old-task"),
        )
        assertEquals(listOf("task[agent-1:line-0, agent-1:line-9]", "a1", "?old-task[agent-2:line-0]"), rows())

        // the older page brings that task: it takes its subagent's events, at its own place
        bed.test.core.olderPages[bed.ref("p1")] =
            ArrayDeque(
                listOf(
                    listOf(
                        event("u0", user("start")),
                        event("old-task", tool("Agent", "Find the bug", kind = ChatToolKind.TASK)),
                    ),
                ),
            )
        chat.loadOlder()
        assertEquals(
            listOf("u0", "old-task[agent-2:line-0]", "task[agent-1:line-0, agent-1:line-9]", "a1"),
            rows(),
        )
    }

    @Test
    fun loadingOlderRowsKeepsTheScrollAnchor() {
        emit(event("u5", user("five")), event("t5", tool("", "", ChatToolStatus.OK, output = "done")), live)
        val before = state().items.map { it.key }
        bed.test.core.olderPages[bed.ref("p1")] =
            ArrayDeque(
                listOf(
                    // the call t5's result was read without, and an older update of it that must not undo the result
                    listOf(event("u4", user("four")), event("t5", tool("Bash", "make"))),
                    emptyList(),
                ),
            )
        chat.loadOlder()
        assertEquals(listOf("u4", "u5", "t5"), rows())
        // the rows already shown keep their keys, after the page; the list holds the old top in place
        assertEquals(before, state().items.drop(1).map { it.key })
        assertEquals(before.first(), state().anchor)
        assertEquals(OlderPages.More, state().older)
        val tool = (entry("t5").body as ChatBody.Tool).tool
        assertEquals("make", tool.title)
        assertEquals(ChatToolStatus.OK, tool.status)

        // the start: no more pages are asked for
        chat.loadOlder()
        assertEquals(OlderPages.AtStart, state().older)
        val asked =
            bed.test.core.calls
                .count { it.method == "loadOlder" }
        chat.loadOlder()
        assertEquals(
            asked,
            bed.test.core.calls
                .count { it.method == "loadOlder" },
        )
    }

    private fun pages(vararg pages: List<ChatEvent>) {
        bed.test.core.olderPages[bed.ref("p1")] = ArrayDeque(pages.toList())
    }

    private fun keys() = state().items.map { it.key }

    @Test
    fun aPageThatBringsThePlaceholdersTaskAnchorsOnTheFirstRowItLeftInPlace() {
        // the backlog starts partway through a subagent's run: its placeholder is the top row
        emit(event("agent-2:line-0", reply("Found it"), parent = "old-task"), event("a1", reply("Done")), live)
        assertEquals(listOf("e:old-task", "e:a1"), keys())
        pages(
            listOf(
                event("u0", user("start")),
                event("old-task", tool("Agent", "Find the bug", kind = ChatToolKind.TASK)),
                event("u1", user("meanwhile")),
            ),
        )
        chat.loadOlder()
        assertEquals(listOf("u0", "old-task[agent-2:line-0]", "u1", "a1"), rows())
        // the task's row has the placeholder's key; the list holds a1, the first row the page didn't move
        assertTrue("e:old-task" in keys())
        assertEquals("e:a1", state().anchor)
    }

    @Test
    fun aPageThatTakesEveryShownRowAnchorsOnTheMovedOne() {
        emit(event("agent-2:line-0", reply("Found it"), parent = "old-task"), live)
        pages(
            listOf(event("old-task", tool("Agent", "Find the bug", kind = ChatToolKind.TASK)), event("u1", user("x"))),
        )
        chat.loadOlder()
        assertEquals(listOf("old-task[agent-2:line-0]", "u1"), rows())
        assertEquals("e:old-task", state().anchor)
        assertTrue(state().anchor in keys())
    }

    @Test
    fun aResetWhileAPageIsOnItsWayDropsThatPage() {
        emit(event("u1", user("old")), live)
        pages(listOf(event("u0", user("older"))), listOf(event("v0", user("new start"))))
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["loadOlder"] = gate
        chat.loadOlder()
        assertEquals(OlderPages.Loading, state().older)

        emit(reset, event("v1", user("new")), live)
        assertEquals(OlderPages.More, state().older)
        gate.complete(Unit)
        // the stale page is read, then dropped; the next one is the new conversation's
        eventually {
            bed.test.core.olderPages[bed.ref("p1")]
                ?.size == 1
        }
        chat.loadOlder()
        eventually { rows() == listOf("--", "v0", "v1") }
        assertEquals("e:v1", state().anchor)
    }

    @Test
    fun aPageCancelledByAResetWaitsForTheReset() {
        emit(event("u1", user("old")), live)
        bed.test.core.failNext("loadOlder", MobileException.Cancelled())
        chat.loadOlder()
        // the core's reset is on its way: no page is asked for until it lands
        assertEquals(OlderPages.Loading, state().older)
        assertEquals(listOf("u1"), rows())
        emit(reset, event("v1", user("new")), live)
        assertEquals(listOf("--", "v1"), rows())
        assertEquals(OlderPages.More, state().older)
        pages(listOf(event("v0", user("new start"))))
        chat.loadOlder()
        assertEquals(listOf("--", "v0", "v1"), rows())
    }

    @Test
    fun aPageThatFailsSaysWhyAndCanBeAskedAgain() {
        emit(event("u1", user("one")), live)
        bed.test.core.failNext("loadOlder", MobileException.Remote("the origin failed to read the page"))
        chat.loadOlder()
        assertEquals(OlderPages.Failed("the origin failed to read the page"), state().older)
        chat.loadOlder()
        assertEquals(OlderPages.AtStart, state().older)
    }

    @Test
    fun aResetStartsTheNewConversationUnderADivider() {
        emit(event("u1", user("old")), event("approval-1", ChatBody.Approval(approval)), live)
        emit(reset, event("v1", user("new")), event("conv", ChatBody.Conversation("claude", "c2", null, null)), live)
        assertEquals(listOf("--", "v1"), rows())
        assertNull(state().approval)
        assertEquals("c2", state().conversation?.conversationId)
        assertEquals(OlderPages.More, state().older)
    }

    @Test
    fun anApprovalCardComesWithItsLabelsAndGoesWhenAnsweredAnywhere() {
        pane.labels = listOf("Yes", "Yes, and don't ask again", "No")
        emit(event("approval-1", ChatBody.Approval(approval)))
        assertEquals(ApprovalCard("approval-1", approval, pane.labels), state().approval)
        // answered on another device: the origin says it is done
        emit(event("approval-1", ChatBody.ApprovalDone(approval)))
        assertNull(state().approval)

        // answered here
        emit(event("approval-2", ChatBody.Approval(approval)))
        chat.answerApproval("approval-2", "Yes")
        // with the card's prompt: the core answers only the dialog that asks about it
        assertEquals(FakeAction.Approval(approval, "Yes"), pane.actions.last())
        assertNull(state().approval)
        assertNull(state().problem)
    }

    @Test
    fun anApprovalTheCoreCantConfirmOnScreenStaysUpAndOffersTheTerminal() {
        emit(event("approval-1", ChatBody.Approval(approval)))
        pane.outcome = AgentInputOutcome.NOT_ON_SCREEN

        // another call's dialog, or one the core can't tell apart: nothing went, and the card still waits
        chat.answerApproval("approval-1", "Yes")
        assertEquals("approval-1", state().approval?.id)
        assertEquals(ChatProblem.NotOnScreen, state().problem)
        assertTrue(state().problem!!.offersTerminal)
        chat.problemShown()
        // the origin's word ends it: answered in the terminal, or anywhere else
        emit(event("approval-1", ChatBody.ApprovalDone(approval)))
        assertNull(state().approval)
    }

    @Test
    fun aTapAnswersOnlyTheCardItWasDrawnOn() {
        emit(event("approval-1", ChatBody.Approval(approval)))
        // the next prompt replaced the card before the tap on the first one's buttons landed
        emit(event("approval-1", ChatBody.ApprovalDone(approval)), event("approval-2", ChatBody.Approval(approval)))
        val before = pane.actions.size
        chat.answerApproval("approval-1", "Yes")
        assertEquals(before, pane.actions.size)
        assertEquals("approval-2", state().approval?.id)
        assertEquals(ChatProblem.NotOnScreen, state().problem)
        chat.problemShown()

        emit(event("approval-2", ChatBody.ApprovalDone(approval)), event("q1", question()))
        chat.answerQuestion("q0", listOf(QuestionAnswer(listOf("Postgres"), null)))
        assertEquals(before, pane.actions.size)
        assertEquals("q1", state().question?.id)
    }

    @Test
    fun aQuestionWhosePickerIsNoLongerOnScreenGoesAndOffersTheTerminal() {
        emit(event("q1", question()))
        assertEquals("q1", state().question?.id)
        pane.outcome = AgentInputOutcome.NOT_ON_SCREEN

        chat.answerQuestion("q1", listOf(QuestionAnswer(listOf("Postgres"), null)))
        assertNull(state().question)
        assertEquals(ChatProblem.NotOnScreen, state().problem)
        // the answer the transcript then records leaves it down
        emit(event("q1", question(ChatToolStatus.OK)))
        assertNull(state().question)
    }

    @Test
    fun aQuestionCardComesUpWhileItRunsAndGoesOnceAnswered() {
        emit(event("q1", question()))
        val card = state().question!!
        assertEquals(
            "Which database?",
            card.question.questions
                .single()
                .question,
        )
        chat.answerQuestion("q1", listOf(QuestionAnswer(listOf("Postgres"), null)))
        assertEquals(
            FakeAction.Question(card.question, listOf(QuestionAnswer(listOf("Postgres"), null))),
            pane.actions.last(),
        )
        assertNull(state().question)

        // a question answered on another device goes when the transcript records the answer
        emit(event("q2", question()))
        assertEquals("q2", state().question?.id)
        emit(event("q2", question(ChatToolStatus.OK)))
        assertNull(state().question)
    }

    @Test
    fun sendAndStopReachThePane() {
        send("fix the build")
        send("   ")
        chat.stop()
        assertEquals(listOf(FakeAction.Prompt("fix the build"), FakeAction.Interrupt), pane.actions.toList())
        assertFalse(state().busy)
        assertNull(state().problem)
    }

    @Test
    fun anActionThatDidNothingSaysWhy() {
        val outcomes =
            mapOf(
                AgentInputOutcome.NOT_ON_SCREEN to ChatProblem.NotOnScreen,
                AgentInputOutcome.DIALOG_OPEN to ChatProblem.DialogOpen,
                AgentInputOutcome.LEAD_REFUSED to ChatProblem.LeadRefused,
                AgentInputOutcome.UNCONFIRMED to ChatProblem.Unconfirmed,
            )
        for ((outcome, problem) in outcomes) {
            pane.outcome = outcome
            send("hi")
            assertEquals(problem, state().problem)
        }
        assertFalse(ChatProblem.DialogOpen.offersTerminal)
        assertFalse(ChatProblem.LeadRefused.offersTerminal)
        assertTrue(ChatProblem.Unconfirmed.offersTerminal)

        val errors =
            mapOf(
                PaneInputException.Refused("another action is on its way") to
                    ChatProblem.Refused("another action is on its way"),
                PaneInputException.Backpressure() to ChatProblem.NotReading,
                PaneInputException.Closed() to ChatProblem.Closed,
            )
        for ((error, problem) in errors) {
            pane.actionError = error
            chat.stop()
            assertEquals(problem, state().problem)
            assertFalse(state().busy)
        }

        // a pane no longer shown has no handle to act through
        pane.actionError = null
        bed.test.panes.hide(bed.ref("p1"))
        send("hi")
        assertEquals(ChatProblem.Closed, state().problem)
    }

    @Test
    fun theChatEndingEndsItsPhase() {
        emit(live)
        bed.test.core.emit(MobileEvent.ChatState(bed.ref("p1"), ChatState.ENDED))
        assertEquals(ChatPhase.Ended, state().phase)
        eventually { bed.chat("p1") == null }
    }
}
