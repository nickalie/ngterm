package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.hasText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.CatalogDriving.SHOWS
import com.nickalie.ngterm.android.e2e.CatalogDriving.awaitRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.openTheMachine
import com.nickalie.ngterm.android.e2e.CatalogDriving.renameOnThePhone
import com.nickalie.ngterm.android.e2e.CatalogDriving.showsRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.toTheTop
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * While a headless machine's catalog cannot be reached, its sessions list keeps its rows under a banner that says
 * "Disconnected: REASON. Retrying."; once the machine answers again, the banner goes and the list is live again.
 *
 * The machine's ngterm-headless stops (SIGSTOP) once its tree shows a rename that the phone made from the open list,
 * so the stream goes silent while ssh stays up, and it goes on (SIGCONT) after [HELD_S] seconds. The machine then
 * makes a session, which the list can get only from the live catalog.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class CatalogBannerE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun theBannerSaysTheCatalogIsLostAndGoesWhenTheMachineAnswersAgain() =
        with(driver) {
            val signal = "$session-signal"
            val cut = "$session-cut"
            val back = "$session-back"
            openANewSession(session)
            run(
                "ngtermctl session new --no-select --name $signal >/dev/null\n" +
                    SHOWS +
                    // this run's ngterm-headless alone: the one with this shell's state dir
                    "app() { for p in \$(pgrep -x ngterm-headless); do " +
                    "grep -qzx \"NGTERM_STATE_DIR=\$NGTERM_STATE_DIR\" /proc/\$p/environ && echo \$p; done; }\n" +
                    "(shows $cut && pid=\$(app) && kill -STOP \$pid && sleep $HELD_S; kill -CONT \$pid; " +
                    "ngtermctl session new --no-select --name $back) >/dev/null 2>&1 &\n" +
                    "echo catalog-armed\n",
            )
            awaitTerminal("catalog-armed")
            openTheMachine()
            awaitRow(signal)

            renameOnThePhone(signal, cut)
            val disconnected = hasText(bannerPrefix(), substring = true)
            val shown =
                runCatching {
                    compose.waitUntil(LOST_WAIT_MS) {
                        toTheTop()
                        nodes(disconnected) > 0
                    }
                }.isSuccess
            assertTrue("the list never said its catalog was lost", shown)
            // the banner comes in above the row the list keeps in view
            toTheTop()
            compose.waitForIdle()
            screenshot("catalog-banner")
            assertTrue("the list dropped its rows while its catalog was lost", showsRow(hasText(cut)))

            compose.waitUntil(BACK_WAIT_MS) { showsRow(hasText(back)) }
            screenshot("catalog-back")
            toTheTop()
            assertTrue("the banner stayed after the catalog came back", nodes(disconnected) == 0)
        }

    // the banner's text up to its reason, which is the core's own words
    private fun SessionLinkDriver.bannerPrefix(): String =
        text(R.string.machine_sessions_catalog_disconnected, REASON_MARK).substringBefore(REASON_MARK)

    private companion object {
        /** How long the machine's ngterm-headless stays stopped: past the stream's 35 seconds of silence. */
        const val HELD_S = 75

        /** The silence limit, the stop's start and a margin. */
        const val LOST_WAIT_MS = 70_000L

        /** The rest of the stop, the core's retry delay and a fresh snapshot. */
        const val BACK_WAIT_MS = 120_000L

        const val REASON_MARK = "\u0000"
    }
}
