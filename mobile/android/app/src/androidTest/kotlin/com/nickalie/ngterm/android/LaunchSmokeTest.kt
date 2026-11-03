package com.nickalie.ngterm.android

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.core.ffi.coreVersion
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import java.io.File

/**
 * The app launches on a device with no key, makes one through the mobile core's native library (JNA) and seals it with
 * the Android Keystore, shows its public half, and then shows the core's version under Settings.
 */
@RunWith(AndroidJUnit4::class)
class LaunchSmokeTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    // a fresh install, no key: the first run shows. Cleared before the compose rule starts the activity.
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = InstrumentationRegistry.getInstrumentation().targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(freshInstall).around(compose)

    private fun awaitText(text: String) =
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }

    @Test
    fun firstRunMakesAKeyThenShowsTheCoreVersion() {
        val version = coreVersion()
        assertTrue("the core reports a version", version.isNotBlank())
        awaitText("Create a key")
        compose.onNodeWithText("Create a key").performClick()
        awaitText("Add this key to your machines")
        compose
            .onAllNodesWithText("ssh-ed25519 ", substring = true)
            .fetchSemanticsNodes()
            .let { assertTrue("the public key line shows", it.isNotEmpty()) }
        compose.onNodeWithText("Done").performScrollTo().performClick()
        awaitText("Settings")
        compose.onNodeWithText("Settings").performClick()
        compose.onNodeWithText("core $version").assertIsDisplayed()
    }

    private companion object {
        const val WAIT_MS = 15_000L
    }
}
