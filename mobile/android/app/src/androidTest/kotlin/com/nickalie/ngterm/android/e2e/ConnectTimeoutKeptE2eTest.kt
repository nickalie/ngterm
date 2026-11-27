package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.KEPT
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.OWN_MACHINE
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.OWN_SECONDS
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.REAL
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.SETTINGS_MACHINE
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.SETTINGS_SECONDS
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * The connect timeouts survive a restart: in a new app process over what [ConnectTimeoutE2eTest] stored, the
 * machines connect at launch and give up after the stored Settings value and the machine's own value, the real
 * machine connects, and Settings and the editor show the stored values.
 *
 * It is skipped unless the run sets [KEPT] to `true`. Set it only for the run of [ConnectTimeoutE2eTest]'s doc, which
 * runs that test first and keeps the app installed: this test needs what that test stored.
 */
@RunWith(AndroidJUnit4::class)
class ConnectTimeoutKeptE2eTest {
    private val kept =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "run after ConnectTimeoutE2eTest with the app kept installed",
                    InstrumentationRegistry.getArguments().getString(KEPT) == "true",
                )
            }
        }

    private val driver = ConnectTimeoutDriver(fresh = false, extra = kept)

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun theStoredTimeoutsApplyFromLaunch() =
        with(driver) {
            assertGaveUpAfter(SETTINGS_SECONDS, secondsToGiveUp(SETTINGS_MACHINE, settingsHole, 1), "Settings' value")
            assertGaveUpAfter(OWN_SECONDS, secondsToGiveUp(OWN_MACHINE, ownHole, 1), "the machine's own value")
            awaitConnected(REAL)
            screenshot("kept-machines")

            openSettings()
            timeoutSlider()
            awaitText(text(R.string.settings_connect_timeout_value, SETTINGS_SECONDS))
            screenshot("kept-settings")

            openMachines()
            openEditor(OWN_MACHINE)
            assertEquals("the own machine's stored timeout", OWN_SECONDS.toString(), timeoutFieldValue())
            assertTimeoutHint(SETTINGS_SECONDS)
            screenshot("kept-machine-form")
            closeEditor()
        }
}
