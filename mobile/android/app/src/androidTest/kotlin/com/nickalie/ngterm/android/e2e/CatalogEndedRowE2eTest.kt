package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isNotEnabled
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.CatalogDriving.SHOWS
import com.nickalie.ngterm.android.e2e.CatalogDriving.awaitRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.openTheMachine
import com.nickalie.ngterm.android.e2e.CatalogDriving.renameOnThePhone
import com.nickalie.ngterm.android.e2e.CatalogDriving.showsRow
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A session that ends on a headless machine stays in its sessions list, dimmed, with "Ended on MACHINE", and a tap on
 * it opens nothing; the machine's card counts it as ended.
 *
 * The machine's live-session server is killed once its tree shows a rename that the phone made from the open list:
 * ngterm-headless then keeps each session whose live session was lost as ended, as upstream does. Only this run's
 * server goes: the one of this shell's live namespace.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class CatalogEndedRowE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun anEndedSessionStaysDimmedAndATapOpensNothing() =
        with(driver) {
            val signal = "$session-signal"
            val go = "$session-go"
            val ending = "$session-ending"
            openANewSession(session)
            run(
                "ngtermctl session new --no-select --name $signal >/dev/null\n" +
                    "ngtermctl session new --no-select --name $ending >/dev/null\n" +
                    SHOWS +
                    "(shows $go && pkill -KILL -fx \"\$(command -v ngterm-session) server \$NGTERM_LIVE_DIR\") " +
                    ">/dev/null 2>&1 &\n" +
                    "echo catalog-armed\n",
            )
            awaitTerminal("catalog-armed")
            openTheMachine()
            awaitRow(ending)

            renameOnThePhone(signal, go)
            val ended = hasText(ending) and isNotEnabled()
            // the badge reads its label as one content description
            val label = hasContentDescription(text(R.string.machine_sessions_ended, MACHINE))
            val shown = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { showsRow(ended) } }.isSuccess
            assertTrue("the row $ending never read as ended", shown)
            assertTrue("no row says ${text(R.string.machine_sessions_ended, MACHINE)}", nodes(label) > 0)
            screenshot("catalog-ended")

            compose.onNode(ended).performClick()
            Thread.sleep(OPEN_WAIT_MS)
            assertEquals("a tap on an ended row opened a session", 0, nodes(terminal))
            assertTrue("the list went away after a tap on an ended row", showsRow(hasText(ending)))

            compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
            val counted = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { nodes(endedCount) > 0 } }.isSuccess
            assertTrue("the machine's card never counted its ended sessions", counted)
            screenshot("catalog-ended-card")
        }

    private val SessionLinkDriver.terminal: SemanticsMatcher
        get() = hasContentDescription(text(R.string.terminal_description))

    // "N ended" on the machine's card, whatever N the machine's earlier sessions make it; a badge's content description
    private val endedCount: SemanticsMatcher
        get() {
            val resources = InstrumentationRegistry.getInstrumentation().targetContext.resources
            val sample = resources.getQuantityString(R.plurals.machine_ended_sessions, SAMPLE, SAMPLE)
            val count = Regex(sample.split("$SAMPLE").joinToString("\\d+") { Regex.escape(it) })
            return SemanticsMatcher("a count of ended sessions") { node ->
                node.config
                    .getOrNull(SemanticsProperties.ContentDescription)
                    .orEmpty()
                    .any { count.matches(it) }
            }
        }

    private companion object {
        /** Long enough for a tap's attach to show its terminal, if it started one. */
        const val OPEN_WAIT_MS = 3_000L

        /** A count the plural's "other" form takes. */
        const val SAMPLE = 7
    }
}
