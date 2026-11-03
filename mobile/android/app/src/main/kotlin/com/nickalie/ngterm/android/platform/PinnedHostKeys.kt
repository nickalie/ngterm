package com.nickalie.ngterm.android.platform

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.nickalie.ngterm.core.ffi.HostKeyStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.runBlocking

/**
 * The host keys pinned on first use, one per machine id, in [store] (Preferences DataStore): the core's
 * [HostKeyStore], the one place it learns a machine's pin from. The core calls [pinned] and [pin] on its own thread
 * and waits for them, so they block; the app's own reads and the removal are suspending.
 */
class PinnedHostKeys(
    private val store: DataStore<Preferences>,
) : HostKeyStore {
    /** Every pin, by machine id, on every change. */
    val all: Flow<Map<String, String>> =
        store.data.map { prefs ->
            prefs
                .asMap()
                .entries
                .mapNotNull { (key, value) ->
                    key.name
                        .removePrefix(PREFIX)
                        .takeIf { key.name.startsWith(PREFIX) }
                        ?.let { it to value as String }
                }.toMap()
        }

    override fun pinned(machineId: String): String? = runBlocking { get(machineId) }

    override fun pin(
        machineId: String,
        fingerprint: String,
    ) = runBlocking { set(machineId, fingerprint) }

    /** The pin of [machineId], if any. */
    suspend fun get(machineId: String): String? = store.data.first()[key(machineId)]

    /** Pins [fingerprint] for [machineId]. */
    suspend fun set(
        machineId: String,
        fingerprint: String,
    ) {
        store.edit { it[key(machineId)] = fingerprint }
    }

    /**
     * Forgets the pin of [machineId] (Remove pinned key, or the machine removed): its next connect asks about whatever
     * key it offers then.
     */
    suspend fun remove(machineId: String) {
        store.edit { it.remove(key(machineId)) }
    }

    private companion object {
        const val PREFIX = "pin."

        fun key(machineId: String) = stringPreferencesKey(PREFIX + machineId)
    }
}
