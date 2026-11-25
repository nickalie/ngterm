package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import com.nickalie.ngterm.android.e2e.PickDriving.awaitGone
import com.nickalie.ngterm.android.e2e.PickDriving.runWithHelpers
import com.nickalie.ngterm.android.e2e.PickDriving.searchField
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * How a pick's sheet ends on the phone and what `ngtermctl` on the origin gets: the keyboard's Go answers with the
 * `--select` item; Back and a swipe down cancel; a pick and an ask whose `--timeout` runs out on the origin close on
 * the phone with no answer from it.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class PickAnswerKeysE2eTest {
    private val driver = SessionLinkDriver()
    private val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun goAnswersWithTheSelectionBackAndASwipeCancelAndATimeoutClosesTheSheet() =
        with(driver) {
            openANewSession(session)
            runWithHelpers(SCRIPT)

            // Go on the keyboard, the field focused as a tap focuses it
            awaitText("QaGo")
            compose.onNode(searchField).performClick()
            compose.waitForIdle()
            shell("input keyevent KEYCODE_ENTER")
            awaitGone("QaGo")
            awaitTerminal("answered-go")

            awaitText("QaBack")
            device.pressBack()
            awaitGone("QaBack")
            awaitTerminal("answered-back")

            awaitText("QaSwipe")
            val heading = compose.onNodeWithText("QaSwipe").fetchSemanticsNode()
            val x = heading.positionOnScreen.x.toInt() + heading.size.width / 2
            val y = heading.positionOnScreen.y.toInt() + heading.size.height / 2
            device.swipe(x, y, x, device.displayHeight - 1, SWIPE_STEPS)
            awaitGone("QaSwipe")
            awaitTerminal("answered-swipe")

            // the origin's timeouts: the phone shows each, then takes it down unanswered
            awaitText("QaPickTimeout")
            screenshot("pick-before-timeout")
            awaitGone("QaPickTimeout")
            awaitTerminal("answered-picktimeout")
            awaitText("QaAskTimeout")
            screenshot("ask-before-timeout")
            awaitGone("QaAskTimeout")
            awaitTerminal("answered-asktimeout")
            screenshot("after-timeouts")
        }

    private companion object {
        // about 100 ms from the heading to the screen's bottom: a fling, as a finger flicks a sheet away
        const val SWIPE_STEPS = 20

        const val SCRIPT =
            "p --prompt QaGo --select beta | grep -q '\"beta\"' && m answered go\n" +
                "p --prompt QaBack | grep -q cancelled && m answered back\n" +
                "p --prompt QaSwipe | grep -q cancelled && m answered swipe\n" +
                "p --prompt QaPickTimeout --timeout 10 | grep -q cancelled && m answered picktimeout\n" +
                "ngtermctl ask QaAskTimeout --button qaok=Proceed --timeout 10 --target \"\$NGTERM_SESSION_ID\" " +
                "| grep -q cancelled && m answered asktimeout\n" +
                "sleep 300\n"
    }
}
