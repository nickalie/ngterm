package com.nickalie.ngterm.android.ui.onboarding

import androidx.lifecycle.viewModelScope
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.FakeKeyMaker
import com.nickalie.ngterm.android.PLAIN_FAKE_KEY
import com.nickalie.ngterm.android.SoftwareWrapper
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.KeyMaker
import com.nickalie.ngterm.android.platform.KeyWrapper
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.core.ffi.KeyException
import com.nickalie.ngterm.core.ffi.KeyPair
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.util.concurrent.CountDownLatch

/** Real key storage and model, with failures only at the native key and Android wrapping boundaries. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class OnboardingFailureTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val models = mutableListOf<OnboardingViewModel>()
    private val maker = FailingKeyMaker()
    private val wrapper = FailingWrapper()
    private lateinit var keys: AppKeys

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        keys = AppKeys(testStore(dir.root, "keys"), KeystoreKeyVault(File(dir.root, "vault"), wrapper), maker, "test")
    }

    @After
    fun tearDown() {
        maker.gate?.countDown()
        models.forEach { it.viewModelScope.cancel() }
        Dispatchers.resetMain()
    }

    private fun model() = OnboardingViewModel(keys).also { models += it }

    private fun OnboardingViewModel.awaitStep(predicate: (OnboardingStep) -> Boolean): OnboardingStep =
        runBlocking { withTimeout(10_000) { step.first(predicate) } }

    @Test
    fun creationRefusedByTheCoreShowsItsReasonAndRetryCanSucceed() {
        val model = model()
        model.awaitStep { it is OnboardingStep.Welcome }
        maker.failure = KeyException.Invalid("entropy unavailable")
        model.create()
        model.awaitStep { it == OnboardingStep.Welcome(failure = "entropy unavailable") }
        maker.failure = null
        model.create()
        model.awaitStep { it is OnboardingStep.KeyReady }
        assertEquals(1, runBlocking { keys.keys.first() }.size)
    }

    @Test
    fun storageFailuresDuringCreationReleaseBusyAndDescribeBlankAndAbsentMessages() {
        val model = model()
        model.awaitStep { it is OnboardingStep.Welcome }
        listOf(
            IOException("disk full") to "disk full",
            GeneralSecurityException("  ") to "GeneralSecurityException",
            IOException() to "IOException",
        ).forEach { (error, words) ->
            wrapper.failure = error
            model.create()
            model.awaitStep { it == OnboardingStep.Welcome(failure = words) }
            assertTrue(runBlocking { keys.keys.first() }.isEmpty())
        }
        wrapper.failure = null
        model.create()
        model.awaitStep { it is OnboardingStep.KeyReady }
    }

    @Test
    fun importStorageFailuresKeepTheUserInImportAndWipeTheInputBeforeRetry() {
        val model = model()
        model.awaitStep { it is OnboardingStep.Welcome }
        model.startImport()
        listOf(
            IOException("no space") to "no space",
            GeneralSecurityException("keystore locked") to "keystore locked",
        ).forEach { (error, words) ->
            wrapper.failure = error
            val input = PLAIN_FAKE_KEY.toByteArray()
            model.import(input, null)
            model.awaitStep { it == OnboardingStep.Import(problem = ImportProblem.NotStored(words)) }
            assertTrue(input.all { it == 0.toByte() })
            assertTrue(runBlocking { keys.keys.first() }.isEmpty())
        }
        wrapper.failure = null
        model.import(PLAIN_FAKE_KEY.toByteArray(), null)
        model.awaitStep { it is OnboardingStep.KeyReady }
    }

    @Test
    fun noKeyOrWhitespaceIsRefusedWithoutReachingTheCoreAndUnreadableCanBeRetried() {
        val model = model()
        model.awaitStep { it is OnboardingStep.Welcome }
        model.back()
        assertEquals(OnboardingStep.Welcome(), model.step.value)
        model.startImport()
        model.unreadable()
        assertEquals(OnboardingStep.Import(problem = ImportProblem.Unreadable), model.step.value)
        listOf(byteArrayOf(), " \t\r\n".toByteArray()).forEach { input ->
            model.import(input, null)
            assertEquals(OnboardingStep.Import(problem = ImportProblem.Empty), model.step.value)
            assertTrue(input.all { it == 0.toByte() })
        }
        assertEquals(null, maker.delegate.lastImported)
        model.back()
        assertEquals(OnboardingStep.Welcome(), model.step.value)
    }

    @Test
    fun createIsIgnoredOutsideWelcomeAndWhileGenerationIsAlreadyRunning() {
        // A queued main dispatcher keeps initialization pending without relying on a race with DataStore.
        val queued = StandardTestDispatcher()
        Dispatchers.setMain(queued)
        val loading = model()
        loading.create()
        assertEquals(OnboardingStep.Loading, loading.step.value)
        assertEquals(0, maker.calls)
        loading.viewModelScope.cancel()
        Dispatchers.setMain(UnconfinedTestDispatcher())
        val model = model()
        model.awaitStep { it is OnboardingStep.Welcome }
        maker.gate = CountDownLatch(1)
        try {
            model.create()
            assertEquals(OnboardingStep.Welcome(busy = true), model.step.value)
            model.create()
        } finally {
            maker.gate?.countDown()
        }
        model.awaitStep { it is OnboardingStep.KeyReady }
        assertEquals(1, maker.calls)
        model.create()
        assertTrue(model.step.value is OnboardingStep.KeyReady)
        assertEquals(1, maker.calls)
    }

    private class FailingKeyMaker : KeyMaker {
        val delegate = FakeKeyMaker()

        @Volatile var failure: Exception? = null

        @Volatile var gate: CountDownLatch? = null
        var calls = 0
            private set

        override fun generate(comment: String): KeyPair {
            calls++
            gate?.await()
            failure?.let { throw it }
            return delegate.generate(comment)
        }

        override fun import(
            privateKey: ByteArray,
            passphrase: String?,
        ): KeyPair = delegate.import(privateKey, passphrase)
    }

    private class FailingWrapper : KeyWrapper {
        private val delegate = SoftwareWrapper()

        @Volatile var failure: Exception? = null

        override fun wrap(plain: ByteArray): ByteArray {
            failure?.let { throw it }
            return delegate.wrap(plain)
        }

        override fun unwrap(sealed: ByteArray): ByteArray = delegate.unwrap(sealed)
    }
}
