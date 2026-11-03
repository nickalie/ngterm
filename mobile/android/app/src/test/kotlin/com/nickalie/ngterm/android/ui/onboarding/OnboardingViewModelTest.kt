package com.nickalie.ngterm.android.ui.onboarding

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ENCRYPTED_FAKE_KEY
import com.nickalie.ngterm.android.FAKE_FINGERPRINT
import com.nickalie.ngterm.android.FAKE_PASSPHRASE
import com.nickalie.ngterm.android.TestGraph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class OnboardingViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() {
        test.close()
        Dispatchers.resetMain()
    }

    private fun OnboardingViewModel.awaitStep(predicate: (OnboardingStep) -> Boolean) =
        runBlocking { step.first(predicate) }

    @Test
    fun anAppWithAKeySkipsTheFirstRun() {
        test.seedKey()
        val model = OnboardingViewModel(test.graph.keys)
        model.awaitStep { it == OnboardingStep.Done }
    }

    @Test
    fun creatingAKeyShowsItsPublicHalfThenFinishes() {
        val model = OnboardingViewModel(test.graph.keys)
        model.awaitStep { it == OnboardingStep.Welcome() }
        model.create()
        val ready = model.awaitStep { it is OnboardingStep.KeyReady } as OnboardingStep.KeyReady
        assertEquals(FAKE_FINGERPRINT, ready.key.fingerprint)
        assertEquals(1, test.maker.generated)
        model.finish()
        assertEquals(OnboardingStep.Done, model.step.value)
        assertEquals(
            listOf(ready.key),
            runBlocking {
                test.graph.keys.keys
                    .first()
            },
        )
    }

    @Test
    fun anImportSaysWhatItNeedsUntilThePassphraseOpensTheKey() {
        val model = OnboardingViewModel(test.graph.keys)
        model.awaitStep { it is OnboardingStep.Welcome }
        model.startImport()
        model.import("   ".toByteArray(), null)
        assertEquals(OnboardingStep.Import(problem = ImportProblem.Empty), model.step.value)
        model.import("not a key".toByteArray(), null)
        model.awaitStep { it == OnboardingStep.Import(problem = ImportProblem.NotAKey("unknown key format")) }
        model.import(ENCRYPTED_FAKE_KEY.toByteArray(), "")
        model.awaitStep { it == OnboardingStep.Import(problem = ImportProblem.NeedsPassphrase) }
        model.import(ENCRYPTED_FAKE_KEY.toByteArray(), "wrong")
        model.awaitStep { it == OnboardingStep.Import(problem = ImportProblem.WrongPassphrase) }
        val bytes = ENCRYPTED_FAKE_KEY.toByteArray()
        model.import(bytes, FAKE_PASSPHRASE)
        val ready = model.awaitStep { it is OnboardingStep.KeyReady } as OnboardingStep.KeyReady
        assertTrue(ready.key.imported)
        assertTrue("the key's bytes are wiped once read", bytes.all { it == 0.toByte() })
    }

    @Test
    fun backFromTheImportReturnsToTheWelcome() {
        val model = OnboardingViewModel(test.graph.keys)
        model.awaitStep { it is OnboardingStep.Welcome }
        model.startImport()
        model.back()
        assertEquals(OnboardingStep.Welcome(), model.step.value)
    }
}
