package com.nickalie.ngterm.android.ui.onboarding

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.core.app.ActivityOptionsCompat
import androidx.lifecycle.viewModelScope
import androidx.test.core.app.ApplicationProvider
import com.nickalie.ngterm.android.FAKE_FINGERPRINT
import com.nickalie.ngterm.android.PLAIN_FAKE_KEY
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.CountDownLatch

/** First-run navigation uses the real model and the core's key boundary. */
@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "w411dp-h891dp")
class OnboardingFlowTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private val registry = KeyPickerRegistry()
    private val owner =
        object : ActivityResultRegistryOwner {
            override val activityResultRegistry = registry
        }
    private lateinit var graph: TestGraph
    private lateinit var model: OnboardingViewModel

    @Before
    fun setUp() {
        graph = TestGraph(dir.root)
        compose.runOnUiThread { model = OnboardingViewModel(graph.graph.keys) }
    }

    @After
    fun tearDown() {
        model.viewModelScope.cancel()
        graph.close()
    }

    private fun show() {
        compose.setContent {
            val step by model.step.collectAsState()
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                NgtermTheme { OnboardingFlow(step, OnboardingActions.of(model)) }
            }
        }
        compose.awaitText("Set up a key")
    }

    @Test
    fun creatingAKeyShowsItsPublicHalfAndDoneLeavesOnboarding() {
        show()
        compose.onNodeWithText("Create a key").performClick()
        compose.awaitText(FAKE_FINGERPRINT, substring = true)
        compose.onNodeWithText(FAKE_FINGERPRINT, substring = true).performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Done").performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals(OnboardingStep.Done, model.step.value)
        assertEquals(1, graph.maker.generated)
        compose.onNodeWithText("Set up a key").assertDoesNotExist()
        compose.onNodeWithText(FAKE_FINGERPRINT, substring = true).assertDoesNotExist()
    }

    @Test
    fun toolbarAndSystemBackReturnToWelcomeAndDiscardThePrivateDraft() {
        show()
        compose.onNodeWithText("Import a key").performClick()
        compose.onNodeWithText("Private key").performTextReplacement(PLAIN_FAKE_KEY)
        compose.onNodeWithContentDescription("Back").performClick()
        compose.onNodeWithText("Set up a key").assertIsDisplayed()
        compose.onNodeWithText("Import a key").performClick()
        compose.onNodeWithText(PLAIN_FAKE_KEY).assertDoesNotExist()
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.onNodeWithText("Set up a key").assertIsDisplayed()
        compose.runOnIdle { assertFalse(compose.activity.onBackPressedDispatcher.hasEnabledCallbacks()) }
        assertEquals(0, graph.maker.generated)
    }

    @Test
    fun aPastedKeyReachesTheCoreAndThePublicHalfCompletesTheFlow() {
        show()
        compose.onNodeWithText("Import a key").performClick()
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        compose.onNodeWithText("Paste a private key or choose its file.").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Private key").performTextReplacement(PLAIN_FAKE_KEY)
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        compose.awaitText(FAKE_FINGERPRINT, substring = true)
        assertEquals(PLAIN_FAKE_KEY, graph.maker.lastImported)
        compose.onNodeWithText("Done").performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals(OnboardingStep.Done, model.step.value)
    }

    @Test
    fun backDuringAnImportShowsProgressAndDisablesCreatingAnotherKey() {
        val gate = CountDownLatch(1)
        graph.maker.importGate = gate
        try {
            show()
            compose.onNodeWithText("Import a key").performClick()
            compose.onNodeWithText("Private key").performTextReplacement(PLAIN_FAKE_KEY)
            compose.onNodeWithText("Import key").performScrollTo().performClick()
            compose.onNodeWithContentDescription("Back").performClick()
            compose.onNodeWithText("Set up a key").assertIsDisplayed()
            compose.onNodeWithText("Create a key").assertIsNotEnabled().performClick()
            compose.onNodeWithText("Importing a key").assertIsDisplayed()
            assertEquals(0, graph.maker.generated)
            // Reopening still shows the same import, with no second submission.
            compose.onNodeWithText("Import a key").performClick()
            compose.runOnIdle { assertEquals(OnboardingStep.Import(busy = true), model.step.value) }
            gate.countDown()
            compose.awaitText(FAKE_FINGERPRINT, substring = true)
        } finally {
            gate.countDown()
        }
    }

    @Test
    fun anUnreadableDocumentShowsTheProblemAndBackAllowsAnotherImport() {
        show()
        compose.onNodeWithText("Import a key").performClick()
        val uri = Uri.parse("content://keys/unavailable")
        shadowOf(compose.activity.contentResolver).registerInputStream(
            uri,
            object : InputStream() {
                override fun read(): Int = throw IOException("provider offline")
            },
        )
        compose.onNodeWithText("Choose file").performScrollTo().performClick()
        compose.runOnIdle { registry.dispatchResult(registry.request, Activity.RESULT_OK, Intent().setData(uri)) }
        compose
            .onNodeWithText("That file couldn't be read. Choose another, or paste the key.")
            .performScrollTo()
            .assertIsDisplayed()
        assertEquals(OnboardingStep.Import(problem = ImportProblem.Unreadable), model.step.value)
        compose.onNodeWithContentDescription("Back").performClick()
        compose.onNodeWithText("Set up a key").assertIsDisplayed()
        compose.onNodeWithText("Import a key").performClick()
        compose.onNodeWithText("That file couldn't be read. Choose another, or paste the key.").assertDoesNotExist()
    }

    @Test
    fun loadingAndFinishedStatesOfferNoKeyActions() {
        val step = mutableStateOf<OnboardingStep>(OnboardingStep.Loading)
        compose.setContent { NgtermTheme { OnboardingFlow(step.value, OnboardingActions.of(model)) } }
        compose.onNodeWithText("Create a key").assertDoesNotExist()
        compose.onNodeWithText("Import key").assertDoesNotExist()
        compose.runOnIdle { step.value = OnboardingStep.Done }
        compose.onNodeWithText("Create a key").assertDoesNotExist()
        compose.onNodeWithText("Import key").assertDoesNotExist()
        compose.onNodeWithText("Done").assertDoesNotExist()
    }

    @Test
    fun aKeyStoreReadFailureSaysSoAndRetryReadsTheKeysAgain() {
        val step = mutableStateOf<OnboardingStep>(OnboardingStep.ReadFailed("disk unavailable"))
        var retried = 0
        val actions = OnboardingActions.of(model).copy(onRetry = { retried++ })
        compose.setContent { NgtermTheme { OnboardingFlow(step.value, actions) } }
        compose.onNodeWithText("Your keys couldn't be read").assertIsDisplayed()
        compose.onNodeWithText("disk unavailable", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Create a key").assertDoesNotExist()
        compose.onNodeWithText("Retry").performClick()
        compose.runOnIdle { assertEquals(1, retried) }
    }

    private class KeyPickerRegistry : ActivityResultRegistry() {
        var request = 0

        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            request = requestCode
            contract.createIntent(ApplicationProvider.getApplicationContext(), input)
        }
    }
}
