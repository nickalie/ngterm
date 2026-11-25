package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.PickDriving.WRITE_ITEMS
import com.nickalie.ngterm.android.e2e.PickDriving.mark
import com.nickalie.ngterm.android.e2e.PickDriving.searchField
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * The pick's sheet as the phone shows it: the prompt as the heading ("Pick one" without one), an item's subtitle and
 * its status glyph (none for idle), the search field that filters the rows, `Use "…"` for a pick that allows a custom
 * answer, "No matches" for one that does not, and Cancel. `ngtermctl pick` on the origin gets the custom text and the
 * cancel.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class PickSheetE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun theSheetShowsItsItemsFiltersThemAndAnswersWithACustomTextOrACancel() =
        with(driver) {
            openANewSession(session)
            run(SCRIPT)
            awaitText(PROMPT)
            awaitText("qa-subtitle")
            // a glyph for active and blocked, which TalkBack reads as its word; none for idle
            assertEquals(1, nodes(hasContentDescription(text(R.string.agent_active))))
            assertEquals(1, nodes(hasContentDescription(text(R.string.agent_blocked))))
            assertEquals(0, nodes(hasContentDescription(text(R.string.status_idle))))
            screenshot("pick-sheet")

            compose.onNode(searchField).performTextInput("gam")
            awaitText("gamma")
            PickDriving.run { assertNotShown("alpha") }
            compose.onNode(searchField).performTextInput("zzq")
            awaitText(CUSTOM_ROW)
            screenshot("pick-custom")
            compose.onNodeWithText(CUSTOM_ROW).performClick()
            awaitTerminal("answered-custom")

            // the second pick has no prompt and no custom answer
            awaitText(text(R.string.pick_title))
            compose.onNode(searchField).performTextInput("zzq")
            awaitText(text(R.string.pick_no_matches))
            screenshot("pick-no-matches")
            compose.onNodeWithText(text(R.string.pick_cancel)).performClick()
            awaitTerminal("answered-cancel")
            PickDriving.run { assertNotShown(text(R.string.pick_title)) }
        }

    private companion object {
        const val PROMPT = "QaSheet"
        const val CUSTOM_ROW = "Use \"gamzzq\""

        val SCRIPT =
            WRITE_ITEMS +
                "ngtermctl pick --prompt $PROMPT --allow-custom --session \"\$NGTERM_SESSION_ID\" " +
                "<qa-items.json >qa-1.json\n" +
                "grep -q '\"custom\"' qa-1.json && grep -q gamzzq qa-1.json && " + mark("answered", "custom") +
                "ngtermctl pick --session \"\$NGTERM_SESSION_ID\" <qa-items.json >qa-2.json\n" +
                "grep -q cancelled qa-2.json && " + mark("answered", "cancel") +
                "cat qa-1.json qa-2.json\n" +
                "sleep 300\n"
    }
}
