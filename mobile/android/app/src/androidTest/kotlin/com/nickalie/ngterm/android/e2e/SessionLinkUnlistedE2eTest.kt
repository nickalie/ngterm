package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.LINKED
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.SESSION
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A tap on a link to a session that the machine made after the phone's last listing opens that session.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class SessionLinkUnlistedE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aTapOnALinkToASessionMadeAfterTheListingOpensIt() =
        with(driver) {
            openANewSession(SESSION)
            printALinkToANewSession()
            tapTheTopLeftCell()
            val opened = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { nodes(more(LINKED)) > 0 } }.isSuccess
            screenshot("link-unlisted-tap")
            assertTrue("a tap on a link to a session made after the listing did not open it", opened)
        }
}
