package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Every tool kind has its own accessible word and icon; a failed question reads as Failed. */
@RunWith(AndroidJUnit4::class)
class ChatStyleTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun everyToolKindHasItsOwnWordAndGlyph() {
        assertEquals(
            listOf(
                R.drawable.ic_tool_shell to R.string.chat_kind_shell,
                R.drawable.ic_tool_edit to R.string.chat_kind_edit,
                R.drawable.ic_tool_read to R.string.chat_kind_read,
                R.drawable.ic_tool_search to R.string.chat_kind_search,
                R.drawable.ic_tool_web to R.string.chat_kind_web,
                R.drawable.ic_tool_task to R.string.chat_kind_task,
                R.drawable.ic_tool_other to R.string.chat_kind_other,
            ),
            ChatToolKind.entries.map { it.icon to it.word },
        )
    }

    @Test
    fun aFailedQuestionSaysFailedRatherThanAnsweredOrNeedsYou() {
        compose.setContent {
            NgtermTheme { QuestionRow(ChatSamples.shells.copy(status = ChatToolStatus.ERROR), false, {}) }
        }
        compose.onNodeWithText("Failed").assertIsDisplayed()
        compose.onNodeWithText("Answered").assertDoesNotExist()
        compose.onNodeWithText("Needs you").assertDoesNotExist()
    }
}
