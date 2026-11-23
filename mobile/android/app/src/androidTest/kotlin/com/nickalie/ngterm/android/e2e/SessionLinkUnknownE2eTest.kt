package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.ASK_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.UNKNOWN
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A tap on a link to a session the machine does not list does the usual tap, and the view stays on the pane.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class SessionLinkUnknownE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aTapOnALinkToASessionTheMachineDoesNotListStaysInThePane() =
        with(driver) {
            openANewSession(session)
            run(
                "printf '\\033[H\\033[2J'\n" +
                    "for i in 1 2 3 4; do printf '\\033]8;;ngterm://session/%s\\007%s\\033]8;;\\007\\n' " +
                    "$UNKNOWN unknown-session-link-here; done\n" +
                    "printf '%s-%s\\n' links ready\n" +
                    "sleep 300\n",
            )
            awaitTerminal("links-ready")
            tapTheTopLeftCell()
            val moved = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(more(session)) == 0 } }.isSuccess
            screenshot("unknown-tapped")
            assertTrue("a tap on an unknown session's link left the session", !moved)
            assertTrue(terminalText().contains("links-ready"))
        }
}
