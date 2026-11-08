package com.nickalie.ngterm.android.ui.asks

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File
import java.io.IOException

/** The permission is explained at first attachment, asked once, and its system answer updates the service. */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class NotificationPermissionPromptTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule val dir = TemporaryFolder()
    private lateinit var test: TestGraph

    @Before fun setUp() {
        test = TestGraph(dir.root, permissionAsked = false)
        (compose.activity.application as TestApplication).testGraph = test.graph
        shadowOf(compose.activity.application).denyPermissions(Manifest.permission.POST_NOTIFICATIONS)
    }

    @After fun tearDown() = test.close()

    private fun show() {
        compose.setContent { NgtermTheme { NotificationPermissionPrompt(test.graph) } }
    }

    private fun attach() {
        val key = test.seedKey()
        compose.runOnIdle { test.connections.connect(testMachine(keyId = key.id).toCore()) }
        compose.waitUntil(WAIT_MS) { "m1" in test.core.connected }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1"))
        compose.runOnIdle {
            runBlocking {
                test.connections.listSessions("m1")
                test.connections.attach("m1", "s1")
            }
        }
    }

    private fun gone() {
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithText(TITLE).fetchSemanticsNodes().isEmpty()
        }
    }

    private fun asked() = runBlocking { test.notificationPrefs.permissionAsked.first() }

    @Test
    fun noSessionMeansNoPromptAndNotNowLeavesItForSettingsEvenAfterReattaching() {
        show()
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        assertFalse(asked())
        attach()
        compose.awaitText(TITLE)
        compose.onNodeWithText("Turn on notifications").assertIsDisplayed()
        compose.onNodeWithText("Not now").performClick()
        gone()
        assertTrue(asked())
        compose.runOnIdle { test.connections.detach("m1", "s1") }
        compose.runOnIdle { runBlocking { test.connections.attach("m1", "s1") } }
        compose.onNodeWithText(TITLE).assertDoesNotExist()
    }

    @Test
    fun turningOnNotificationsRequestsTheSystemPermissionAndAGrantRestartsTheService() {
        attach()
        show()
        compose.awaitText(TITLE)
        compose.onNodeWithText("Turn on notifications").performClick()
        val request = shadowOf(compose.activity).lastRequestedPermission
        assertEquals(listOf(Manifest.permission.POST_NOTIFICATIONS), request.requestedPermissions.toList())
        compose.runOnIdle {
            shadowOf(compose.activity.application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
            compose.activity.activityResultRegistry.dispatchResult(
                request.requestCode,
                true,
            )
        }
        gone()
        assertTrue(asked())
        assertEquals(
            LinkService::class.java.name,
            shadowOf(compose.activity.application).nextStartedService.component?.className,
        )
    }

    @Test
    fun aSystemRefusalDoesNotStartTheServiceOrRepeatTheQuestion() {
        attach()
        show()
        compose.awaitText(TITLE)
        compose.onNodeWithText("Turn on notifications").performClick()
        val request = shadowOf(compose.activity).lastRequestedPermission
        compose.runOnIdle {
            compose.activity.activityResultRegistry.dispatchResult(request.requestCode, false)
        }
        gone()
        assertTrue(asked())
        assertNull(shadowOf(compose.activity.application).nextStartedService)
        assertEquals(
            PackageManager.PERMISSION_DENIED,
            compose.activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS),
        )
    }

    @Test
    fun anExistingGrantNeedsNoExplanationAndDoesNotMarkItAsked() {
        shadowOf(compose.activity.application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        attach()
        show()
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        assertFalse(asked())
    }

    @Test
    fun dismissingTheExplanationLeavesTheQuestionForSettings() {
        attach()
        show()
        compose.awaitText(TITLE)
        compose.onNodeWithContentDescription("Close sheet").performSemanticsAction(SemanticsActions.OnClick) { it() }
        compose.waitForIdle()
        gone()
        assertTrue(asked())
    }

    @Test
    fun unreadablePermissionPreferencesDoNotFlashAQuestionOverAnAttachedSession() {
        assertTrue(File(dir.root, "notifications.preferences_pb").mkdir())
        attach()
        val failure =
            runCatching {
                runBlocking(Dispatchers.IO) { withTimeout(WAIT_MS) { test.notificationPrefs.permissionAsked.first() } }
            }.exceptionOrNull()
        assertTrue(failure is IOException)
        show()
        compose.waitForIdle()
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        compose.onNodeWithText("Turn on notifications").assertDoesNotExist()
    }

    @Test
    @Config(sdk = [32])
    fun androidBeforeRuntimeNotificationPermissionsShowsNoQuestion() {
        attach()
        show()
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        assertFalse(asked())
    }

    private companion object {
        const val WAIT_MS = 10_000L
        const val TITLE = "Get told when an agent needs you"
    }
}
