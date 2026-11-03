package com.nickalie.ngterm.android.ui.asks

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAsk
import java.text.BreakIterator

/** The most UTF-8 bytes an ask's text may hold (`AskInputForm::MAX_TEXT_BYTES` in ngterm-core). */
const val ASK_TEXT_MAX_BYTES = 65_536

/** The answer of a dismissed ask (back, a swipe, or its cancel button): no button, no payload; the origin escapes. */
val EscapedReply = AskReply(button = null, text = null, selected = null)

/**
 * The user's unsubmitted input to an ask, after ngterm-core's `AskDraft` (upstream's): an input field's [text], or a
 * choice list's [checked] ids and its free-form row (its [text], [otherSelected]). Immutable, so the sheet keeps it as
 * state and every change is a new draft.
 *
 * A radio list starts on its first checked id, else its first item. In a choice list a non-blank text selects the
 * free-form row (a radio list hands its pick over, and gets it back when the text goes blank); a blank one unselects
 * it, and an answer never carries blank free-form text.
 */
data class AskDraft(
    val text: String,
    val checked: Set<String>,
    val otherSelected: Boolean,
    private val choiceIds: List<String>,
    private val multiple: Boolean,
    private val hasOther: Boolean,
    internal val previousPick: String?,
) {
    /** The free-form row's text is not blank, so the row can be selected. */
    val otherEnabled: Boolean get() = hasOther && !isBlank(text)

    /** The checked ids in the ask's order; the free-form row answers through [otherText]. */
    val selected: List<String> get() = choiceIds.filter { it in checked }

    /** The free-form row's text while the row is selected. */
    val otherText: String? get() = text.takeIf { otherSelected && otherEnabled }

    /**
     * The text replaced by [value], cut to the cap. In a choice list a non-blank text selects the free-form row and a
     * blank one unselects it.
     */
    fun withText(value: String): AskDraft {
        val next = copy(text = clampAskText(value))
        if (!hasOther) return next
        return if (isBlank(next.text)) next.unselectOther() else next.selectOther()
    }

    /**
     * Picks the radio item or flips the checkbox at [index]; the free-form row (the index after the choices) toggles
     * only while enabled, and a picked radio row stays picked.
     */
    fun toggle(index: Int): AskDraft {
        val id = choiceIds.getOrNull(index)
        return when {
            hasOther && index == choiceIds.size -> toggleOther()
            id == null -> this
            !multiple -> copy(checked = setOf(id), otherSelected = false, previousPick = null)
            id in checked -> copy(checked = checked - id)
            else -> copy(checked = checked + id)
        }
    }

    private fun toggleOther(): AskDraft =
        when {
            !otherEnabled -> this
            multiple && otherSelected -> copy(otherSelected = false)
            else -> selectOther()
        }

    private fun selectOther(): AskDraft =
        when {
            otherSelected -> this
            multiple -> copy(otherSelected = true)
            else -> copy(otherSelected = true, checked = emptySet(), previousPick = checked.firstOrNull())
        }

    private fun unselectOther(): AskDraft =
        when {
            !otherSelected -> {
                this
            }

            multiple -> {
                copy(otherSelected = false)
            }

            else -> {
                copy(
                    otherSelected = false,
                    checked = setOfNotNull(previousPick ?: choiceIds.firstOrNull()),
                    previousPick = null,
                )
            }
        }

    companion object {
        /** The draft [ask] opens with: its field's prefill, or its list's checked ids (a radio list's first). */
        fun of(ask: RemoteAsk): AskDraft {
            val list = ask.choices
            val ids = list?.choices.orEmpty().map { it.id }
            val checked =
                when {
                    list == null -> emptySet()
                    list.multiple -> list.checked.toSet()
                    else -> setOfNotNull(list.checked.firstOrNull() ?: ids.firstOrNull())
                }
            return AskDraft(
                text = clampAskText(ask.input?.value.orEmpty()),
                checked = checked,
                otherSelected = false,
                choiceIds = ids,
                multiple = list?.multiple == true,
                hasOther = list?.other != null,
                previousPick = null,
            )
        }
    }
}

/**
 * The answer of pressing [buttonId] on [ask] with [draft] (ngterm-core's `PendingAsk::answer`): the cancel button
 * escapes with no payload; any other answers with the field's text, or the selected ids plus the selected free-form
 * row's text.
 */
fun askReply(
    ask: RemoteAsk,
    buttonId: String,
    draft: AskDraft = AskDraft.of(ask),
): AskReply =
    when {
        ask.cancelId == buttonId -> EscapedReply
        ask.input != null -> AskReply(button = buttonId, text = draft.text, selected = null)
        ask.choices != null -> AskReply(button = buttonId, text = draft.otherText, selected = draft.selected)
        else -> AskReply(button = buttonId, text = null, selected = null)
    }

/** Whether [text] has no non-whitespace character. */
fun isBlank(text: String): Boolean = text.all { it.isWhitespace() }

/**
 * Whether [text] holds a C0 control or DEL, a line break and a tab excepted in a [multiline] field (ngterm-core's
 * `has_forbidden_controls`): a field refuses an edit that would add one, as desktop does.
 */
fun hasForbiddenControls(
    text: String,
    multiline: Boolean,
): Boolean =
    text.any { ch ->
        val control = ch.code < CONTROL_END || ch.code == DEL
        control && (!multiline || (ch != '\n' && ch != '\t'))
    }

/**
 * [text] as a field of the kind ([multiline]) can answer it, for text typed where no field can refuse an edit (a
 * notification's reply): its forbidden controls turned to spaces, then cut to the cap.
 */
fun fitAskText(
    text: String,
    multiline: Boolean,
): String {
    val fitted =
        if (hasForbiddenControls(text, multiline)) {
            text.map { if (hasForbiddenControls(it.toString(), multiline)) ' ' else it }.joinToString("")
        } else {
            text
        }
    return clampAskText(fitted)
}

/** [text] cut to [ASK_TEXT_MAX_BYTES] UTF-8 bytes at the end of the last whole character that fits. */
fun clampAskText(text: String): String {
    if (text.toByteArray(Charsets.UTF_8).size <= ASK_TEXT_MAX_BYTES) return text
    val breaks = BreakIterator.getCharacterInstance().apply { setText(text) }
    var bytes = 0
    var end = 0
    var start = breaks.first()
    var next = breaks.next()
    while (next != BreakIterator.DONE) {
        bytes += text.substring(start, next).toByteArray(Charsets.UTF_8).size
        if (bytes > ASK_TEXT_MAX_BYTES) break
        end = next
        start = next
        next = breaks.next()
    }
    return text.substring(0, end)
}

private const val CONTROL_END = 0x20
private const val DEL = 0x7f
