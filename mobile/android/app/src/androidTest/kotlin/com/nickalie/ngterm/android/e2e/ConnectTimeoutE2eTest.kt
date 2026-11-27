package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.DEFAULT_SECONDS
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.LOOPBACK
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.OWN_MACHINE
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.OWN_PORT
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.OWN_SECONDS
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.SETTINGS_MACHINE
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.SETTINGS_PORT
import com.nickalie.ngterm.android.e2e.ConnectTimeoutDriver.Companion.SETTINGS_SECONDS
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * The connect timeout from a fresh install, end to end: a machine that never answers gives up after 10 s, then after
 * the Settings value once Connection timeout moves, and a machine with its own value gives up after that one. A real
 * machine still connects.
 *
 * [ConnectTimeoutKeptE2eTest] then starts the app again over what this test stored. Run the two in one script run
 * with the app kept installed between them:
 *
 * ```
 * GRADLE_OPTS="-Dorg.gradle.project.android.injected.androidTest.leaveApksInstalledAfterRun=true \
 *   -Dorg.gradle.project.android.testInstrumentationRunnerArguments.e2eConnectTimeoutKept=true" \
 *   scripts/test-android-e2e.sh ConnectTimeoutE2eTest ConnectTimeoutKeptE2eTest
 * ```
 */
@RunWith(AndroidJUnit4::class)
class ConnectTimeoutE2eTest {
    private val driver = ConnectTimeoutDriver(fresh = true)

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aMachineGivesUpAfterTheSettingsValueOrItsOwn() =
        with(driver) {
            signIn()
            fillNewMachine(SETTINGS_MACHINE, LOOPBACK, SETTINGS_PORT, timeout = "")
            assertTimeoutHint(DEFAULT_SECONDS)
            save()
            assertGaveUpAfter(DEFAULT_SECONDS, secondsToGiveUp(SETTINGS_MACHINE, settingsHole, 1), "the default")
            screenshot("gave-up-default")

            openSettings()
            timeoutSlider()
            awaitText(text(R.string.settings_connect_timeout_value, DEFAULT_SECONDS))
            screenshot("settings-default")
            setSettingsTimeout(SETTINGS_SECONDS)
            screenshot("settings-changed")

            openMachines()
            retry(SETTINGS_MACHINE)
            assertGaveUpAfter(SETTINGS_SECONDS, secondsToGiveUp(SETTINGS_MACHINE, settingsHole, 2), "Settings' value")

            fillNewMachine(OWN_MACHINE, LOOPBACK, OWN_PORT, timeout = OWN_SECONDS.toString())
            assertTimeoutHint(SETTINGS_SECONDS)
            screenshot("machine-form-own")
            save()
            assertGaveUpAfter(OWN_SECONDS, secondsToGiveUp(OWN_MACHINE, ownHole, 1), "the machine's own value")
            screenshot("gave-up-own")

            addRealMachine()
            screenshot("real-connected")

            openEditor(OWN_MACHINE)
            assertEquals("the own machine's stored timeout", OWN_SECONDS.toString(), timeoutFieldValue())
            screenshot("machine-form-edit-own")
            closeEditor()
            openEditor(SETTINGS_MACHINE)
            assertEquals("the Settings machine's stored timeout", "", timeoutFieldValue())
            assertTimeoutHint(SETTINGS_SECONDS)
            closeEditor()
        }
}
