package com.nickalie.ngterm.android.e2e

import android.app.NotificationManager
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.NgtermApplication
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.update.AppUpdates
import com.nickalie.ngterm.android.update.SelfUpdates
import com.nickalie.ngterm.android.update.UpdateState
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith
import java.io.File

/**
 * The GitHub build's App updates switch in Settings > Notifications, end to end against GitHub's real releases (this
 * build is older than the newest one). With the switch off before the app starts, the start's check finds the new
 * version and posts no notice. The user turns the switch on and the app starts again: the notice posts, as the check
 * with the switch off did not record the version as announced. The user turns the switch off while the notice is up:
 * it goes away. Skipped in the Play build (no updates) and when GitHub can't be read or offers nothing newer.
 *
 * Run through `scripts/test-android-e2e.sh`, whose key signs the app in; no machine and no live ngterm.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.TIRAMISU)
class UpdateNotificationSwitchE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val compose = createAndroidComposeRule<MainActivity>()
    private val context get() = instrumentation.targetContext

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no key: run it through scripts/test-android-e2e.sh",
                    !arguments.getString(KEY).isNullOrBlank(),
                )
                assumeTrue("the Play build has no updates", updates() != null)
            }
        }

    /** A fresh install that may post notifications, with App updates off before the activity starts. */
    private val freshInstallUpdatesOff =
        object : ExternalResource() {
            override fun before() {
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
                shell("pm grant ${context.packageName} android.permission.POST_NOTIFICATIONS")
                manager().cancelAll()
                val updates = checkNotNull(updates())
                updates.setNotify(false)
                runBlocking { withTimeout(WAIT_MS) { updates.notify.first { !it } } }
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                screenshot("failed")
                Log.e(TAG, "update state: ${updates()?.state?.value}, notice up: ${noticeUp()}")
            }
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstallUpdatesOff)
            .around(compose)
            .around(evidence)

    @Test
    fun theAppUpdatesSwitchDecidesWhetherTheUpdateNoticeIsUp() {
        val updates = checkNotNull(updates())
        signIn()
        val found = awaitCheck(updates)
        assumeTrue("GitHub offers no newer release ($found)", found is UpdateState.Available)
        // the start's check may still be announcing; this one returns only after its own announce step
        runBlocking { (updates as SelfUpdates).checkInBackground() }
        assertFalse("the update notice is up with App updates off", noticeUp())
        screenshot("off-no-notice")

        compose.onNodeWithText(text(R.string.nav_settings)).performClick()
        setAppUpdates(ToggleableState.On)
        // the next start of the app, or the twice-a-day background check, announces again
        updates.appStarted()
        compose.waitUntil(WAIT_MS) { noticeUp() }
        screenshot("on-notice-up")

        setAppUpdates(ToggleableState.Off)
        compose.waitUntil(WAIT_MS) { !noticeUp() }
        screenshot("off-notice-down")
    }

    private fun awaitCheck(updates: AppUpdates): UpdateState {
        compose.waitUntil(WAIT_MS) {
            updates.state.value.let { it !is UpdateState.Idle && it !is UpdateState.Checking }
        }
        return updates.state.value
    }

    private fun setAppUpdates(state: ToggleableState) {
        val row = hasText(text(R.string.settings_alert_updates)) and SemanticsMatcher.keyIsDefined(TOGGLE)
        val before = if (state == ToggleableState.On) ToggleableState.Off else ToggleableState.On
        compose.onNode(row).performScrollTo()
        // the row shows On until the stored value loads
        compose.waitUntil(WAIT_MS) { toggle(row) == before }
        compose.onNode(row).performClick()
        compose.waitUntil(WAIT_MS) { toggle(row) == state }
        val stored = state == ToggleableState.On
        runBlocking { withTimeout(WAIT_MS) { checkNotNull(updates()).notify.first { it == stored } } }
    }

    private fun toggle(row: SemanticsMatcher): ToggleableState? =
        compose
            .onNode(row)
            .fetchSemanticsNode()
            .config
            .getOrNull(TOGGLE)

    private fun signIn() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.nav_settings))
    }

    private fun updates(): AppUpdates? = (context.applicationContext as NgtermApplication).graph.updates

    private fun manager() = context.getSystemService(NotificationManager::class.java)

    private fun noticeUp(): Boolean = manager().activeNotifications.any { it.tag == UPDATE_TAG }

    private fun awaitText(text: String) =
        compose.waitUntil(WAIT_MS) { compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }

    private fun text(id: Int): String = context.getString(id)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/update-notification-switch-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation argument the script passes: the client key that signs the app in. */
        const val KEY = "e2eKey"

        /** The update notice's notification tag (UpdateNotifications). */
        const val UPDATE_TAG = "update"

        val TOGGLE = SemanticsProperties.ToggleableState

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val WAIT_MS = 60_000L
    }
}
