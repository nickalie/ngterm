package com.nickalie.ngterm.android.ui.onboarding

import androidx.lifecycle.viewModelScope
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ENCRYPTED_FAKE_KEY
import com.nickalie.ngterm.android.FAKE_FINGERPRINT
import com.nickalie.ngterm.android.FAKE_PASSPHRASE
import com.nickalie.ngterm.android.FakeKeyMaker
import com.nickalie.ngterm.android.PLAIN_FAKE_KEY
import com.nickalie.ngterm.android.SoftwareWrapper
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.testStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.job
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch

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

    @Test
    fun aReadFailureMustNotPretendThatTheAppHasNoKey() =
        runBlocking {
            val unreadable = dir.newFolder("unreadable.preferences_pb")
            val keys =
                AppKeys(
                    testStore(dir.root, "unreadable"),
                    KeystoreKeyVault(File(dir.root, "vault"), SoftwareWrapper()),
                    FakeKeyMaker(),
                    "test",
                )
            val model = OnboardingViewModel(keys)
            try {
                val settled =
                    withTimeout(10_000) {
                        model.step.first { it != OnboardingStep.Loading }
                    }
                assertNotEquals(
                    "A store read failure needs a visible error, not a clean first run",
                    OnboardingStep.Welcome(),
                    settled,
                )
                assertTrue(settled is OnboardingStep.ReadFailed)
                // nothing but Retry leaves it: no key is made or brought over an unread store
                model.create()
                model.startImport()
                assertEquals(settled, model.step.value)
                unreadable.delete()
                model.retry()
                assertEquals(
                    OnboardingStep.Welcome(),
                    withTimeout(10_000) { model.step.first { it != OnboardingStep.Loading } },
                )
            } finally {
                model.viewModelScope.cancel()
            }
        }

    @Test
    fun backAndReopenDuringImportMustNotAllowTwoKeysToBeStored() =
        runBlocking {
            val model = OnboardingViewModel(test.graph.keys)
            val gate = CountDownLatch(1)
            try {
                withTimeout(10_000) {
                    model.step.first { it is OnboardingStep.Welcome }
                }
                test.maker.importGate = gate
                model.startImport()
                model.import(PLAIN_FAKE_KEY.toByteArray(), null)
                model.back()
                // the import goes on: no key is made beside it, and the import screen shows it still busy
                model.create()
                assertEquals(OnboardingStep.Welcome(), model.step.value)
                model.startImport()
                assertEquals(OnboardingStep.Import(busy = true), model.step.value)
                val second = PLAIN_FAKE_KEY.toByteArray()
                model.import(second, null)
                assertTrue("a refused key's bytes are wiped", second.all { it == 0.toByte() })
                val work =
                    model.viewModelScope.coroutineContext.job.children
                        .toList()
                gate.countDown()
                withTimeout(10_000) { work.joinAll() }
                assertEquals(
                    1,
                    test.graph.keys.keys
                        .first()
                        .size,
                )
                assertEquals(0, test.maker.generated)
                assertTrue(model.step.value is OnboardingStep.KeyReady)
            } finally {
                gate.countDown()
                model.viewModelScope.cancel()
            }
        }

    @Test
    fun aFailedImportFinishingAfterBackLeavesTheWelcomeAndFreesTheImport() =
        runBlocking {
            val model = OnboardingViewModel(test.graph.keys)
            val gate = CountDownLatch(1)
            try {
                withTimeout(10_000) { model.step.first { it is OnboardingStep.Welcome } }
                test.maker.importGate = gate
                model.startImport()
                model.import("not a key".toByteArray(), null)
                model.back()
                val work =
                    model.viewModelScope.coroutineContext.job.children
                        .toList()
                gate.countDown()
                withTimeout(10_000) { work.joinAll() }
                assertEquals(OnboardingStep.Welcome(), model.step.value)
                model.startImport()
                assertEquals(OnboardingStep.Import(), model.step.value)
            } finally {
                gate.countDown()
                model.viewModelScope.cancel()
            }
        }
}
