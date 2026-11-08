package com.nickalie.ngterm.android.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.lifecycle.viewModelScope
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.FAKE_PUBLIC_KEY
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import java.io.File

/** A restored key page must reconcile its saved id with the newly read stores after process recreation. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp-mdpi")
class SettingsRestorationTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private val graphs = mutableListOf<TestGraph>()
    private val models = mutableListOf<SettingsViewModel>()
    private var graph by mutableStateOf<TestGraph?>(null)
    private var model by mutableStateOf<SettingsViewModel?>(null)

    @After fun tearDown() {
        models.forEach { it.viewModelScope.cancel() }
        graphs.forEach { it.close() }
    }

    private fun graph(name: String): TestGraph =
        TestGraph(File(dir.root, name).apply { mkdirs() }).also { graphs += it }

    private fun openKey(): StateRestorationTester {
        val initial = graph("initial")
        initial.seedKey()
        graph = initial
        model = SettingsViewModel(initial.graph).also { models += it }
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            NgtermTheme { SettingsRoute(requireNotNull(graph).graph, "restored", model = requireNotNull(model)) }
        }
        compose.awaitText("core restored")
        compose.onNodeWithText("Show public key").performScrollTo().performClick()
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertExists()
        return restoration
    }

    private fun restoreWith(
        restoration: StateRestorationTester,
        next: TestGraph,
    ): TestDispatcher {
        val dispatcher = StandardTestDispatcher()
        val fresh =
            try {
                Dispatchers.setMain(dispatcher)
                SettingsViewModel(next.graph).also { models += it }
            } finally {
                Dispatchers.resetMain()
            }
        compose.runOnIdle {
            graph = next
            model = fresh
        }
        restoration.emulateSavedInstanceStateRestore()
        return dispatcher
    }

    // runs [dispatcher]'s queued work until [text] shows
    private fun runUntilShown(
        dispatcher: TestDispatcher,
        text: String,
    ) {
        compose.waitUntil(10_000) {
            dispatcher.scheduler.runCurrent()
            compose
                .onAllNodes(hasText(text))
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
    }

    private fun assertNoRouteBack() {
        compose.runOnIdle { assertFalse(compose.activity.onBackPressedDispatcher.hasEnabledCallbacks()) }
    }

    // Bringing the old key back must not reopen it: Ready without it must have cleared the saved page id.
    private fun assertMissingKeyPageWasCleared() {
        assertNoRouteBack()
        compose.runOnIdle {
            graph = graphs.first()
            model = models.first()
        }
        compose.awaitText("core restored")
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertDoesNotExist()
        assertNoRouteBack()
    }

    @Test
    fun aRestoredKeyPageLetsBackThroughWhileLoadingThenShowsItsExistingKey() {
        val restoration = openKey()
        val dispatcher = restoreWith(restoration, requireNotNull(graph))
        compose.onNodeWithContentDescription("Reading the settings").assertExists()
        assertNoRouteBack()
        runUntilShown(dispatcher, FAKE_PUBLIC_KEY)
        compose.runOnIdle { assertTrue(compose.activity.onBackPressedDispatcher.hasEnabledCallbacks()) }
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        runUntilShown(dispatcher, "core restored")
        assertNoRouteBack()
    }

    @Test
    fun aKeyAbsentFromTheNewProcessReturnsToSettingsWhenTheStoresFinishReading() {
        val restoration = openKey()
        val dispatcher = restoreWith(restoration, graph("next"))
        // until the stores are read, the restored page shows Settings' progress, not a blank screen
        compose.onNodeWithContentDescription("Reading the settings").assertExists()
        runUntilShown(dispatcher, "core restored")
        compose.onNodeWithText("No machine uses it").assertDoesNotExist()
        assertMissingKeyPageWasCleared()
    }

    @Test
    fun aRestoredKeyPageWhoseStoresFailShowsWhyWithRetry() {
        val restoration = openKey()
        val broken = File(dir.root, "broken").apply { mkdirs() }
        val settings = File(broken, "settings.preferences_pb").apply { writeBytes(byteArrayOf(-1, -1, -1, -1)) }
        val dispatcher = restoreWith(restoration, graph("broken"))
        runUntilShown(dispatcher, "Can't read the settings")
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertDoesNotExist()
        assertNoRouteBack()
        settings.delete()
        compose.onNodeWithText("Retry").performClick()
        runUntilShown(dispatcher, "core restored")
        compose.onNodeWithText("Can't read the settings").assertDoesNotExist()
        assertMissingKeyPageWasCleared()
    }
}
