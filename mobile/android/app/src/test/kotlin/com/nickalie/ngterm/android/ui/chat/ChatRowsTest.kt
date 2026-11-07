package com.nickalie.ngterm.android.ui.chat

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.View
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHasNoClickAction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.LayoutDirection
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatNoticeKind
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

/** Empty transcript entries still read clearly; only rows with details offer to open them. */
@RunWith(AndroidJUnit4::class)
class ChatRowsTest {
    @get:Rule
    val compose = createComposeRule()

    private val expanded = mutableStateOf(false)

    private fun clickLabel(label: String) =
        SemanticsMatcher("tap says $label") {
            it.config.getOrElseNullable(SemanticsActions.OnClick) { null }?.label == label
        }

    @Test
    fun aToolsNameAndThenTheEarlierCallLabelStandInForAMissingTitle() {
        val tool = mutableStateOf(ChatTool("Read", ChatToolKind.READ, " ", ChatToolStatus.OK, null, null, false))
        compose.setContent { NgtermTheme { ToolRow(tool.value, false, {}) } }
        compose.onNodeWithText("Read").assertHasNoClickAction().assert(hasText("Done"))
        tool.value = tool.value.copy(name = " ")
        compose.onNodeWithText("An earlier tool call").assertHasNoClickAction()
    }

    @Test
    fun anEmptyOutputCanBeRestoredOpenAndACutOutputExplainsWhereTheRestIs() {
        expanded.value = true
        val tool =
            mutableStateOf(
                ChatTool(
                    "Bash",
                    ChatToolKind.SHELL,
                    "pwd",
                    ChatToolStatus.OK,
                    output = "",
                    diff = null,
                    truncated = false,
                ),
            )
        compose.setContent { NgtermTheme { ToolRow(tool.value, expanded.value, { expanded.value = !expanded.value }) } }
        compose.onNodeWithText("pwd").assert(clickLabel("Hide output"))
        compose.onNodeWithText("Cut short here", substring = true).assertDoesNotExist()
        tool.value = tool.value.copy(output = "/home/nick\n\n", truncated = true)
        compose.onNodeWithText("/home/nick").assertIsDisplayed()
        compose.onNodeWithText("Cut short here: the terminal has the rest.").assertIsDisplayed()
        compose.onNodeWithText("pwd").performClick()
        compose.onNodeWithText("/home/nick").assertDoesNotExist()
        compose.onNodeWithText("pwd").assert(clickLabel("Show output")).performClick()
        compose.onNodeWithText("/home/nick").assertIsDisplayed()
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun earlierSubagentStepsOpenAndCloseInEitherReadingDirection() {
        val direction = mutableStateOf(LayoutDirection.Ltr)
        lateinit var view: View
        var ruleColor = 0
        compose.setContent {
            CompositionLocalProvider(LocalLayoutDirection provides direction.value) {
                NgtermTheme {
                    view = LocalView.current
                    ruleColor = MaterialTheme.colorScheme.outlineVariant.toArgb()
                    SubagentRow(1, expanded.value, { expanded.value = !expanded.value }) { Text("Checked the parser") }
                }
            }
        }
        val row = compose.onNodeWithText("A subagent's earlier task")
        row.assert(hasText("1 step")).assert(clickLabel("Show steps")).performClick()
        row.assert(clickLabel("Hide steps"))
        compose.onNodeWithText("Checked the parser").assertIsDisplayed()
        assertRuleOnLeadingSide(view, ruleColor, LayoutDirection.Ltr)
        direction.value = LayoutDirection.Rtl
        compose.onNodeWithText("Checked the parser").assertIsDisplayed()
        assertRuleOnLeadingSide(view, ruleColor, LayoutDirection.Rtl)
        row.performClick()
        compose.onNodeWithText("Checked the parser").assertDoesNotExist()
    }

    // Render the real Android view so the assertion also catches a missing rule or one left on the wrong side.
    private fun assertRuleOnLeadingSide(
        view: View,
        color: Int,
        direction: LayoutDirection,
    ) {
        compose.runOnIdle {
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                view.draw(Canvas(bitmap))
                val columns =
                    (0 until bitmap.width).filter { x ->
                        (0 until bitmap.height).any { y -> bitmap.getPixel(x, y) == color }
                    }
                assertTrue("the steps have a visible rule", columns.isNotEmpty())
                val leading =
                    if (direction ==
                        LayoutDirection.Ltr
                    ) {
                        0 until bitmap.width / 2
                    } else {
                        bitmap.width / 2 until bitmap.width
                    }
                assertTrue("the rule follows the reading direction", columns.all { it in leading })
            } finally {
                bitmap.recycle()
            }
        }
    }

    @Test
    fun aTaskWithOnlyItsStepsOffersThemAndCountsThem() {
        val tool =
            mutableStateOf(ChatTool("Task", ChatToolKind.TASK, "Check", ChatToolStatus.RUNNING, null, null, false))
        compose.setContent {
            NgtermTheme {
                ToolRow(
                    tool.value,
                    expanded.value,
                    { expanded.value = !expanded.value },
                    steps = 2,
                ) { Text("Step result") }
            }
        }
        compose
            .onNodeWithText("Check")
            .assert(hasText("2 steps"))
            .assert(clickLabel("Show steps"))
            .performClick()
        compose.onNodeWithText("Step result").assertIsDisplayed()
        compose.onNode(hasScrollAction()).assertDoesNotExist()
        tool.value = tool.value.copy(output = "")
        compose.onNodeWithText("Step result").assertIsDisplayed()
        compose.onNode(hasScrollAction()).assertDoesNotExist()
        tool.value = tool.value.copy(output = "No errors")
        compose.onNodeWithText("No errors").assertIsDisplayed()
        compose.onNode(hasScrollAction()).assertExists()
    }

    @Test
    fun thinkingWithoutWordsIsQuietAndASummaryOpensAndCloses() {
        val text = mutableStateOf(" ** ** ")
        compose.setContent {
            NgtermTheme {
                ThinkingRow(
                    text.value,
                    expanded.value,
                    { expanded.value = !expanded.value },
                )
            }
        }
        compose.onNodeWithText("Thinking").assertHasNoClickAction()
        text.value = " **Check the parser** "
        compose.onNodeWithText("Check the parser").assert(clickLabel("Show all of it")).performClick()
        compose.onNodeWithText("Check the parser").assert(clickLabel("Show less")).performClick()
        compose.onNodeWithText("Check the parser").assert(clickLabel("Show all of it"))
    }

    @Test
    fun noticesNameTheirKindAndOnlyNonblankDetailsOpen() {
        val kind = mutableStateOf(ChatNoticeKind.COMPACTION)
        val text = mutableStateOf("")
        val detail = mutableStateOf<String?>(null)
        compose.setContent {
            NgtermTheme {
                NoticeRow(kind.value, text.value, detail.value, expanded.value, {
                    expanded.value =
                        !expanded.value
                })
            }
        }
        val labels = listOf("Context compacted", "Interrupted", "API error", "Summary")
        ChatNoticeKind.entries.zip(labels).forEach { (value, label) ->
            kind.value = value
            compose.onNodeWithText(label).assertHasNoClickAction()
        }
        detail.value = "  "
        compose.onNodeWithText("Summary").assertHasNoClickAction()
        text.value = "Earlier work"
        detail.value = "The parser now handles escapes"
        compose.onNodeWithText("Earlier work").assert(clickLabel("Show summary")).performClick()
        compose.onNodeWithText("The parser now handles escapes").assertIsDisplayed()
        compose.onNodeWithText("Earlier work").assert(clickLabel("Hide summary")).performClick()
        compose.onNodeWithText("The parser now handles escapes").assertDoesNotExist()
    }

    @Test
    fun questionRowsShowEveryQuestionAndOnlyAnAnswerWithWords() {
        expanded.value = true
        val question = mutableStateOf(ChatQuestion(emptyList(), ChatToolStatus.RUNNING, null))
        compose.setContent {
            NgtermTheme {
                QuestionRow(question.value, expanded.value, {
                    expanded.value =
                        !expanded.value
                })
            }
        }
        compose
            .onNodeWithText("Question")
            .assert(hasText("Needs you"))
            .assert(clickLabel("Hide the answer"))
        question.value =
            question.value.copy(questions = listOf(ChatQuestionItem(" ", null, false, emptyList())), output = " ")
        compose.onNodeWithText("Question").assert(clickLabel("Hide the answer"))
        compose.onNode(hasScrollAction()).assertDoesNotExist()
        question.value =
            question.value.copy(
                questions =
                    listOf(
                        ChatQuestionItem("First?", null, false, emptyList()),
                        ChatQuestionItem("Second?", null, false, emptyList()),
                    ),
                status = ChatToolStatus.OK,
                output = "Use fish\n",
            )
        compose.onNode(hasText("First?") and hasText("Answered")).assertIsDisplayed()
        compose.onNodeWithText("Second?").assertIsDisplayed()
        compose.onNodeWithText("Use fish").assertIsDisplayed()
        compose.onNode(hasScrollAction()).assertExists()
        compose.onNode(hasText("First?") and hasText("Answered")).performClick()
        compose.onNodeWithText("Second?").assertDoesNotExist()
        compose.onNodeWithText("First?").assert(clickLabel("Show the answer")).performClick()
        compose.onNodeWithText("Use fish").assertIsDisplayed()
    }

    @Test
    fun aPromptWithOneImageSaysSoAndWithoutImagesReadsOnlyItsWords() {
        val images = mutableIntStateOf(1)
        compose.setContent { NgtermTheme { PromptBubble("Look here", images.intValue) } }
        compose.onNodeWithContentDescription("You: Look here. With 1 image").assertIsDisplayed()
        images.intValue = 0
        compose.onNodeWithContentDescription("You: Look here").assertIsDisplayed()
        compose.onNodeWithContentDescription("You: Look here. With 1 image").assertDoesNotExist()
    }
}
