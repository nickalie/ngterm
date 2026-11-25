package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.PickDriving.leaveTheSessionView
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A pick a headless origin opens on a session no viewer presents waits there, and the phone that takes the session
 * next is handed it in the origin's pending batch: the sheet shows as the phone opens the session, and a tap answers
 * the pick, which `ngtermctl pick result` then reads on the origin.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class HeldPickE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aPickOpenedWithNoPresenterIsHandedToThePhoneThatAttachesNext() =
        with(driver) {
            openANewSession(session)
            run(heldPickScript(linked))
            awaitTerminal(HELD_READY)

            // the session the pick waits on opens from the machine's list: the held pick comes with it
            leaveTheSessionView()
            compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
            awaitText(MACHINE)
            compose.onNodeWithText(MACHINE).performClick()
            awaitText(linked)
            compose.onAllNodesWithText(linked)[0].performClick()
            awaitText(PROMPT)
            screenshot("held-pick-shown")
            compose.onNodeWithText("beta").performClick()

            // the first session's script read the answer on the origin
            compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodesWithText(PROMPT).fetchSemanticsNodes().isEmpty() }
            leaveTheSessionView()
            compose.onAllNodesWithText(text(R.string.nav_sessions))[0].performClick()
            awaitText(session)
            compose.onAllNodesWithText(session)[0].performClick()
            awaitTerminal(SessionPickE2eTest.PICKED_BETA)
        }

    private companion object {
        const val PROMPT = "QaHeldPick"

        /** What [heldPickScript] prints once its pick waits on the origin (the script's own text does not hold it). */
        const val HELD_READY = "held-ready"

        // makes the session [name], which no viewer presents, opens a pick on it, and prints how it was answered
        fun heldPickScript(name: String): String =
            "id=\$(ngtermctl session new --no-select --name $name)\n" +
                "pick=\$(printf 'alpha\\nbeta\\n' | ngtermctl pick --no-block --prompt $PROMPT --session \"\$id\" " +
                "| grep -o '[0-9A-F-]\\{36\\}')\n" +
                "printf '%s-%s\\n' held ready\n" +
                // a resolved pick's result is read once: the read that sees it keeps it
                "until ngtermctl pick result \"\$pick\" >qa-pick.json 2>/dev/null; grep -q picked qa-pick.json; " +
                "do sleep 1; done\n" +
                "grep -q beta qa-pick.json && printf '%s-%s\\n' answered beta\n" +
                "sleep 300\n"
    }
}
