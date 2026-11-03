package com.nickalie.ngterm.android.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.android.ui.onboarding.ImportProblem
import com.nickalie.ngterm.android.ui.onboarding.describe
import com.nickalie.ngterm.android.ui.onboarding.toProblem
import com.nickalie.ngterm.core.ffi.KeyException
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import java.io.IOException
import java.security.GeneralSecurityException

/** One of the app's keys as Settings lists it: the machines that sign in with it, and whether that is every one. */
data class KeyItem(
    val key: AppKey,
    /** The names of the machines that use it, in the machines' order. */
    val usedBy: List<String>,
    /** Whether every machine uses it (vacuously so with no machine). */
    val everyMachine: Boolean,
)

/** A machine whose programs read the clipboard without asking. */
data class TrustedMachine(
    val id: String,
    val title: String,
    val address: String,
)

/** What Settings shows. */
sealed interface SettingsUiState {
    /** The stores are being read. */
    data object Loading : SettingsUiState

    /** A store could not be read, for [reason]. */
    data class Failed(
        val reason: String,
    ) : SettingsUiState

    /** The settings, the notification kinds that are on, the keys and the machines trusted with the clipboard. */
    data class Ready(
        val settings: AppSettings,
        val alerts: Set<AlertKind>,
        val keys: List<KeyItem>,
        val clipboard: List<TrustedMachine>,
    ) : SettingsUiState
}

/** What Settings' controls do, as [SettingsViewModel] and the route do it. */
data class SettingsActions(
    val onChange: ((AppSettings) -> AppSettings) -> Unit,
    val onAlert: (AlertKind, Boolean) -> Unit,
    val onRevokeClipboard: (String) -> Unit,
    val onShowKey: (String) -> Unit,
    val onRotate: () -> Unit,
    val onImport: () -> Unit,
    val onTurnOnNotifications: () -> Unit,
    val onRetry: () -> Unit,
)

/** Where making or bringing a key stands. */
data class KeyWork(
    /** A new key is being made (Rotate key). */
    val rotating: Boolean = false,
    /** Why the last rotation failed. */
    val rotateFailure: String? = null,
    /** An import is under way. */
    val importing: Boolean = false,
    /** What was wrong with the last import. */
    val importProblem: ImportProblem? = null,
)

/**
 * Settings over [graph]'s stores: the app's settings, the notification switches, the machines trusted with the
 * clipboard, and the keys. A key made ([rotate]) or brought ([import]) lands in [newKey], for its public key to be
 * shown; [useEverywhere] then points every machine at it.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SettingsViewModel(
    private val graph: AppGraph,
) : ViewModel() {
    private val reads = MutableStateFlow(0)
    private val work = MutableStateFlow(KeyWork())
    private val fresh = MutableStateFlow<String?>(null)

    /** What Settings shows. */
    val state: StateFlow<SettingsUiState> =
        reads
            .flatMapLatest {
                combine(
                    graph.settings.settings,
                    graph.notificationPrefs.enabled,
                    graph.keys.keys,
                    graph.machines.machines,
                    graph.clipboardReads.alwaysAllowed,
                ) { settings, alerts, keys, machines, trusted ->
                    SettingsUiState.Ready(
                        settings = settings,
                        alerts = alerts,
                        keys = keys.map { key -> key.item(machines) },
                        clipboard =
                            machines
                                .filter { it.id in trusted }
                                .map { TrustedMachine(it.id, it.title, it.address) },
                    ) as SettingsUiState
                }.catch { emit(SettingsUiState.Failed(it.message ?: it.javaClass.simpleName)) }
            }.stateIn(viewModelScope, SharingStarted.Eagerly, SettingsUiState.Loading)

    /** Where making or bringing a key stands. */
    val keyWork: StateFlow<KeyWork> = work.asStateFlow()

    /** The id of a key just made or brought, until [newKeyShown]. */
    val newKey: StateFlow<String?> = fresh.asStateFlow()

    /** Reads the stores again after a failure. */
    fun retry() {
        reads.value += 1
    }

    /** Changes the app's settings by [change]. */
    fun change(change: (AppSettings) -> AppSettings) {
        viewModelScope.launch { graph.settings.update(change) }
    }

    /** Turns the notifications of [kind] on or off. */
    fun setAlert(
        kind: AlertKind,
        on: Boolean,
    ) {
        viewModelScope.launch { graph.notificationPrefs.set(kind, on) }
    }

    /** The machine [id]'s programs ask again before they read the clipboard. */
    fun revokeClipboard(id: String) {
        viewModelScope.launch { graph.clipboardReads.revoke(id) }
    }

    /** Makes a new key; the machines keep theirs until [useEverywhere]. */
    fun rotate() {
        if (work.value.rotating) return
        work.value = work.value.copy(rotating = true, rotateFailure = null)
        viewModelScope.launch {
            val failure =
                try {
                    fresh.value = graph.keys.generate().id
                    null
                } catch (error: KeyException) {
                    error.describe()
                } catch (error: IOException) {
                    error.describe()
                } catch (error: GeneralSecurityException) {
                    error.describe()
                }
            work.value = work.value.copy(rotating = false, rotateFailure = failure)
        }
    }

    /** Opens the import afresh: no problem from an earlier attempt. */
    fun startImport() {
        work.value = work.value.copy(importing = false, importProblem = null)
    }

    /** The chosen file could not be read. */
    fun unreadable() {
        work.value = work.value.copy(importProblem = ImportProblem.Unreadable)
    }

    /** Imports the key in [privateKey] (wiped once read), opened with [passphrase]. */
    fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ) {
        if (work.value.importing) {
            privateKey.fill(0)
            return
        }
        if (privateKey.all { it.toInt().toChar().isWhitespace() }) {
            privateKey.fill(0)
            work.value = work.value.copy(importProblem = ImportProblem.Empty)
            return
        }
        work.value = work.value.copy(importing = true, importProblem = null)
        viewModelScope.launch {
            val problem =
                try {
                    fresh.value = graph.keys.import(privateKey, passphrase).id
                    null
                } catch (error: KeyException) {
                    error.toProblem()
                } catch (error: IOException) {
                    ImportProblem.NotStored(error.describe())
                } catch (error: GeneralSecurityException) {
                    ImportProblem.NotStored(error.describe())
                }
            work.value = work.value.copy(importing = false, importProblem = problem)
        }
    }

    /** The new key's public half is on screen. */
    fun newKeyShown() {
        fresh.value = null
    }

    /** Points every machine at the key [id], from their next connection. */
    fun useEverywhere(id: String) {
        viewModelScope.launch { graph.machines.useKey(id) }
    }
}

private fun AppKey.item(machines: List<StoredMachine>): KeyItem {
    val users = machines.filter { it.keyId == id }
    return KeyItem(this, users.map { it.title }, everyMachine = users.size == machines.size)
}
