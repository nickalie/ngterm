package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.DARK
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.LIGHT
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.SESSION
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.SETTLE_MS
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A program that asks for the color scheme (`CSI ? 996 n`) gets the app's light or dark, and with mode 2031 on, a
 * flip of the system's appearance is reported to it.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class ColorSchemeE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aProgramGetsTheAppsAppearanceAndItsFlip() =
        with(driver) {
            openANewSession(SESSION)
            run(
                "stty raw -echo; printf '\\033[?996n'; r=\$(dd bs=1 count=9 2>/dev/null); stty sane\n" +
                    "printf '%s:%s\\n' asked \"\$r\" | tr -d '\\033'\n",
            )
            awaitTerminal("asked:$LIGHT")
            screenshot("scheme-light")

            run(
                "stty raw -echo; printf '\\033[?2031h%s%s\\r\\n' arm ed; r=\$(dd bs=1 count=9 2>/dev/null)\n" +
                    "printf '\\033[?2031l'; stty sane; printf '%s%s:%s\\n' flip ped \"\$r\" | tr -d '\\033'\n",
            )
            awaitTerminal("armed")
            appearance(night = true)
            awaitTerminal("flipped:$DARK")
            screenshot("scheme-flipped")

            // the flip's configuration change closes the soft keyboard: a tap gives the terminal the focus again
            tapTheTopLeftCell()
            Thread.sleep(SETTLE_MS)
            run(
                "stty raw -echo; printf '\\033[?996n'; r=\$(dd bs=1 count=9 2>/dev/null); stty sane\n" +
                    "printf '%s:%s\\n' again \"\$r\" | tr -d '\\033'\n",
            )
            awaitTerminal("again:$DARK")
        }
}
