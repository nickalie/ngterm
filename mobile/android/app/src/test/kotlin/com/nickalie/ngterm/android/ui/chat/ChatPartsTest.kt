package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The chat screen's pure parts: a diff's lines, a question card's draft, and the approval card's stand-in answers. */
class ChatPartsTest {
    @Test
    fun aDiffsLinesAreItsHeadersHunksAndChanges() {
        val lines = diffLines(ChatSamples.diff).lines.map { it.kind }
        assertEquals(
            listOf(
                DiffLineKind.File,
                DiffLineKind.File,
                DiffLineKind.File,
                DiffLineKind.Hunk,
                DiffLineKind.Context,
                DiffLineKind.Removed,
                DiffLineKind.Added,
                DiffLineKind.Added,
                DiffLineKind.Context,
                DiffLineKind.Context,
            ),
            lines,
        )
    }

    @Test
    fun aRemovedOrAddedLineThatLooksLikeAHeaderIsStillAChangeInsideAHunk() {
        val diff = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n--- old rule\n+++ new rule\ndiff --git a/y b/y\n--- a/y"
        assertEquals(
            listOf(
                DiffLineKind.File,
                DiffLineKind.File,
                DiffLineKind.Hunk,
                DiffLineKind.Removed,
                DiffLineKind.Added,
                DiffLineKind.File,
                DiffLineKind.File,
            ),
            diffLines(diff).lines.map { it.kind },
        )
    }

    @Test
    fun aLongDiffStopsAtItsLimitAndCountsTheRest() {
        val diff = (1..10).joinToString("\n") { "+line $it" }
        val cut = diffLines(diff, limit = 4)
        assertEquals(listOf("+line 1", "+line 2", "+line 3", "+line 4"), cut.lines.map { it.text })
        assertEquals(6, cut.more)
        assertEquals(0, diffLines(diff).more)
    }

    private val single =
        ChatQuestionItem(
            "Which database?",
            "Database",
            false,
            listOf(ChatQuestionOption("Postgres", null), ChatQuestionOption("SQLite", null)),
        )
    private val multi = ChatSamples.shells.questions.single()
    private val both = ChatQuestion(listOf(single, multi), ChatToolStatus.RUNNING, null)

    @Test
    fun aDraftAnswersOnlyOnceEveryQuestionHasOne() {
        var draft = QuestionDraft.of(both)
        assertNull(draft.answers(both))
        draft = draft.toggle(0, single, "SQLite")
        assertNull(draft.answers(both))
        draft = draft.toggle(1, multi, "bash").toggle(1, multi, "fish")
        assertEquals(
            listOf(QuestionAnswer(listOf("SQLite"), null), QuestionAnswer(listOf("bash", "fish"), null)),
            draft.answers(both),
        )
        // a second tap lets a multi-select pick go; a single-select one moves
        draft = draft.toggle(1, multi, "bash").toggle(0, single, "Postgres")
        assertEquals(
            listOf(QuestionAnswer(listOf("Postgres"), null), QuestionAnswer(listOf("fish"), null)),
            draft.answers(both),
        )
    }

    @Test
    fun typingTakesASingleSelectPickAndClearingLetsItGo() {
        val question = ChatQuestion(listOf(single), ChatToolStatus.RUNNING, null)
        var draft = QuestionDraft.of(question).toggle(0, single, "Postgres")
        draft = draft.withText(0, single, "  MariaDB ")
        assertEquals(listOf(QuestionAnswer(emptyList(), "MariaDB")), draft.answers(question))
        // picking an option lets the typed row go, keeping its text
        draft = draft.toggle(0, single, "SQLite")
        assertEquals(listOf(QuestionAnswer(listOf("SQLite"), null)), draft.answers(question))
        assertEquals("  MariaDB ", draft.other[0])
        // its marker picks it back
        draft = draft.toggleOther(0, single)
        assertEquals(listOf(QuestionAnswer(emptyList(), "MariaDB")), draft.answers(question))
        draft = draft.withText(0, single, " ")
        assertNull(draft.answers(question))
    }

    @Test
    fun aMultiSelectQuestionTakesTheTypedAnswerWithItsPicks() {
        val question = ChatSamples.shells
        var draft = QuestionDraft.of(question).toggle(0, multi, "zsh").withText(0, multi, "nushell")
        assertEquals(listOf(QuestionAnswer(listOf("zsh"), "nushell")), draft.answers(question))
        draft = draft.toggleOther(0, multi)
        assertEquals(listOf(QuestionAnswer(listOf("zsh"), null)), draft.answers(question))
        // an empty typed row takes no tap
        val empty = QuestionDraft.of(question)
        assertEquals(empty, empty.toggleOther(0, multi))
    }

    @Test
    fun anApprovalCardStandsInTheAgentsOwnAnswersUntilTheScreenIsRead() {
        assertEquals(listOf("Yes", "No"), genericApprovalLabels("claude"))
        assertEquals(
            listOf("Yes, proceed", "No, and tell Codex what to do differently"),
            genericApprovalLabels("codex"),
        )
        assertEquals(listOf("Yes", "No"), genericApprovalLabels(null))
    }
}
