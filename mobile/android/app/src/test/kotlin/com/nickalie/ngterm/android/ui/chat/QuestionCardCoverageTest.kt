package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.LocalSaveableStateRegistry
import androidx.compose.runtime.saveable.SaveableStateRegistry
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.QuestionAnswer
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The card's real draft follows typed answers, choice changes and restored Android state. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class QuestionCardCoverageTest {
    @get:Rule
    val compose = createComposeRule()

    private val answers = mutableListOf<List<QuestionAnswer>>()
    private val visible = mutableStateOf(true)
    private var registry = SaveableStateRegistry(null) { true }
    private val busy = mutableStateOf(false)

    private fun show(
        multiple: Boolean = false,
        header: String? = null,
    ) {
        val question =
            ChatQuestion(
                listOf(
                    ChatQuestionItem(
                        "Which shell?",
                        header,
                        multiple,
                        listOf(
                            ChatQuestionOption("bash", null),
                            ChatQuestionOption("fish", " "),
                            ChatQuestionOption("zsh", "Has completion"),
                        ),
                    ),
                ),
                ChatToolStatus.RUNNING,
                null,
            )
        compose.setContent {
            if (visible.value) {
                CompositionLocalProvider(LocalSaveableStateRegistry provides registry) {
                    NgtermTheme { QuestionCardView(OpenQuestion("q1", question), null, busy.value, { answers += it }) }
                }
            }
        }
    }

    @Test
    fun typingTakesTheRadioChoiceAndClearingItRequiresAnAnswerAgain() {
        show()
        compose.onNodeWithText("Has completion").assertIsDisplayed()
        compose.onNodeWithContentDescription("Type an answer").assertIsNotEnabled()
        compose.onNodeWithText("bash").performClick()
        compose.onNodeWithText("Send answers").assertIsEnabled()
        compose.onNode(hasSetTextAction()).performTextInput("tcsh\nplease")
        compose.onNodeWithText("tcsh please").assertExists()
        compose.onNodeWithText("Send answers").performClick()
        assertEquals(listOf(listOf(QuestionAnswer(emptyList(), "tcsh please"))), answers)
        compose.onNode(hasSetTextAction()).performTextReplacement("")
        compose.onNodeWithText("Send answers").assertIsNotEnabled()
        compose.onNodeWithText("fish").performClick()
        busy.value = true
        compose.onNodeWithText("Send answers").assertIsNotEnabled()
        compose.onNodeWithText("Sending").assertIsDisplayed()
    }

    @Test
    fun aTypedCheckboxCanBeUnpickedAndPickedWithoutLosingItsText() {
        show(multiple = true, header = " ")
        compose.onNode(hasSetTextAction()).performTextInput("tcsh")
        compose.onNodeWithContentDescription("Type an answer").performClick()
        compose.onNodeWithText("Send answers").assertIsNotEnabled()
        compose.onNodeWithText("tcsh").assertExists()
        compose.onNodeWithContentDescription("Type an answer").performClick()
        compose.onNodeWithText("fish").performClick()
        compose.onNodeWithText("Send answers").performClick()
        assertEquals(listOf(listOf(QuestionAnswer(listOf("fish"), "tcsh"))), answers)
    }

    @Test
    fun malformedSavedDraftsAreDiscardedAndTheUserCanAnswerAfresh() {
        show()
        val valid = listOf("q1", listOf(listOf("bash")), listOf(""), listOf(false))
        val invalid =
            listOf(
                emptyList(),
                valid.toMutableList().apply { this[1] = "not a list" },
                valid.toMutableList().apply { this[1] = listOf("not a list") },
                valid.toMutableList().apply { this[1] = listOf(listOf(7)) },
                valid.toMutableList().apply { this[2] = listOf(7) },
                valid.toMutableList().apply { this[3] = listOf("false") },
                valid.toMutableList().apply { this[0] = "q2" },
            )
        invalid.forEach { saved ->
            compose.onNodeWithText("bash").performClick()
            compose.onNodeWithText("Send answers").assertIsEnabled()
            restore(saved)
            compose.onNodeWithText("Send answers").assertIsNotEnabled()
            compose.onNodeWithText("fish").performClick()
            compose.onNodeWithText("Send answers").performClick()
        }
        assertEquals(List(invalid.size) { listOf(QuestionAnswer(listOf("fish"), null)) }, answers)
    }

    private fun restore(draft: List<Any>) {
        lateinit var saved: Map<String, List<Any?>>
        compose.runOnIdle { saved = registry.performSave() }
        compose.runOnIdle { visible.value = false }
        compose.waitForIdle()
        val restored =
            saved.mapValues { (_, values) ->
                if ((values.firstOrNull() as? List<*>)?.firstOrNull() == "q1") listOf(draft) else values
            }
        registry = SaveableStateRegistry(restored) { true }
        compose.runOnIdle { visible.value = true }
        compose.waitForIdle()
    }
}
