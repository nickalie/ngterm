package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.e2e.PickDriving.assertNotShown
import com.nickalie.ngterm.android.e2e.PickDriving.leaveTheSessionView
import com.nickalie.ngterm.android.e2e.PickDriving.openFromSessions
import com.nickalie.ngterm.android.e2e.PickDriving.runWithHelpers
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * When a pick waits on the phone: while an ask of its session is open, the ask's sheet comes first and the pick's
 * shows once the ask is answered; while its session is not on screen, nothing shows and no notification is posted,
 * and the sheet shows when the session is opened. `ngtermctl` on the origin gets each answer.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class PickWaitsE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aPickWaitsForTheAskOfItsSessionAndForItsSessionOnScreen() =
        with(driver) {
            openANewSession(session)
            runWithHelpers(SCRIPT)

            awaitText("QaFirstAsk")
            // the pick opened two seconds after the ask: it waits under the ask's sheet
            Thread.sleep(PICK_OPENED_MS)
            assertNotShown("QaSecondPick")
            screenshot("ask-before-pick")
            compose.onNodeWithText("Proceed").performClick()
            awaitText("QaSecondPick")
            screenshot("pick-after-ask")
            compose.onNodeWithText("beta").performClick()
            awaitTerminal("answered-both")

            // the next pick opens while the phone shows the session list
            leaveTheSessionView()
            Thread.sleep(OFF_SCREEN_MS)
            assertNotShown("QaOffScreen")
            screenshot("pick-off-screen")
            val notifications = shell("dumpsys notification --noredact")
            assertFalse("a notification for the pick:\n$notifications", "QaOffScreen" in notifications)

            openFromSessions(session)
            awaitText("QaOffScreen")
            screenshot("pick-on-return")
            compose.onNodeWithText("gamma").performClick()
            awaitTerminal("answered-offscreen")
        }

    private companion object {
        const val PICK_OPENED_MS = 6_000L

        // the script opens its pick 15 s after it printed its marker; the phone looks 10 s after that
        const val OFF_SCREEN_MS = 25_000L

        const val SCRIPT =
            "ngtermctl ask QaFirstAsk --button qaok=Proceed --target \"\$NGTERM_SESSION_ID\" >qa-ask.json &\n" +
                "sleep 2\n" +
                "p --prompt QaSecondPick >qa-pick.json &\n" +
                "wait\n" +
                "grep -q qaok qa-ask.json && grep -q '\"beta\"' qa-pick.json && m answered both\n" +
                "sleep 15\n" +
                "p --prompt QaOffScreen | grep -q '\"gamma\"' && m answered offscreen\n" +
                "sleep 300\n"
    }
}
