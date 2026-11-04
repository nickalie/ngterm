package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.QuestionAnswer

/**
 * The answers a question card holds before they are sent, one slot per question of the `AskUserQuestion`: the option
 * labels [picked], the text typed into its "Type an answer" row ([other]), and whether that row is picked
 * ([otherPicked]).
 *
 * A single-select question holds one pick: an option, or the typed text, which typing picks and clearing lets go. A
 * multi-select question holds any options, and the typed text with them once it is picked.
 */
data class QuestionDraft(
    val picked: List<List<String>>,
    val other: List<String>,
    val otherPicked: List<Boolean>,
) {
    /** The draft with option [label] of question [index] picked, or let go when it is a multi-select pick already. */
    fun toggle(
        index: Int,
        item: ChatQuestionItem,
        label: String,
    ): QuestionDraft {
        val now = picked[index]
        val next =
            when {
                !item.multiSelect -> listOf(label)
                label in now -> now - label
                else -> now + label
            }
        val otherNow = if (item.multiSelect) otherPicked[index] else false
        return copy(picked = picked.with(index, next), otherPicked = otherPicked.with(index, otherNow))
    }

    /** The draft with [text] typed for question [index]: typing picks the row, clearing it lets the row go. */
    fun withText(
        index: Int,
        item: ChatQuestionItem,
        text: String,
    ): QuestionDraft {
        val typed = text.isNotBlank()
        val options = if (typed && !item.multiSelect) emptyList() else picked[index]
        return copy(
            picked = picked.with(index, options),
            other = other.with(index, text),
            otherPicked = otherPicked.with(index, typed),
        )
    }

    /** The draft with question [index]'s typed row picked or let go; it takes a tap only while it holds text. */
    fun toggleOther(
        index: Int,
        item: ChatQuestionItem,
    ): QuestionDraft {
        if (other[index].isBlank()) return this
        val on = !otherPicked[index] || !item.multiSelect
        val options = if (on && !item.multiSelect) emptyList() else picked[index]
        return copy(picked = picked.with(index, options), otherPicked = otherPicked.with(index, on))
    }

    /**
     * The answers to send for [question], one per question in order; `null` while one has none (a single-select
     * question takes exactly one, a multi-select one at least one).
     */
    fun answers(question: ChatQuestion): List<QuestionAnswer>? =
        question.questions.indices.map { index ->
            val text = other[index].trim().takeIf { otherPicked[index] && it.isNotEmpty() }
            val count = picked[index].size + (if (text != null) 1 else 0)
            if (count == 0) return null
            QuestionAnswer(picked = picked[index], other = text)
        }

    companion object {
        /** An empty draft for [question]. */
        fun of(question: ChatQuestion): QuestionDraft {
            val count = question.questions.size
            return QuestionDraft(List(count) { emptyList() }, List(count) { "" }, List(count) { false })
        }
    }
}

private fun <T> List<T>.with(
    index: Int,
    value: T,
): List<T> = mapIndexed { at, old -> if (at == index) value else old }
