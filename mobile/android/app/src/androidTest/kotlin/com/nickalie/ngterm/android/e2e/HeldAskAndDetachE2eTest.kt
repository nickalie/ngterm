package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.PickDriving.assertNotShown
import com.nickalie.ngterm.android.e2e.PickDriving.awaitGone
import com.nickalie.ngterm.android.e2e.PickDriving.leaveTheSessionView
import com.nickalie.ngterm.android.e2e.PickDriving.openFromSessions
import com.nickalie.ngterm.android.e2e.PickDriving.runWithHelpers
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * What a headless origin holds for the next phone, and what a detach does to a pick the phone holds. An ask and a pick
 * opened on a session no viewer presents both come to the phone that opens the session (one pending batch): the ask's
 * sheet first, then the pick's. A pick that opens while the phone holds the session off screen, which the phone then
 * detaches, is not answered by the detach: it shows again when the phone opens the session again, and its answer
 * reaches the origin.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class HeldAskAndDetachE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aHeldAskAndPickComeInOneBatchAndADetachLeavesAPickUnanswered() =
        with(driver) {
            openANewSession(session)
            runWithHelpers(script(linked))
            awaitTerminal("held-ready")

            // the held ask and pick come with the session: the ask first
            leaveTheSessionView()
            openFromMachine(linked)
            awaitText("QaHeldAsk")
            assertNotShown("QaHeldPick")
            screenshot("held-ask-first")
            compose.onNodeWithText("Proceed").performClick()
            awaitText("QaHeldPick")
            screenshot("held-pick-second")
            compose.onNodeWithText("beta").performClick()
            awaitGone("QaHeldPick")

            // the next pick opens while the session is attached and off screen; the phone then detaches it
            leaveTheSessionView()
            openFromSessions(session)
            awaitTerminal("answered-held")
            awaitTerminal("detach-open")
            leaveTheSessionView()
            assertNotShown("QaDetach")
            compose.onAllNodes(more(linked))[0].performClick()
            awaitText(text(R.string.machine_sessions_detach))
            screenshot("before-detach")
            compose.onNodeWithText(text(R.string.machine_sessions_detach)).performClick()
            compose.waitForIdle()
            Thread.sleep(DETACH_SETTLE_MS)
            screenshot("after-detach")

            // the origin still waits on it: it comes again with the session
            openFromMachine(linked)
            awaitText("QaDetach")
            screenshot("pick-after-detach")
            compose.onNodeWithText("gamma").performClick()
            awaitGone("QaDetach")

            leaveTheSessionView()
            openFromSessions(session)
            awaitTerminal("answered-detach")
        }

    private fun SessionLinkDriver.openFromMachine(name: String) {
        compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(name)
        compose.onAllNodesWithText(name)[0].performClick()
    }

    private companion object {
        // a person reads the list before opening the session again
        const val DETACH_SETTLE_MS = 5_000L

        // makes [name], which no viewer presents, opens an ask and a pick on it, and prints how each was answered;
        // then opens a third pick on it once the phone has left it (the 10 s are its time to leave) and prints that
        // it is open; each wait ends on an outcome, never on "pending"
        fun script(name: String): String =
            "id=\$(ngtermctl session new --no-select --name $name)\n" +
                "a=\$(ngtermctl ask QaHeldAsk --no-block --button qaok=Proceed --target \"\$id\" | u)\n" +
                "k=\$(ngtermctl pick --no-block --prompt QaHeldPick --session \"\$id\" <qa-items.json | u)\n" +
                "m held ready\n" +
                "w ask \"\$a\" qa-ask.json qaok; w pick \"\$k\" qa-pick.json picked\n" +
                "grep -q '\"beta\"' qa-pick.json && m answered held\n" +
                "sleep 10\n" +
                "l=\$(ngtermctl pick --no-block --prompt QaDetach --session \"\$id\" <qa-items.json | u)\n" +
                "m detach open\n" +
                "w pick \"\$l\" qa-last.json '\"result\":\"\\(picked\\|cancelled\\)'\n" +
                "grep -q '\"gamma\"' qa-last.json && m answered detach\n" +
                "sleep 300\n"
    }
}
