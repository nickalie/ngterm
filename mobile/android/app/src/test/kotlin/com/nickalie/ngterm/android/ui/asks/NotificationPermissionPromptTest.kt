package com.nickalie.ngterm.android.ui.asks

import android.Manifest
import android.app.NotificationManager
import android.content.pm.PackageManager
import androidx.activity.ComponentActivity
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.emptyPreferences
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.NotificationPrefs
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onCompletion
import kotlinx.coroutines.runBlocking
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
import java.io.IOException

// what the probe beside the prompt shows once the store has answered
private const val ANSWERED = "the store answered"

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

    private fun show(prefs: NotificationPrefs = test.graph.notificationPrefs) {
        val original = test.graph
        val graph =
            AppGraph(
                machines = original.machines,
                keys = original.keys,
                pins = original.pins,
                connections = original.connections,
                clipboardReads = original.clipboardReads,
                panes = original.panes,
                chats = original.chats,
                asks = original.asks,
                notificationPrefs = prefs,
                settings = original.settings,
                opens = original.opens,
                online = original.online,
            )
        compose.setContent {
            NgtermTheme {
                NotificationPermissionPrompt(graph)
                if (prefs === original.notificationPrefs) StoreAnswered(prefs)
            }
        }
    }

    // For a readable DataStore, both collectors share the completed initial read.
    private fun settled() {
        compose.awaitText(ANSWERED)
        compose.waitForIdle()
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
        settled()
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
        settled()
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
        attach()
        val store = FailingPermissionStore()
        show(NotificationPrefs(store))
        failRead(store)
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        compose.onNodeWithText("Turn on notifications").assertDoesNotExist()
    }

    @Test
    fun aPermissionReadFailureClosesAnAlreadyVisibleQuestion() {
        attach()
        val store = FailingPermissionStore(initiallyUnasked = true)
        show(NotificationPrefs(store))
        // Positive control: the prompt's collector is active and this session can show the question.
        compose.awaitText(TITLE)
        failRead(store)
        compose.onNodeWithText(TITLE).assertDoesNotExist()
    }

    private fun failRead(store: FailingPermissionStore) {
        compose.runOnIdle { store.fail.complete(Unit) }
        compose.waitUntil(WAIT_MS) { store.finished.isCompleted }
        // The fake throws in the prompt's collector, without an IO dispatcher.
        // Idle also drains its catch and recomposition.
        compose.waitForIdle()
    }

    @Test
    @Config(sdk = [32])
    fun androidBeforeRuntimeNotificationPermissionsShowsNoQuestion() {
        // notifications turned off: the compat check reads that as no permission, so only the SDK check keeps it away
        shadowOf(compose.activity.getSystemService(NotificationManager::class.java)).setNotificationsEnabled(false)
        attach()
        show()
        settled()
        compose.onNodeWithText(TITLE).assertDoesNotExist()
        assertFalse(asked())
    }

    private companion object {
        const val WAIT_MS = 10_000L
        const val TITLE = "Get told when an agent needs you"
    }
}

/** [ANSWERED] once this collector reads [prefs]; only a readable DataStore shares the prompt's initial read. */
@Composable
private fun StoreAnswered(prefs: NotificationPrefs) {
    val read = remember(prefs) { prefs.permissionAsked.map { true }.catch { emit(true) } }
    val answered by read.collectAsState(initial = false)
    if (answered) Text(ANSWERED)
}

/** Only the prompt collects this store, so completion belongs to its read rather than a separate readiness probe. */
private class FailingPermissionStore(
    private val initiallyUnasked: Boolean = false,
) : DataStore<Preferences> {
    val fail = CompletableDeferred<Unit>()
    val finished = CompletableDeferred<Unit>()
    override val data =
        flow {
            if (initiallyUnasked) emit(emptyPreferences())
            fail.await()
            throw IOException("Unreadable notification preferences")
        }.onCompletion { finished.complete(Unit) }

    override suspend fun updateData(transform: suspend (Preferences) -> Preferences): Preferences =
        error("The permission test must not write preferences")
}
