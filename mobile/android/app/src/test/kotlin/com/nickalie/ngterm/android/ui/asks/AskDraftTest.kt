package com.nickalie.ngterm.android.ui.asks

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAsk
import com.nickalie.ngterm.core.ffi.RemoteAskButton
import com.nickalie.ngterm.core.ffi.RemoteAskChoice
import com.nickalie.ngterm.core.ffi.RemoteAskChoices
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * An ask for the tests: [buttons] by id (labelled the same), the first the default and `no` the cancel button, and an
 * optional form; anything else through `copy`.
 */
fun testAsk(
    vararg buttons: String = arrayOf("yes", "no"),
    input: RemoteAskInput? = null,
    choices: RemoteAskChoices? = null,
) = RemoteAsk(
    id = "A1",
    title = "Deploy to production?",
    message = null,
    buttons = buttons.map { RemoteAskButton(it, it, null) },
    defaultId = buttons.firstOrNull(),
    destructiveId = null,
    cancelId = "no",
    style = "terminal",
    align = "right",
    width = null,
    pane = null,
    input = input,
    choices = choices,
)

/** A choice list of a, b and c. */
fun testChoices(
    multiple: Boolean = false,
    checked: List<String> = emptyList(),
    other: String? = null,
) = RemoteAskChoices(
    choices = listOf("a", "b", "c").map { RemoteAskChoice(it, it.uppercase(), null) },
    multiple = multiple,
    checked = checked,
    other = other,
)

/** The draft and the replies follow ngterm-core's `AskDraft` and `PendingAsk::answer`. */
class AskDraftTest {
    @Test
    fun aRadioListStartsOnItsFirstCheckedOrFirstItem() {
        assertEquals(setOf("a"), AskDraft.of(testAsk(choices = testChoices())).checked)
        assertEquals(setOf("b"), AskDraft.of(testAsk(choices = testChoices(checked = listOf("b", "c")))).checked)
        assertEquals(
            setOf("b", "c"),
            AskDraft.of(testAsk(choices = testChoices(multiple = true, checked = listOf("b", "c")))).checked,
        )
        assertEquals(emptySet<String>(), AskDraft.of(testAsk(choices = testChoices(multiple = true))).checked)
    }

    @Test
    fun theFreeFormRowTakesARadioPickAndGivesItBackWhenBlank() {
        val ask = testAsk(choices = testChoices(checked = listOf("b"), other = "Something else"))
        val typed = AskDraft.of(ask).withText("d")
        assertTrue(typed.otherSelected)
        assertEquals(emptyList<String>(), typed.selected)
        assertEquals("d", typed.otherText)
        val cleared = typed.withText("  ")
        assertFalse(cleared.otherSelected)
        assertEquals(listOf("b"), cleared.selected)
        assertNull(cleared.otherText)
    }

    @Test
    fun theFreeFormRowTogglesOnlyWithText() {
        val ask = testAsk(choices = testChoices(multiple = true, other = "Other"))
        val blank = AskDraft.of(ask)
        assertEquals(blank, blank.toggle(3))
        val typed = blank.withText("x")
        assertTrue(typed.otherSelected)
        assertFalse(typed.toggle(3).otherSelected)
        assertTrue(typed.toggle(3).toggle(3).otherSelected)
    }

    @Test
    fun aCheckboxFlipsAndARadioPicks() {
        val multiple = AskDraft.of(testAsk(choices = testChoices(multiple = true)))
        assertEquals(listOf("a", "c"), multiple.toggle(2).toggle(0).selected)
        assertEquals(
            listOf("c"),
            multiple
                .toggle(2)
                .toggle(0)
                .toggle(0)
                .selected,
        )
        val radio = AskDraft.of(testAsk(choices = testChoices(other = "Other"))).withText("x")
        val picked = radio.toggle(1)
        assertEquals(listOf("b"), picked.selected)
        assertFalse(picked.otherSelected)
        assertEquals(picked, picked.toggle(9))
    }

    @Test
    fun aButtonAnswersWithTheFormAndTheCancelButtonEscapes() {
        val plain = testAsk()
        assertEquals(AskReply("yes", null, null), askReply(plain, "yes"))
        assertEquals(EscapedReply, askReply(plain, "no"))

        val input = testAsk(input = RemoteAskInput(false, "main", null))
        assertEquals(AskReply("yes", "main", null), askReply(input, "yes"))
        assertEquals(AskReply("yes", "dev", null), askReply(input, "yes", AskDraft.of(input).withText("dev")))
        assertEquals(EscapedReply, askReply(input, "no", AskDraft.of(input).withText("dev")))

        val list = testAsk(choices = testChoices(multiple = true, other = "Other"))
        val draft = AskDraft.of(list).toggle(1).withText("d")
        assertEquals(AskReply("yes", "d", listOf("b")), askReply(list, "yes", draft))
    }

    @Test
    fun textKeepsToItsFieldsRules() {
        assertTrue(hasForbiddenControls("a\nb", multiline = false))
        assertFalse(hasForbiddenControls("a\n\tb", multiline = true))
        assertTrue(hasForbiddenControls("a\u0007", multiline = true))
        assertTrue(hasForbiddenControls("a\u007f", multiline = true))
        assertEquals("a b", fitAskText("a\nb", multiline = false))
        assertEquals("a\nb", fitAskText("a\nb", multiline = true))
        val long = "é".repeat(ASK_TEXT_MAX_BYTES)
        val clamped = clampAskText(long)
        assertEquals(ASK_TEXT_MAX_BYTES, clamped.toByteArray().size)
        assertEquals("ok", clampAskText("ok"))
    }
}
