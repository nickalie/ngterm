package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AgentInputOutcome
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class FakePaneTest {
    private fun shown() = FakePane(frames = { ByteArray(0) })

    @Test
    fun chatActionsAreRecordedInOrderAndAnswerTheOutcomeSet() =
        runTest {
            val pane = shown()
            val question =
                ChatQuestion(
                    questions =
                        listOf(
                            ChatQuestionItem(
                                question = "Pick a color",
                                header = null,
                                multiSelect = false,
                                options = listOf(ChatQuestionOption("Red", null), ChatQuestionOption("Green", null)),
                            ),
                        ),
                    status = ChatToolStatus.RUNNING,
                    output = null,
                )
            val answers = listOf(QuestionAnswer(picked = listOf("Red"), other = null))

            assertEquals(AgentInputOutcome.SENT, pane.sendPrompt("/compact keep it short"))
            assertEquals(AgentInputOutcome.SENT, pane.interrupt())
            pane.outcome = AgentInputOutcome.NOT_ON_SCREEN
            assertEquals(AgentInputOutcome.NOT_ON_SCREEN, pane.answerApproval("Yes"))
            assertEquals(AgentInputOutcome.NOT_ON_SCREEN, pane.answerQuestion(question, answers))
            assertEquals(
                listOf(
                    FakeAction.Prompt("/compact keep it short"),
                    FakeAction.Interrupt,
                    FakeAction.Approval("Yes"),
                    FakeAction.Question(question, answers),
                ),
                pane.actions,
            )

            pane.labels = listOf("Yes", "No")
            assertEquals(listOf("Yes", "No"), pane.approvalLabels())
        }

    @Test
    fun aRefusalOrAHiddenPaneThrowsAndRecordsNothing() {
        val pane = shown()
        pane.actionError = PaneInputException.Refused("the pane runs no agent the chat can answer")
        assertThrows(PaneInputException.Refused::class.java) { runBlocking { pane.interrupt() } }

        val hidden = FakePane()
        assertThrows(PaneInputException.Closed::class.java) { runBlocking { hidden.sendPrompt("hi") } }
        assertThrows(PaneInputException.Closed::class.java) { runBlocking { hidden.approvalLabels() } }
        assertTrue(pane.actions.isEmpty())
        assertTrue(hidden.actions.isEmpty())
    }
}
