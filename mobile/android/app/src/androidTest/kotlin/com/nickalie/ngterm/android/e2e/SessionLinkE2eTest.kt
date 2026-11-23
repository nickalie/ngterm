package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.LINKED
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.SESSION
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A tap on an OSC 8 link `ngterm://session/<id>` in a session's pane opens that session, attached first since the
 * phone has not attached it.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class SessionLinkE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aTapOnASessionLinkOpensThatSessionAttachedFirst() =
        with(driver) {
            openANewSession(SESSION)
            printALinkToANewSession()
            // the machine's sessions screen lists the machine again; the session then opens from the Sessions list
            compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
            compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
            awaitText(MACHINE)
            compose.onNodeWithText(MACHINE).performClick()
            awaitText(LINKED)
            screenshot("link-listed")
            compose.onAllNodesWithText(text(R.string.nav_sessions))[0].performClick()
            awaitText(SESSION)
            assertEquals("the phone attached the linked session before the tap", 0, nodes(hasText(LINKED)))
            compose.onNodeWithText(SESSION).performClick()
            awaitTerminal("links-ready")
            tapTheTopLeftCell()
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(more(LINKED)) > 0 }
            // the machine's own terminal of the new session leads it, so its pane shows behind Take control
            awaitText(text(R.string.session_take_control))
            screenshot("linked-opened")
            assertEquals("the linked session's bar is not the one shown", 0, nodes(more(SESSION)))
            compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
            awaitText(LINKED)
            screenshot("linked-attached")
        }
}
