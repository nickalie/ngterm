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
 * An ask and a pick a headless origin opens on a session no viewer presents both come to the phone that opens the
 * session, in one pending batch: the ask's sheet first, then the pick's. `ngtermctl ask result` and `ngtermctl pick
 * result` on the origin read the phone's answers.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class HeldAskE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aHeldAskAndPickComeInOneBatchTheAskFirst() =
        with(driver) {
            openANewSession(session)
            runWithHelpers(script(linked))
            awaitTerminal("held-ready")

            leaveTheSessionView()
            compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
            awaitText(MACHINE)
            compose.onNodeWithText(MACHINE).performClick()
            awaitText(linked)
            compose.onAllNodesWithText(linked)[0].performClick()
            awaitText("QaHeldAsk")
            assertNotShown("QaHeldPick")
            screenshot("held-ask-first")
            compose.onNodeWithText("Proceed").performClick()
            awaitText("QaHeldPick")
            screenshot("held-pick-second")
            compose.onNodeWithText("beta").performClick()
            awaitGone("QaHeldPick")

            leaveTheSessionView()
            openFromSessions(session)
            awaitTerminal("answered-held")
        }

    private companion object {
        // makes [name], which no viewer presents, opens an ask and a pick on it, and prints how each was answered
        fun script(name: String): String =
            "id=\$(ngtermctl session new --no-select --name $name)\n" +
                "a=\$(ngtermctl ask QaHeldAsk --no-block --button qaok=Proceed --target \"\$id\" | u)\n" +
                "k=\$(ngtermctl pick --no-block --prompt QaHeldPick --session \"\$id\" <qa-items.json | u)\n" +
                "m held ready\n" +
                "w ask \"\$a\" qa-ask.json qaok; w pick \"\$k\" qa-pick.json picked\n" +
                "grep -q '\"beta\"' qa-pick.json && m answered held\n" +
                "sleep 300\n"
    }
}
