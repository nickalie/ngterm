package com.nickalie.ngterm.android.ui.settings

import android.app.NotificationManager
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.FAKE_PUBLIC_KEY
import com.nickalie.ngterm.android.PLAIN_FAKE_KEY
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Settings navigation and its real model: keys are made/imported, shown, and deliberately adopted by machines. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp-mdpi")
class SettingsRouteTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private lateinit var test: TestGraph

    @Before fun setUp() {
        test = TestGraph(dir.root)
    }

    @After fun tearDown() = test.close()

    private fun show() {
        compose.setContent { NgtermTheme { SettingsRoute(test.graph, "settings-test") } }
        compose.awaitText("core settings-test")
    }

    private fun tap(text: String) = compose.onNodeWithText(text).performScrollTo().performClick()

    private fun back() {
        compose.waitForIdle()
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
    }

    @Test
    fun anExistingPublicKeyOpensAndBothBackActionsReturnToSettings() {
        test.seedKey()
        show()
        tap("Show public key")
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertExists()
        compose.onNodeWithText("Use for every machine").assertDoesNotExist()
        compose.onNodeWithText("Every machine signs in with this key.").assertDoesNotExist()
        compose.onNodeWithContentDescription("Back").performClick()
        compose.awaitText("core settings-test")
        tap("Show public key")
        back()
        compose.awaitText("core settings-test")
    }

    @Test
    fun rotatingShowsTheNewKeyAndOnlySwitchesMachinesWhenAsked() {
        val old = test.seedKey()
        test.seedMachine(testMachine(keyId = old.id))
        show()
        tap("Rotate key")
        compose.awaitText("Use for every machine")
        assertEquals(
            old.id,
            runBlocking {
                test.graph.machines.machines
                    .first()
                    .single()
                    .keyId
            },
        )
        tap("Use for every machine")
        compose.awaitText("Every machine signs in with this key.")
        val new =
            runBlocking {
                test.graph.keys.keys
                    .first()
                    .last()
            }
        assertTrue(new.id != old.id)
        assertEquals(
            new.id,
            runBlocking {
                test.graph.machines.machines
                    .first()
                    .single()
                    .keyId
            },
        )
        back()
        compose.awaitText("core settings-test")
    }

    @Test
    fun importRefusesAnEmptyKeyThenShowsItsPublicHalfAfterAValidPaste() {
        show()
        tap("Import a key")
        tap("Import key")
        compose.awaitText("Paste a private key or choose its file.")
        compose.onNodeWithText("Private key").performTextInput(PLAIN_FAKE_KEY)
        tap("Import key")
        compose.awaitText(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForScreenshotsOnlyNotARealKey0000000000 imported",
        )
        assertTrue(
            runBlocking {
                test.graph.keys.keys
                    .first()
                    .single()
                    .imported
            },
        )
        back()
        compose.awaitText("core settings-test")
        tap("Import a key")
        back()
        compose.awaitText("core settings-test")
    }

    @Test
    fun notificationSettingsOpenForThisAppAndPermissionIsReadAgainOnResume() {
        val manager = compose.activity.getSystemService(NotificationManager::class.java)
        shadowOf(manager).setNotificationsEnabled(false)
        show()
        tap("Turn on")
        val intent = shadowOf(compose.activity).nextStartedActivity
        assertEquals(Settings.ACTION_APP_NOTIFICATION_SETTINGS, intent.action)
        assertEquals(compose.activity.packageName, intent.getStringExtra(Settings.EXTRA_APP_PACKAGE))
        compose.activityRule.scenario.moveToState(Lifecycle.State.STARTED)
        shadowOf(manager).setNotificationsEnabled(true)
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.onNodeWithText("Turn on").assertDoesNotExist()
    }

    @Test
    fun notificationAndClipboardControlsChangeTheirStores() {
        test.seedMachine(testMachine())
        runBlocking { test.trust.allow("m1") }
        show()
        compose
            .onNodeWithText("Finished")
            .performScrollTo()
            .assertIsOn()
            .performClick()
        compose.waitUntil(10_000) {
            compose.waitForIdle()
            runBlocking { com.nickalie.ngterm.android.data.AlertKind.Done !in test.notificationPrefs.enabled.first() }
        }
        compose.onNodeWithText("Finished").assertIsOff()
        compose
            .onNodeWithContentDescription(
                "Ask again before build-01 reads the clipboard",
            ).performScrollTo()
            .performClick()
        compose.awaitText("Every machine asks before its programs read your clipboard.")
        assertFalse(runBlocking { "m1" in test.trust.machines.first() })
    }
}
