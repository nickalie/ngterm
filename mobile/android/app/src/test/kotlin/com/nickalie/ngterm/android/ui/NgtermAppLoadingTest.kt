package com.nickalie.ngterm.android.ui

import androidx.activity.compose.LocalActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.lifecycle.viewModelScope
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.ui.onboarding.OnboardingViewModel
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

/** The shell waits for the real onboarding model to read the key before drawing its destinations. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class NgtermAppLoadingTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createComposeRule()
    private lateinit var test: TestGraph
    private lateinit var onboarding: OnboardingViewModel

    @Before fun setUp() {
        test = TestGraph(dir.root)
        test.seedKey()
    }

    @After fun tearDown() {
        if (::onboarding.isInitialized) onboarding.viewModelScope.cancel()
        test.close()
    }

    @Test
    fun navigationWaitsForTheKeyStoreAndAppearsWhenItsReadCompletes() {
        val dispatcher = StandardTestDispatcher()
        Dispatchers.setMain(dispatcher)
        try {
            onboarding = OnboardingViewModel(test.graph.keys)
        } finally {
            Dispatchers.resetMain()
        }
        compose.setContent {
            NgtermTheme {
                CompositionLocalProvider(LocalActivity provides null) {
                    NgtermApp(test.graph, "loading-test", layout = NavLayout.Bar, onboarding = onboarding)
                }
            }
        }
        compose.onNodeWithText("Machines").assertDoesNotExist()
        compose.onNodeWithText("Set up a key").assertDoesNotExist()
        compose.waitUntil(10_000) {
            dispatcher.scheduler.runCurrent()
            compose
                .onAllNodes(
                    androidx.compose.ui.test
                        .hasText("No machines yet"),
                ).fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("No machines yet").assertExists()
    }
}
