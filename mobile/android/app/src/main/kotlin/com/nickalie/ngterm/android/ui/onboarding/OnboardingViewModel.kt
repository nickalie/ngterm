package com.nickalie.ngterm.android.ui.onboarding

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.core.ffi.KeyException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.IOException
import java.security.GeneralSecurityException

/** Why an import failed, as the import screen says it. */
sealed interface ImportProblem {
    /** The key is encrypted: it needs its passphrase. */
    data object NeedsPassphrase : ImportProblem

    /** The passphrase given doesn't open the key. */
    data object WrongPassphrase : ImportProblem

    /** The text is not a private key the app can use, for [reason] (the core's words). */
    data class NotAKey(
        val reason: String,
    ) : ImportProblem

    /** The key was read but could not be stored, for [reason]. */
    data class NotStored(
        val reason: String,
    ) : ImportProblem

    /** Nothing was pasted or chosen. */
    data object Empty : ImportProblem

    /** The chosen file could not be read. */
    data object Unreadable : ImportProblem
}

/** Where the first run is. */
sealed interface OnboardingStep {
    /** The app is finding out whether it has a key. */
    data object Loading : OnboardingStep

    /** No key yet: make one or bring one. [failure] says why the last attempt to make one failed. */
    data class Welcome(
        val busy: Boolean = false,
        val failure: String? = null,
    ) : OnboardingStep

    /** The user is importing a key. */
    data class Import(
        val busy: Boolean = false,
        val problem: ImportProblem? = null,
    ) : OnboardingStep

    /** A key is ready: its public half is shown, to put on the machines. */
    data class KeyReady(
        val key: AppKey,
    ) : OnboardingStep

    /** The app has a key: the first run is over. */
    data object Done : OnboardingStep
}

/** The first run: the app needs a key before it can reach any machine. */
class OnboardingViewModel(
    private val keys: AppKeys,
) : ViewModel() {
    private val current = MutableStateFlow<OnboardingStep>(OnboardingStep.Loading)

    val step: StateFlow<OnboardingStep> = current.asStateFlow()

    init {
        viewModelScope.launch {
            val existing = runCatching { keys.keys.first() }.getOrDefault(emptyList())
            current.value = if (existing.isEmpty()) OnboardingStep.Welcome() else OnboardingStep.Done
        }
    }

    /** Makes a new key on the device. */
    fun create() {
        val now = current.value
        if (now !is OnboardingStep.Welcome || now.busy) return
        current.value = OnboardingStep.Welcome(busy = true)
        viewModelScope.launch {
            current.value =
                try {
                    OnboardingStep.KeyReady(keys.generate())
                } catch (error: KeyException) {
                    OnboardingStep.Welcome(failure = error.describe())
                } catch (error: IOException) {
                    OnboardingStep.Welcome(failure = error.describe())
                } catch (error: GeneralSecurityException) {
                    OnboardingStep.Welcome(failure = error.describe())
                }
        }
    }

    /** Opens the import screen. */
    fun startImport() {
        current.value = OnboardingStep.Import()
    }

    /** Back from the import screen to the welcome. */
    fun back() {
        if (current.value is OnboardingStep.Import) current.value = OnboardingStep.Welcome()
    }

    /** The chosen file could not be read. */
    fun unreadable() {
        current.value = OnboardingStep.Import(problem = ImportProblem.Unreadable)
    }

    /** Imports the key in [privateKey] (wiped once read), opened with [passphrase]. */
    fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ) {
        if (privateKey.all { it.toInt().toChar().isWhitespace() }) {
            privateKey.fill(0)
            current.value = OnboardingStep.Import(problem = ImportProblem.Empty)
            return
        }
        current.value = OnboardingStep.Import(busy = true)
        viewModelScope.launch {
            current.value =
                try {
                    OnboardingStep.KeyReady(keys.import(privateKey, passphrase))
                } catch (error: KeyException) {
                    OnboardingStep.Import(problem = error.toProblem())
                } catch (error: IOException) {
                    OnboardingStep.Import(problem = ImportProblem.NotStored(error.describe()))
                } catch (error: GeneralSecurityException) {
                    OnboardingStep.Import(problem = ImportProblem.NotStored(error.describe()))
                }
        }
    }

    /** The public key has been seen: on to the machines. */
    fun finish() {
        current.value = OnboardingStep.Done
    }
}

private fun KeyException.toProblem(): ImportProblem =
    when (this) {
        is KeyException.PassphraseRequired -> ImportProblem.NeedsPassphrase
        is KeyException.WrongPassphrase -> ImportProblem.WrongPassphrase
        is KeyException.Invalid -> ImportProblem.NotAKey(reason)
    }

// UniFFI's messages are its fields (`reason=...`): the reason is the words
private fun Exception.describe(): String =
    when (this) {
        is KeyException.Invalid -> reason
        else -> message?.takeIf { it.isNotBlank() } ?: javaClass.simpleName
    }
